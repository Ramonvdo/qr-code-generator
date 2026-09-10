//! Stamping codes onto a PDF template the user supplies.
//!
//! The obvious implementation, generating one stamped copy per ticket and then
//! merging N documents, means renumbering every object and rebuilding the page
//! tree by hand. This does something cheaper and far less fragile: it loads the
//! template once and adds one small page object per ticket, all of which point
//! at the template's original, untouched content stream.
//!
//! PDF allows a page's `/Contents` to be an array of streams, concatenated in
//! order. So each ticket page is `[template content, our stamp]`. The template
//! artwork exists once in the file no matter how many tickets are issued, and
//! nothing in it is rewritten, which means nothing in it can be corrupted.

use crate::fonts::{self, Align, Font};
use crate::render::{Qr, Rgb};
use crate::vars::{self, Vars};
use lopdf::{dictionary, Dictionary, Document, Object, ObjectId, Stream};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt::Write as _;

/// Everything that can be drawn on a ticket, in the order it is drawn.
///
/// One ordered list rather than a fixed set of slots, so a design can carry a
/// name, a seat, a tier badge and a tear-off rule without any of those being
/// concepts this code knows about. The order is the stacking order.
///
/// Positions are fractions of the page with the origin at the top left,
/// matching the editor. Storing them normalised is what lets a layout made
/// against A4 land correctly on US Letter.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Element {
    Qr(QrElement),
    Text(TextElement),
    Image(ImageElement),
    Shape(ShapeElement),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QrElement {
    pub id: String,
    pub x: f32,
    pub y: f32,
    /// Side as a fraction of the page width, quiet zone included.
    pub size: f32,
    #[serde(default)]
    pub rotation: f32,
    #[serde(default = "one")]
    pub opacity: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextElement {
    pub id: String,
    pub x: f32,
    pub y: f32,
    /// Rendered through `vars`, so it can be anything from a bare
    /// `{{first_name}}` to a whole sentence.
    pub template: String,
    pub points: f32,
    #[serde(default)]
    pub font: Font,
    pub colour: String,
    #[serde(default)]
    pub align: Align,
    #[serde(default)]
    pub rotation: f32,
    #[serde(default = "one")]
    pub opacity: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageElement {
    pub id: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    /// The variable holding a path to the image for this ticket. A per-tier
    /// sponsor logo is a column, not a setting.
    pub column: String,
    #[serde(default)]
    pub rotation: f32,
    #[serde(default = "one")]
    pub opacity: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShapeElement {
    pub id: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    /// Hex, or absent for no fill. A shape with neither fill nor stroke draws
    /// nothing, which is a harmless way to leave a placeholder in the list.
    #[serde(default)]
    pub fill: Option<String>,
    #[serde(default)]
    pub stroke: Option<String>,
    #[serde(default = "one")]
    pub stroke_width: f32,
    #[serde(default)]
    pub rotation: f32,
    #[serde(default = "one")]
    pub opacity: f32,
}

fn one() -> f32 {
    1.0
}

/// Round a value to the precision it will be printed at.
///
/// `cos(90°)` in f32 is about -4.4e-8 rather than zero, which prints as
/// `-0.00000` and makes a content stream look broken to anyone reading it.
/// Snapping to the printed precision also collapses negative zero, which is
/// valid PDF but equally alarming to find in a file.
fn snap(v: f32) -> f32 {
    let r = (v * 100_000.0).round() / 100_000.0;
    if r == 0.0 {
        0.0
    } else {
        r
    }
}

impl Element {
    /// Every variable this element mentions, for working out which CSV columns
    /// an event needs.
    pub fn referenced(&self) -> Vec<String> {
        match self {
            Element::Text(e) => vars::referenced(&e.template),
            // An image names its column directly rather than through braces.
            Element::Image(e) if !e.column.trim().is_empty() => vec![e.column.trim().to_string()],
            _ => Vec::new(),
        }
    }
}

/// A page's worth of drawing, plus the resources it needs.
///
/// Opacity in PDF is a graphics-state object rather than an operator, so the
/// distinct values used have to be collected while drawing and declared in the
/// page's resources afterwards. This carries both.
#[derive(Default)]
struct Canvas {
    ops: String,
    /// Opacity value to its `/GS` resource name.
    alphas: BTreeMap<String, String>,
    /// Whether each font was actually used, so unused ones stay out of the
    /// resource dictionary.
    fonts: Vec<Font>,
    /// Image column to the decoded bytes, keyed by resource name.
    images: Vec<(String, Vec<u8>)>,
}

impl Canvas {
    /// The `/GS` name for an opacity, registering it on first use.
    fn alpha(&mut self, opacity: f32) -> Option<String> {
        let a = opacity.clamp(0.0, 1.0);
        if a >= 1.0 {
            return None;
        }
        let key = format!("{a:.3}");
        let next = self.alphas.len();
        Some(
            self.alphas
                .entry(key)
                .or_insert_with(|| format!("GA{next}"))
                .clone(),
        )
    }

    fn use_font(&mut self, font: Font) {
        if !self.fonts.contains(&font) {
            self.fonts.push(font);
        }
    }

    /// Open a transformed, optionally faded drawing context at `(x, y)`.
    ///
    /// Everything after this draws relative to the element's own origin, which
    /// is what makes rotation a single matrix rather than trigonometry at
    /// every corner.
    fn begin(&mut self, x: f32, y: f32, rotation: f32, opacity: f32) {
        self.ops.push_str("q\n");
        if let Some(gs) = self.alpha(opacity) {
            let _ = writeln!(self.ops, "/{gs} gs");
        }
        let r = rotation.to_radians();
        let (sin, cos) = (snap(r.sin()), snap(r.cos()));
        let _ = writeln!(
            self.ops,
            "{cos:.5} {sin:.5} {:.5} {cos:.5} {x:.3} {y:.3} cm",
            snap(-sin)
        );
    }

    fn end(&mut self) {
        self.ops.push_str("Q\n");
    }
}

#[derive(Debug)]
pub enum PdfError {
    Open(String),
    NoPages,
    NoSuchPage(usize),
    Rotated(i64),
    NoMediaBox,
    Write(String),
}

impl std::fmt::Display for PdfError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            PdfError::Open(e) => write!(f, "Could not read that PDF: {e}"),
            PdfError::NoPages => write!(f, "That PDF has no pages."),
            PdfError::NoSuchPage(n) => write!(f, "That template has no page {n}."),
            PdfError::Rotated(r) => write!(
                f,
                "That page is rotated by {r} degrees, which is not supported yet. \
                 Save a flattened copy with the rotation applied and try again."
            ),
            PdfError::NoMediaBox => write!(f, "That PDF does not declare a page size."),
            PdfError::Write(e) => write!(f, "Could not write the PDF: {e}"),
        }
    }
}

impl std::error::Error for PdfError {}

/// One ticket: its code, and the values every template resolves against.
pub struct Stamp<'a> {
    pub qr: &'a Qr,
    pub vars: Vars,
}

/// A template and the page of it that carries the code.
///
/// Every page is kept, not just the first. A ticket design is very often two
/// sided, and dropping the back of it while silently producing a plausible
/// looking front is the kind of bug you only notice after printing.
pub struct Template {
    doc: Document,
    /// All template pages, in document order.
    page_ids: Vec<ObjectId>,
    /// Which of them the code goes on.
    stamp_index: usize,
    /// `[x0, y0, x1, y1]` of the stamped page, in points.
    media_box: [f32; 4],
}

impl Template {
    pub fn load(bytes: &[u8]) -> Result<Template, PdfError> {
        let doc = Document::load_mem(bytes).map_err(|e| PdfError::Open(e.to_string()))?;
        // `get_pages` is keyed by page number, so iterating it in key order is
        // what keeps a multi-page template in the order the designer made it.
        let page_ids: Vec<ObjectId> = doc.get_pages().into_values().collect();
        if page_ids.is_empty() {
            return Err(PdfError::NoPages);
        }
        let media_box = page_geometry(&doc, page_ids[0])?;
        Ok(Template {
            doc,
            page_ids,
            stamp_index: 0,
            media_box,
        })
    }

    pub fn page_count(&self) -> usize {
        self.page_ids.len()
    }

    /// Choose which page the code lands on, validating that page's geometry.
    pub fn stamp_on(mut self, index: usize) -> Result<Template, PdfError> {
        let id = *self
            .page_ids
            .get(index)
            .ok_or(PdfError::NoSuchPage(index + 1))?;
        self.media_box = page_geometry(&self.doc, id)?;
        self.stamp_index = index;
        Ok(self)
    }

    /// An empty page of the given size, for issuing without a design.
    ///
    /// Requiring a template would make the whole feature unusable for anyone
    /// who just wants numbered codes on paper, and building the blank page
    /// through the same path means it is stamped by exactly the same code.
    pub fn blank(width: f32, height: f32) -> Template {
        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), width.into(), height.into()],
            "Resources" => dictionary! {},
        });
        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => vec![page_id.into()],
                "Count" => 1,
            }),
        );
        let catalog = doc.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        doc.trailer.set("Root", catalog);

        Template {
            doc,
            page_ids: vec![page_id],
            stamp_index: 0,
            media_box: [0.0, 0.0, width, height],
        }
    }

    /// Page size in points, as the user sees it.
    pub fn page_size(&self) -> (f32, f32) {
        (
            (self.media_box[2] - self.media_box[0]).abs(),
            (self.media_box[3] - self.media_box[1]).abs(),
        )
    }

    /// Build one finished document from these stamps.
    ///
    /// Takes `&self` and works on a clone of the loaded template, so the same
    /// template produces a file per buyer without being re-parsed each time.
    /// Cloning a template is cheap; re-reading and re-parsing one several
    /// hundred times is not.
    pub fn stamp(
        &self,
        stamps: &[Stamp],
        layout: &[Element],
        quiet_zone: usize,
        dark: Rgb,
        light: Rgb,
    ) -> Result<Vec<u8>, PdfError> {
        let mut doc = self.doc.clone();

        // Snapshot every template page up front. Each ticket gets a copy of
        // all of them, so a two-sided design stays two sided.
        let templates: Vec<Dictionary> = self
            .page_ids
            .iter()
            .map(|id| {
                doc.get_object(*id)
                    .and_then(|o| o.as_dict())
                    .cloned()
                    .map_err(|e| PdfError::Open(e.to_string()))
            })
            .collect::<Result<_, _>>()?;

        let stamp_page_id = self.page_ids[self.stamp_index];
        let page = templates[self.stamp_index].clone();

        // The template's own drawing operations, referenced rather than
        // copied, so the artwork appears once in the file regardless of how
        // many tickets are issued.
        let base_contents: Vec<Object> = match page.get(b"Contents") {
            Ok(Object::Reference(id)) => vec![Object::Reference(*id)],
            Ok(Object::Array(items)) => items.clone(),
            _ => vec![],
        };
        let base_resources = inherited(&doc, stamp_page_id, b"Resources")
            .and_then(|o| match o {
                Object::Dictionary(d) => Some(d.clone()),
                Object::Reference(id) => doc
                    .get_object(*id)
                    .ok()
                    .and_then(|o| o.as_dict().ok())
                    .cloned(),
                _ => None,
            })
            .unwrap_or_default();

        // Every font the layout could use, declared once and shared by every
        // page. All are among the 14 built into readers, so nothing is
        // embedded and the file stays small however many tickets there are.
        let font_ids: BTreeMap<&str, ObjectId> = Font::all()
            .iter()
            .map(|f| {
                let id = doc.add_object(dictionary! {
                    "Type" => "Font",
                    "Subtype" => "Type1",
                    "BaseFont" => f.base_name(),
                    "Encoding" => "WinAnsiEncoding",
                });
                (f.resource(), id)
            })
            .collect();

        let (pw, ph) = self.page_size();
        let page_box = Page {
            x0: self.media_box[0],
            y0: self.media_box[1],
            width: pw,
            height: ph,
        };

        let mut page_ids = Vec::with_capacity(stamps.len() * templates.len());
        for stamp in stamps {
            // Pages before the stamped one, copied through untouched and
            // still pointing at the template's own content and resources.
            for template_page in &templates[..self.stamp_index] {
                page_ids.push(doc.add_object(Object::Dictionary(template_page.clone())));
            }

            let image_id = doc.add_object(qr_image(stamp.qr, quiet_zone, dark, light));

            let mut xobjects = Dictionary::new();
            xobjects.set("QRC", Object::Reference(image_id));
            let mut resources = base_resources.clone();
            // Merge rather than replace, or a template whose own images live
            // in /XObject would lose them.
            if let Ok(Object::Dictionary(existing)) = resources.get(b"XObject") {
                for (k, v) in existing.iter() {
                    if k != b"QRC" {
                        xobjects.set(k.to_vec(), v.clone());
                    }
                }
            }
            resources.set("XObject", Object::Dictionary(xobjects));

            let mut canvas = Canvas::default();
            draw(&mut canvas, page_box, layout, &|template| {
                stamp.vars.render(template)
            });

            let mut fonts = match resources.get(b"Font") {
                Ok(Object::Dictionary(d)) => d.clone(),
                _ => Dictionary::new(),
            };
            for font in &canvas.fonts {
                if let Some(id) = font_ids.get(font.resource()) {
                    fonts.set(font.resource(), Object::Reference(*id));
                }
            }
            resources.set("Font", Object::Dictionary(fonts));

            // Images named by a column differ per ticket, so they are added
            // here rather than shared like the fonts.
            for (name, bytes) in &canvas.images {
                if let Ok((mut image, mask)) = photo_stream(bytes) {
                    // The mask is added first so the image has an id to point
                    // its /SMask at.
                    if let Some(mask) = mask {
                        let mask_id = doc.add_object(mask);
                        image.dict.set("SMask", Object::Reference(mask_id));
                    }
                    let id = doc.add_object(image);
                    if let Ok(Object::Dictionary(x)) = resources.get_mut(b"XObject") {
                        x.set(name.as_bytes().to_vec(), Object::Reference(id));
                    }
                }
            }

            // Opacity is a graphics-state object, not an operator, so the
            // values the drawing used have to be declared back into the page.
            if !canvas.alphas.is_empty() {
                let mut states = match resources.get(b"ExtGState") {
                    Ok(Object::Dictionary(d)) => d.clone(),
                    _ => Dictionary::new(),
                };
                for (value, name) in &canvas.alphas {
                    let alpha: f32 = value.parse().unwrap_or(1.0);
                    let id = doc.add_object(dictionary! {
                        "Type" => "ExtGState",
                        "ca" => alpha,
                        "CA" => alpha,
                    });
                    states.set(name.as_bytes().to_vec(), Object::Reference(id));
                }
                resources.set("ExtGState", Object::Dictionary(states));
            }

            let overlay = doc.add_object(
                Stream::new(dictionary! {}, canvas.ops.into_bytes()).with_compression(true),
            );

            let mut contents = base_contents.clone();
            contents.push(Object::Reference(overlay));

            let mut new_page = page.clone();
            new_page.set("Contents", Object::Array(contents));
            new_page.set("Resources", Object::Dictionary(resources));
            page_ids.push(doc.add_object(Object::Dictionary(new_page)));

            // And the rest of the design after it, so page order survives.
            for template_page in &templates[self.stamp_index + 1..] {
                page_ids.push(doc.add_object(Object::Dictionary(template_page.clone())));
            }
        }

        rebuild_page_tree(&mut doc, &page_ids)?;

        let mut out = Vec::new();
        doc.save_to(&mut out)
            .map_err(|e| PdfError::Write(e.to_string()))?;
        Ok(out)
    }
}

/// Point the catalogue at exactly the generated pages.
///
/// The template's original page objects are left in the document but orphaned;
/// `prune_objects` collects them along with anything else now unreachable.
fn rebuild_page_tree(doc: &mut Document, page_ids: &[ObjectId]) -> Result<(), PdfError> {
    let pages_id = doc
        .catalog()
        .and_then(|c| c.get(b"Pages"))
        .and_then(|o| o.as_reference())
        .map_err(|e| PdfError::Open(e.to_string()))?;

    for id in page_ids {
        if let Ok(Object::Dictionary(d)) = doc.get_object_mut(*id) {
            d.set("Parent", Object::Reference(pages_id));
        }
    }

    if let Ok(Object::Dictionary(pages)) = doc.get_object_mut(pages_id) {
        pages.set(
            "Kids",
            Object::Array(page_ids.iter().map(|id| Object::Reference(*id)).collect()),
        );
        pages.set("Count", Object::Integer(page_ids.len() as i64));
    }

    doc.prune_objects();
    doc.renumber_objects();
    doc.compress();
    Ok(())
}

/// Validate one page and return its box.
///
/// `/Rotate` and `/MediaBox` are both inheritable, so a page that does not
/// carry them is not necessarily missing them.
fn page_geometry(doc: &Document, page_id: ObjectId) -> Result<[f32; 4], PdfError> {
    let rotate = inherited(doc, page_id, b"Rotate")
        .and_then(|o| o.as_i64().ok())
        .unwrap_or(0)
        .rem_euclid(360);
    if rotate != 0 {
        // Refused rather than guessed at. Placing the code somewhere plausible
        // but wrong on a thousand printed tickets is a far worse outcome than
        // saying so up front.
        return Err(PdfError::Rotated(rotate));
    }

    inherited(doc, page_id, b"MediaBox")
        .and_then(|o| o.as_array().ok())
        .and_then(|a| {
            let v: Vec<f32> = a.iter().filter_map(as_f32).collect();
            <[f32; 4]>::try_from(v).ok()
        })
        .ok_or(PdfError::NoMediaBox)
}

/// Walk up the `/Parent` chain looking for an inheritable page attribute.
fn inherited<'a>(doc: &'a Document, page_id: ObjectId, key: &[u8]) -> Option<&'a Object> {
    let mut current = page_id;
    // Bounded so a template with a cyclic parent chain cannot hang the app.
    for _ in 0..32 {
        let dict = doc.get_object(current).ok()?.as_dict().ok()?;
        if let Ok(value) = dict.get(key) {
            return Some(value);
        }
        current = dict.get(b"Parent").ok()?.as_reference().ok()?;
    }
    None
}

fn as_f32(o: &Object) -> Option<f32> {
    match o {
        Object::Integer(i) => Some(*i as f32),
        Object::Real(r) => Some(*r),
        _ => None,
    }
}

/// The QR as a one-bit indexed image, at exactly one pixel per module.
///
/// PDF scales it up at print time, so there is no reason to rasterise here.
/// A 33 module code is 33x33 bits, roughly 140 bytes before compression, which
/// is what keeps a thousand-ticket file small enough to email.
fn qr_image(qr: &Qr, quiet: usize, dark: Rgb, light: Rgb) -> Stream {
    // The quiet zone is part of the image rather than something the caller is
    // trusted to leave room for. A code stamped flush against ticket artwork
    // fails on a large share of scanners, and the failure only shows up once
    // the tickets are printed and someone is standing at a door with them.
    let size = qr.size() + quiet * 2;
    let row_bytes = size.div_ceil(8);
    // All ones: every pixel starts light, so the border is the quiet zone.
    let mut data = vec![0xffu8; row_bytes * size];
    for y in 0..qr.size() {
        for x in 0..qr.size() {
            // Index 0 is dark, 1 is light, so a cleared bit is a dark module.
            if qr.dark(x, y) {
                let (px, py) = (x + quiet, y + quiet);
                data[py * row_bytes + px / 8] &= !(0x80 >> (px % 8));
            }
        }
    }

    let palette = vec![dark.0, dark.1, dark.2, light.0, light.1, light.2];
    let mut stream = Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Image",
            "Width" => size as i64,
            "Height" => size as i64,
            "ColorSpace" => Object::Array(vec![
                "Indexed".into(),
                "DeviceRGB".into(),
                Object::Integer(1),
                Object::String(palette, lopdf::StringFormat::Hexadecimal),
            ]),
            "BitsPerComponent" => 1,
            // Smoothing a QR turns crisp module edges into grey mush that
            // scanners read less reliably at small print sizes.
            "Interpolate" => false,
        },
        data,
    );
    // `Document::compress` only touches page content streams, so image data
    // has to be offered explicitly. lopdf declines when deflate would not
    // actually shrink the stream, which is the usual outcome here: module
    // data is close to random, and a small code compresses to slightly more
    // than it started as. Left in because it does pay off on larger codes.
    let _ = stream.compress();
    stream
}

/// Turn image bytes into a PDF image, plus its transparency mask if it has one.
///
/// PDF has no alpha channel on an image. Transparency is a second, greyscale
/// image referenced as the `/SMask`. Without it a logo saved as a transparent
/// PNG arrives as a white rectangle, which is exactly the case people use most.
///
/// Returns `(image, mask)`. The caller adds the mask first so it has an object
/// id to point at.
fn photo_stream(bytes: &[u8]) -> Result<(Stream, Option<Stream>), PdfError> {
    let decoded = image::load_from_memory(bytes)
        .map_err(|e| PdfError::Open(format!("Could not read that image: {e}")))?;
    let rgba = decoded.to_rgba8();
    let (w, h) = rgba.dimensions();

    let mut colour = Vec::with_capacity((w * h * 3) as usize);
    let mut alpha = Vec::with_capacity((w * h) as usize);
    let mut transparent = false;
    for px in rgba.pixels() {
        colour.extend_from_slice(&px.0[..3]);
        alpha.push(px.0[3]);
        transparent |= px.0[3] != 255;
    }

    let mut image = Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Image",
            "Width" => w as i64,
            "Height" => h as i64,
            "ColorSpace" => "DeviceRGB",
            "BitsPerComponent" => 8,
        },
        colour,
    );
    let _ = image.compress();

    let mask = transparent.then(|| {
        let mut mask = Stream::new(
            dictionary! {
                "Type" => "XObject",
                "Subtype" => "Image",
                "Width" => w as i64,
                "Height" => h as i64,
                "ColorSpace" => "DeviceGray",
                "BitsPerComponent" => 8,
            },
            alpha,
        );
        let _ = mask.compress();
        mask
    });

    Ok((image, mask))
}

/// The page geometry every element is positioned against.
#[derive(Debug, Clone, Copy)]
struct Page {
    x0: f32,
    y0: f32,
    width: f32,
    height: f32,
}

impl Page {
    /// Normalised top-left coordinates to PDF user space, whose origin is at
    /// the bottom left. Getting this backwards is the classic PDF mistake and
    /// it is confined to these two lines.
    fn at(&self, x: f32, y: f32) -> (f32, f32) {
        (
            self.x0 + x * self.width,
            self.y0 + self.height - y * self.height,
        )
    }
}

fn ink(hex: &str) -> Rgb {
    Rgb::from_hex(hex).unwrap_or(Rgb::BLACK)
}

fn rgb_op(c: Rgb) -> String {
    format!(
        "{:.3} {:.3} {:.3}",
        c.0 as f32 / 255.0,
        c.1 as f32 / 255.0,
        c.2 as f32 / 255.0
    )
}

/// Draw one ticket's elements into a content stream.
///
/// `resolve` turns a template into text for this particular ticket, which is
/// how the same layout produces a different name on every page.
fn draw(canvas: &mut Canvas, page: Page, elements: &[Element], resolve: &dyn Fn(&str) -> String) {
    for element in elements {
        match element {
            // The code is placed by the caller, which already has the matrix
            // and the image resource; here it only reserves its slot in the
            // stacking order.
            Element::Qr(e) => {
                let (x, y) = page.at(e.x, e.y);
                let side = e.size * page.width;
                canvas.begin(x, y - side, e.rotation, e.opacity);
                let _ = writeln!(canvas.ops, "{side:.3} 0 0 {side:.3} 0 0 cm\n/QRC Do");
                canvas.end();
            }

            Element::Text(e) => {
                let text = resolve(&e.template);
                if text.trim().is_empty() {
                    // A variable that is empty for this buyer leaves nothing
                    // behind rather than an orphaned label.
                    continue;
                }
                canvas.use_font(e.font);
                let (x, y) = page.at(e.x, e.y);
                // Alignment is entirely a matter of where the cursor starts,
                // because PDF only ever draws rightwards from it.
                let shift = Align::offset(e.align, fonts::width(e.font, &text, e.points));
                canvas.begin(x, y, e.rotation, e.opacity);
                let _ = writeln!(
                    canvas.ops,
                    "{} rg\nBT\n/{} {:.2} Tf\n{:.3} 0 Td\n({}) Tj\nET",
                    rgb_op(ink(&e.colour)),
                    e.font.resource(),
                    e.points,
                    snap(-shift),
                    escape_pdf_text(&text)
                );
                canvas.end();
            }

            Element::Image(e) => {
                let path = resolve(&format!("{{{{{}}}}}", e.column));
                let path = path.trim();
                // Silently skipped rather than failed: a sponsor logo column
                // that is blank for most tiers is normal, and one bad path
                // should not lose the other four hundred tickets.
                if path.is_empty() {
                    continue;
                }
                let Ok(bytes) = std::fs::read(path) else {
                    continue;
                };
                let name = format!("IM{}", canvas.images.len());
                let (x, y) = page.at(e.x, e.y);
                let (w, h) = (e.width * page.width, e.height * page.height);
                canvas.begin(x, y - h, e.rotation, e.opacity);
                let _ = writeln!(canvas.ops, "{w:.3} 0 0 {h:.3} 0 0 cm\n/{name} Do");
                canvas.end();
                canvas.images.push((name, bytes));
            }

            Element::Shape(e) => {
                if e.fill.is_none() && e.stroke.is_none() {
                    continue;
                }
                let (x, y) = page.at(e.x, e.y);
                let (w, h) = (e.width * page.width, e.height * page.height);
                canvas.begin(x, y - h, e.rotation, e.opacity);
                if let Some(fill) = &e.fill {
                    let _ = writeln!(canvas.ops, "{} rg", rgb_op(ink(fill)));
                }
                if let Some(stroke) = &e.stroke {
                    let _ = writeln!(
                        canvas.ops,
                        "{} RG\n{:.3} w",
                        rgb_op(ink(stroke)),
                        e.stroke_width.max(0.1)
                    );
                }
                let paint = match (&e.fill, &e.stroke) {
                    (Some(_), Some(_)) => "B",
                    (Some(_), None) => "f",
                    _ => "S",
                };
                let _ = writeln!(canvas.ops, "0 0 {w:.3} {h:.3} re\n{paint}");
                canvas.end();
            }
        }
    }
}

/// Escape a string for a PDF literal, and drop what Helvetica cannot show.
///
/// WinAnsiEncoding covers Latin-1. Anything outside it would render as a
/// wrong glyph rather than nothing, so it is replaced visibly instead.
fn escape_pdf_text(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for c in raw.chars() {
        match c {
            '(' => out.push_str("\\("),
            ')' => out.push_str("\\)"),
            '\\' => out.push_str("\\\\"),
            '\n' | '\r' | '\t' => out.push(' '),
            c if c.is_ascii_graphic() || c == ' ' => out.push(c),
            // Everything else goes in as an octal escape of its WinAnsi byte.
            // A PDF string is bytes, and this stream is assembled as UTF-8
            // text, so pushing `é` directly would write two bytes the reader
            // decodes as `Ã©`. The escape keeps the stream ASCII and the
            // printed character correct.
            c => match fonts::winansi(c) {
                Some(b) => {
                    let _ = write!(out, "\\{b:03o}");
                }
                // Matches what the width table falls back to, so a name with
                // an unwritable character in it still comes out aligned.
                None => out.push('?'),
            },
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::Ecc;

    /// A minimal one-page PDF, built by hand so the test does not depend on a
    /// checked-in binary fixture.
    fn template(width: f32, height: f32) -> Vec<u8> {
        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let content = doc.add_object(Stream::new(
            dictionary! {},
            b"0.9 g 10 10 100 50 re f\n".to_vec(),
        ));
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => content,
            "MediaBox" => vec![0.into(), 0.into(), width.into(), height.into()],
            "Resources" => dictionary! {},
        });
        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => vec![page_id.into()],
                "Count" => 1,
            }),
        );
        let catalog = doc.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        doc.trailer.set("Root", catalog);
        let mut out = Vec::new();
        doc.save_to(&mut out).unwrap();
        out
    }

    /// A layout with just a code, at a known spot.
    fn place() -> Vec<Element> {
        vec![Element::Qr(QrElement {
            id: "qr".into(),
            x: 0.1,
            y: 0.1,
            size: 0.3,
            rotation: 0.0,
            opacity: 1.0,
        })]
    }

    fn text(template: &str, x: f32, y: f32, align: Align) -> Element {
        Element::Text(TextElement {
            id: format!("t{x}{y}"),
            x,
            y,
            template: template.into(),
            points: 12.0,
            font: Font::Helvetica,
            colour: "#112233".into(),
            align,
            rotation: 0.0,
            opacity: 1.0,
        })
    }

    fn stamp_of<'a>(qr: &'a Qr, pairs: &[(&str, &str)]) -> Stamp<'a> {
        let mut vars = Vars::default();
        for (k, v) in pairs {
            vars.set(*k, *v);
        }
        Stamp { qr, vars }
    }

    /// The operators of one stamped page, for asserting on directly.
    fn page_ops(bytes: &[u8], page: u32) -> String {
        let doc = Document::load_mem(bytes).unwrap();
        let dict = doc
            .get_object(doc.get_pages()[&page])
            .unwrap()
            .as_dict()
            .unwrap();
        let overlay = dict.get(b"Contents").unwrap().as_array().unwrap();
        let id = overlay.last().unwrap().as_reference().unwrap();
        let stream = doc.get_object(id).unwrap().as_stream().unwrap();
        String::from_utf8(
            stream
                .decompressed_content()
                .unwrap_or_else(|_| stream.content.clone()),
        )
        .unwrap()
    }

    #[test]
    fn reads_the_page_size_from_the_template() {
        let t = Template::load(&template(595.0, 842.0)).unwrap();
        let (w, h) = t.page_size();
        assert!((w - 595.0).abs() < 0.01, "got {w}");
        assert!((h - 842.0).abs() < 0.01, "got {h}");
    }

    #[test]
    fn produces_one_page_per_ticket() {
        let qr = Qr::encode("TKT1.abc.def", Ecc::M).unwrap();
        let stamps: Vec<Stamp> = (1..=5)
            .map(|n| stamp_of(&qr, &[("serial", &format!("{n:04}"))]))
            .collect();

        let t = Template::load(&template(595.0, 842.0)).unwrap();
        let out = t
            .stamp(
                &stamps,
                &[
                    place(),
                    vec![text("No. {{serial}}", 0.1, 0.45, Align::Left)],
                ]
                .concat(),
                4,
                Rgb::BLACK,
                Rgb::WHITE,
            )
            .unwrap();

        let doc = Document::load_mem(&out).unwrap();
        assert_eq!(doc.get_pages().len(), 5, "one page per ticket");
    }

    /// The template's own artwork has to survive, or the tickets come out
    /// blank with a code floating on them.
    #[test]
    fn the_template_content_is_still_referenced_by_every_page() {
        let qr = Qr::encode("TKT1.abc.def", Ecc::M).unwrap();
        let stamps = vec![stamp_of(&qr, &[]), stamp_of(&qr, &[])];
        let t = Template::load(&template(400.0, 400.0)).unwrap();
        let out = t
            .stamp(&stamps, &place(), 4, Rgb::BLACK, Rgb::WHITE)
            .unwrap();

        let doc = Document::load_mem(&out).unwrap();
        for (_, page_id) in doc.get_pages() {
            let page = doc.get_object(page_id).unwrap().as_dict().unwrap();
            let contents = page.get(b"Contents").unwrap().as_array().unwrap();
            assert_eq!(
                contents.len(),
                2,
                "each page is the template stream plus our overlay"
            );
        }
        // The template's rectangle should appear exactly once in the file,
        // shared by both pages rather than duplicated.
        let shared = doc.get_pages();
        let first = doc.get_object(shared[&1]).unwrap().as_dict().unwrap();
        let second = doc.get_object(shared[&2]).unwrap().as_dict().unwrap();
        assert_eq!(
            first.get(b"Contents").unwrap().as_array().unwrap()[0],
            second.get(b"Contents").unwrap().as_array().unwrap()[0],
            "the template stream must be shared, not copied per ticket"
        );
    }

    #[test]
    fn each_page_carries_its_own_code_image() {
        let a = Qr::encode("TKT1.aaa.aaa", Ecc::M).unwrap();
        let b = Qr::encode("TKT1.bbb.bbb", Ecc::M).unwrap();
        let stamps = vec![stamp_of(&a, &[]), stamp_of(&b, &[])];
        let t = Template::load(&template(400.0, 400.0)).unwrap();
        let out = t
            .stamp(&stamps, &place(), 4, Rgb::BLACK, Rgb::WHITE)
            .unwrap();

        let doc = Document::load_mem(&out).unwrap();
        let pages = doc.get_pages();
        let xobj = |n: u32| {
            let page = doc.get_object(pages[&n]).unwrap().as_dict().unwrap();
            let res = page.get(b"Resources").unwrap().as_dict().unwrap();
            let xo = res.get(b"XObject").unwrap().as_dict().unwrap();
            xo.get(b"QRC").unwrap().as_reference().unwrap()
        };
        assert_ne!(xobj(1), xobj(2), "two tickets must not share one code");
    }

    /// The end-to-end guarantee: a code that goes into the PDF comes back out
    /// of it and still decodes.
    ///
    /// Everything else here checks structure, which would pass just as
    /// happily if the image data were upside down, bit-reversed or off by a
    /// row. This reads the image stream back out of the finished file,
    /// unpacks it, and hands it to the decoder, so a packing bug cannot reach
    /// a print run.
    #[test]
    fn the_code_inside_the_finished_pdf_still_decodes() {
        let payload = "TKT1.U3VtbWVyfDEzNw.bXktc2lnbmF0dXJl";
        let qr = Qr::encode(payload, Ecc::M).unwrap();
        let size = qr.size();

        let t = Template::load(&template(595.0, 842.0)).unwrap();
        let out = t
            .stamp(&[stamp_of(&qr, &[])], &place(), 4, Rgb::BLACK, Rgb::WHITE)
            .unwrap();

        // Find the code image on the first page of the saved document.
        let doc = Document::load_mem(&out).unwrap();
        let page_id = doc.get_pages()[&1];
        let page = doc.get_object(page_id).unwrap().as_dict().unwrap();
        let res = page.get(b"Resources").unwrap().as_dict().unwrap();
        let image_id = res
            .get(b"XObject")
            .unwrap()
            .as_dict()
            .unwrap()
            .get(b"QRC")
            .unwrap()
            .as_reference()
            .unwrap();
        let stream = doc.get_object(image_id).unwrap().as_stream().unwrap();
        // The stored image is the code plus its quiet zone.
        let stored = size + QUIET * 2;
        assert_eq!(
            stream.dict.get(b"Width").unwrap().as_i64().unwrap(),
            stored as i64
        );

        // Unpack one bit per module back into a greyscale raster at a
        // resolution the decoder can lock onto. No margin is added here: if
        // the stored image does not already carry one, the decode fails,
        // which is precisely what this needs to catch.
        const S: usize = 4;
        const QUIET: usize = 4;
        // Small codes are stored raw, because deflating near-random module
        // data makes it bigger. Either form has to read back the same.
        let bits = match stream.dict.get(b"Filter") {
            Ok(_) => stream.decompressed_content().unwrap(),
            Err(_) => stream.content.clone(),
        };
        let row_bytes = stored.div_ceil(8);
        let side = stored * S;
        let mut pixels = vec![255u8; side * side];
        for y in 0..stored {
            for x in 0..stored {
                let set = bits[y * row_bytes + x / 8] & (0x80 >> (x % 8)) != 0;
                if set {
                    continue; // a set bit is a light module
                }
                for dy in 0..S {
                    for dx in 0..S {
                        pixels[(y * S + dy) * side + x * S + dx] = 0;
                    }
                }
            }
        }

        let luma = crate::render::Luma {
            width: side,
            height: side,
            pixels,
        };
        assert_eq!(
            crate::verify::decode_luma(&luma).as_deref(),
            Some(payload),
            "the code embedded in the PDF must read back as the ticket it was issued for"
        );
    }

    /// A two-page template, so page order and page retention can be checked.
    fn two_page_template() -> Vec<u8> {
        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let mut kids = Vec::new();
        for label in [b"front", b"back!"] {
            let content = doc.add_object(Stream::new(dictionary! {}, label.to_vec()));
            kids.push(
                doc.add_object(dictionary! {
                    "Type" => "Page",
                    "Parent" => pages_id,
                    "Contents" => content,
                    "MediaBox" => vec![0.into(), 0.into(), 400.into(), 400.into()],
                    "Resources" => dictionary! {},
                })
                .into(),
            );
        }
        let count = kids.len() as i64;
        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages", "Kids" => kids, "Count" => count,
            }),
        );
        let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        doc.trailer.set("Root", catalog);
        let mut out = Vec::new();
        doc.save_to(&mut out).unwrap();
        out
    }

    /// A two-sided design must stay two sided. Dropping the back while
    /// producing a plausible front is a bug you only find after printing.
    #[test]
    fn every_template_page_survives_into_every_ticket() {
        let qr = Qr::encode("TKT1.abc.def", Ecc::M).unwrap();
        let stamps: Vec<Stamp> = (0..3).map(|_| stamp_of(&qr, &[])).collect();

        let t = Template::load(&two_page_template()).unwrap();
        assert_eq!(t.page_count(), 2);
        let out = t
            .stamp(&stamps, &place(), 4, Rgb::BLACK, Rgb::WHITE)
            .unwrap();

        let doc = Document::load_mem(&out).unwrap();
        assert_eq!(doc.get_pages().len(), 6, "three tickets of two pages each");
    }

    /// The stamped page has to stay in its original position in the design.
    #[test]
    fn page_order_is_preserved_when_stamping_the_second_page() {
        let qr = Qr::encode("TKT1.abc.def", Ecc::M).unwrap();
        let t = Template::load(&two_page_template())
            .unwrap()
            .stamp_on(1)
            .unwrap();
        let out = t
            .stamp(&[stamp_of(&qr, &[])], &place(), 4, Rgb::BLACK, Rgb::WHITE)
            .unwrap();

        let doc = Document::load_mem(&out).unwrap();
        let pages = doc.get_pages();
        assert_eq!(pages.len(), 2);

        let has_code = |n: u32| {
            let page = doc.get_object(pages[&n]).unwrap().as_dict().unwrap();
            page.get(b"Resources")
                .and_then(|r| r.as_dict())
                .and_then(|r| r.get(b"XObject"))
                .and_then(|x| x.as_dict())
                .map(|x| x.has(b"QRC"))
                .unwrap_or(false)
        };
        assert!(!has_code(1), "the front must be left alone");
        assert!(has_code(2), "the code belongs on the page that was chosen");
    }

    #[test]
    fn asking_for_a_page_that_does_not_exist_is_refused() {
        let t = Template::load(&two_page_template()).unwrap();
        assert!(matches!(t.stamp_on(5), Err(PdfError::NoSuchPage(6))));
    }

    /// Without its own margin the code sits flush against the ticket artwork,
    /// which is exactly where scanners start failing.
    #[test]
    fn the_embedded_code_carries_its_quiet_zone() {
        let qr = Qr::encode("https://example.com", Ecc::M).unwrap();
        let quiet = 4;
        let stream = qr_image(&qr, quiet, Rgb::BLACK, Rgb::WHITE);
        let side = qr.size() + quiet * 2;
        assert_eq!(
            stream.dict.get(b"Width").unwrap().as_i64().unwrap(),
            side as i64,
            "the image has to include the margin, not assume the caller left room"
        );

        // Every pixel of the outermost row must be light.
        let row_bytes = side.div_ceil(8);
        let data = match stream.dict.get(b"Filter") {
            Ok(_) => stream.decompressed_content().unwrap(),
            Err(_) => stream.content.clone(),
        };
        for x in 0..side {
            let bit = data[x / 8] & (0x80 >> (x % 8));
            assert!(bit != 0, "top edge pixel {x} should be light");
        }
        for y in 0..side {
            let bit = data[y * row_bytes] & 0x80;
            assert!(bit != 0, "left edge pixel {y} should be light");
        }
    }

    /// Pins the coordinate mapping with numbers worked out by hand, so a sign
    /// flip or a forgotten box height cannot pass silently.
    #[test]
    fn the_placement_lands_where_the_arithmetic_says() {
        let qr = Qr::encode("https://example.com", Ecc::M).unwrap();
        let t = Template::load(&template(595.0, 842.0)).unwrap();
        let out = t
            .stamp(&[stamp_of(&qr, &[])], &place(), 4, Rgb::BLACK, Rgb::WHITE)
            .unwrap();

        // side = 0.3 * 595 = 178.5
        // x    = 0.1 * 595 = 59.5
        // y    = 842 - (0.1 * 842) - 178.5 = 579.3
        let doc = Document::load_mem(&out).unwrap();
        let page = doc
            .get_object(doc.get_pages()[&1])
            .unwrap()
            .as_dict()
            .unwrap();
        let overlay_id = page.get(b"Contents").unwrap().as_array().unwrap()[1]
            .as_reference()
            .unwrap();
        let stream = doc.get_object(overlay_id).unwrap().as_stream().unwrap();
        let ops = String::from_utf8(
            stream
                .decompressed_content()
                .unwrap_or(stream.content.clone()),
        )
        .unwrap();
        assert!(
            ops.contains("59.500 579.300 cm"),
            "the code should be translated to its corner:\n{ops}"
        );
        assert!(
            ops.contains("178.500 0 0 178.500 0 0 cm"),
            "and then scaled to its side length:\n{ops}"
        );
    }

    /// One ticket's overlay, for asserting on the operators directly.
    fn ops_for(layout: &[Element], pairs: &[(&str, &str)]) -> String {
        let qr = Qr::encode("TKT1.abc.def", Ecc::M).unwrap();
        let out = Template::load(&template(595.0, 842.0))
            .unwrap()
            .stamp(&[stamp_of(&qr, pairs)], layout, 4, Rgb::BLACK, Rgb::WHITE)
            .unwrap();
        page_ops(&out, 1)
    }

    /// Worked by hand. "Hi" in 12pt Helvetica is 722 + 222 = 944 units, so
    /// 11.328 points wide. Centred on x = 0.5 of a 595pt page, the cursor
    /// starts half that width to the left of 297.5.
    #[test]
    fn centred_text_starts_half_its_width_to_the_left() {
        let ops = ops_for(&[text("Hi", 0.5, 0.5, Align::Centre)], &[]);
        assert!(
            ops.contains("297.500 421.000 cm"),
            "the anchor should be the point given:\n{ops}"
        );
        assert!(
            ops.contains("-5.664 0 Td"),
            "and the cursor backed off by half the text width:\n{ops}"
        );
    }

    #[test]
    fn left_aligned_text_starts_exactly_at_its_anchor() {
        let ops = ops_for(&[text("Hi", 0.5, 0.5, Align::Left)], &[]);
        assert!(ops.contains("0.000 0 Td"), "got {ops}");
    }

    #[test]
    fn right_aligned_text_ends_at_its_anchor() {
        let ops = ops_for(&[text("Hi", 0.5, 0.5, Align::Right)], &[]);
        assert!(ops.contains("-11.328 0 Td"), "got {ops}");
    }

    /// The whole reason the metrics exist: two names of different lengths,
    /// centred on the same point, must end up with the same midpoint.
    #[test]
    fn two_names_of_different_lengths_centre_on_the_same_point() {
        let midpoint = |name: &str| {
            let ops = ops_for(&[text("{{n}}", 0.5, 0.5, Align::Centre)], &[("n", name)]);
            let td: f32 = ops
                .lines()
                .find(|l| l.ends_with(" 0 Td"))
                .and_then(|l| l.split_whitespace().next())
                .and_then(|v| v.parse().ok())
                .expect("a Td offset");
            let w = fonts::width(Font::Helvetica, name, 12.0);
            297.5 + td + w / 2.0
        };
        for name in ["Bo", "Marieke van Dijk"] {
            assert!(
                (midpoint(name) - 297.5).abs() < 0.01,
                "{name} centred at {}",
                midpoint(name)
            );
        }
    }

    #[test]
    fn a_text_element_is_filled_from_this_ticket_s_variables() {
        let ops = ops_for(
            &[text("{{first_name}} in {{seat}}", 0.1, 0.4, Align::Left)],
            &[("first_name", "Marieke"), ("seat", "A12")],
        );
        assert!(ops.contains("(Marieke in A12) Tj"), "got {ops}");
    }

    /// A column that is blank for this buyer leaves nothing behind, rather
    /// than an orphaned label or a stray brace.
    #[test]
    fn an_element_whose_variables_are_all_empty_draws_nothing() {
        let ops = ops_for(&[text("{{seat}}", 0.1, 0.4, Align::Left)], &[("seat", "")]);
        assert!(!ops.contains("Tj"), "nothing should be drawn:\n{ops}");
    }

    #[test]
    fn rotation_becomes_a_transform_matrix() {
        let mut element = text("Hi", 0.5, 0.5, Align::Left);
        if let Element::Text(t) = &mut element {
            t.rotation = 90.0;
        }
        let ops = ops_for(&[element], &[]);
        // cos 90 is 0 and sin 90 is 1, so the matrix turns the axes a
        // quarter turn.
        assert!(
            ops.contains("0.00000 1.00000 -1.00000 0.00000"),
            "got {ops}"
        );
    }

    #[test]
    fn no_rotation_leaves_the_identity_and_no_negative_zero() {
        let ops = ops_for(&[text("Hi", 0.5, 0.5, Align::Left)], &[]);
        assert!(ops.contains("1.00000 0.00000 0.00000 1.00000"), "got {ops}");
        assert!(
            !ops.contains("-0.00000"),
            "negative zero is valid but reads as a bug:\n{ops}"
        );
    }

    #[test]
    fn opacity_becomes_a_graphics_state_the_page_declares() {
        let mut element = text("Hi", 0.5, 0.5, Align::Left);
        if let Element::Text(t) = &mut element {
            t.opacity = 0.4;
        }
        let qr = Qr::encode("TKT1.abc.def", Ecc::M).unwrap();
        let out = Template::load(&template(595.0, 842.0))
            .unwrap()
            .stamp(&[stamp_of(&qr, &[])], &[element], 4, Rgb::BLACK, Rgb::WHITE)
            .unwrap();

        assert!(
            page_ops(&out, 1).contains(" gs"),
            "the stream selects a state"
        );

        let doc = Document::load_mem(&out).unwrap();
        let page = doc
            .get_object(doc.get_pages()[&1])
            .unwrap()
            .as_dict()
            .unwrap();
        let states = page
            .get(b"Resources")
            .and_then(|r| r.as_dict())
            .and_then(|r| r.get(b"ExtGState"))
            .and_then(|g| g.as_dict())
            .expect("the page must declare the state it uses");
        assert_eq!(states.len(), 1, "one state for one opacity");
    }

    #[test]
    fn a_full_opacity_element_needs_no_graphics_state() {
        let ops = ops_for(&[text("Hi", 0.5, 0.5, Align::Left)], &[]);
        assert!(!ops.contains(" gs"), "got {ops}");
    }

    #[test]
    fn a_filled_shape_paints_and_a_stroked_one_outlines() {
        let shape = |fill: Option<&str>, stroke: Option<&str>| {
            Element::Shape(ShapeElement {
                id: "s".into(),
                x: 0.1,
                y: 0.1,
                width: 0.2,
                height: 0.05,
                fill: fill.map(|s| s.to_string()),
                stroke: stroke.map(|s| s.to_string()),
                stroke_width: 1.0,
                rotation: 0.0,
                opacity: 1.0,
            })
        };
        assert!(ops_for(&[shape(Some("#ff0000"), None)], &[]).contains("re\nf"));
        assert!(ops_for(&[shape(None, Some("#00ff00"))], &[]).contains("re\nS"));
        assert!(ops_for(&[shape(Some("#ff0000"), Some("#00ff00"))], &[]).contains("re\nB"));
        assert!(
            !ops_for(&[shape(None, None)], &[]).contains(" re"),
            "a shape with neither is a placeholder, not an error"
        );
    }

    /// The list is the stacking order, so a later element draws over an
    /// earlier one.
    #[test]
    fn elements_are_drawn_in_the_order_they_are_listed() {
        let ops = ops_for(
            &[
                text("first", 0.1, 0.1, Align::Left),
                text("second", 0.1, 0.2, Align::Left),
            ],
            &[],
        );
        assert!(
            ops.find("(first)").unwrap() < ops.find("(second)").unwrap(),
            "got {ops}"
        );
    }

    #[test]
    fn each_element_is_bracketed_so_nothing_leaks_into_the_template() {
        let ops = ops_for(
            &[
                text("a", 0.1, 0.1, Align::Left),
                text("b", 0.1, 0.2, Align::Left),
            ],
            &[],
        );
        // Counted as whole lines. `/QRF` contains a Q, so matching bare
        // characters counted the font name as a restore.
        let count = |token: &str| ops.lines().filter(|l| l.trim() == token).count();
        assert!(count("q") > 0, "something should have been drawn:\n{ops}");
        assert_eq!(
            count("q"),
            count("Q"),
            "every save is matched by a restore:\n{ops}"
        );
    }

    #[test]
    fn a_rotated_template_is_refused_rather_than_misplaced() {
        let mut doc = Document::load_mem(&template(595.0, 842.0)).unwrap();
        let page_id = *doc.get_pages().values().next().unwrap();
        if let Ok(Object::Dictionary(d)) = doc.get_object_mut(page_id) {
            d.set("Rotate", Object::Integer(90));
        }
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).unwrap();

        assert!(matches!(Template::load(&bytes), Err(PdfError::Rotated(90))));
    }

    #[test]
    fn rejects_something_that_is_not_a_pdf() {
        assert!(Template::load(b"definitely not a pdf").is_err());
    }

    /// The exact objects `newElement` in `src/lib/api.js` builds.
    ///
    /// This is the one seam the rest of the suite cannot see: the editor
    /// creates elements in JavaScript and they arrive here as JSON. A renamed
    /// field would compile on both sides and fail only when somebody added an
    /// element and tried to save it. Copied verbatim from that factory, so
    /// changing one without the other fails here.
    #[test]
    fn the_editors_new_elements_deserialise() {
        let cases = [
            r##"{"id":"qr-a1b2c3","kind":"qr","x":0.1,"y":0.2,"rotation":0,
                 "opacity":1,"size":0.26}"##,
            r##"{"id":"text-a1b2c3","kind":"text","x":0.1,"y":0.2,"rotation":0,
                 "opacity":1,"template":"{{first_name}}","points":14,
                 "font":"helvetica","colour":"#101014","align":"left"}"##,
            r##"{"id":"image-a1b2c3","kind":"image","x":0.1,"y":0.2,"rotation":0,
                 "opacity":1,"width":0.15,"height":0.08,"column":"logo"}"##,
            r##"{"id":"shape-a1b2c3","kind":"shape","x":0.1,"y":0.2,"rotation":0,
                 "opacity":1,"width":0.3,"height":0.002,"fill":"#101014",
                 "stroke":null,"strokeWidth":1}"##,
        ];
        for json in cases {
            let element: Element = serde_json::from_str(json)
                .unwrap_or_else(|e| panic!("api.js and pdf.rs disagree: {e}\n{json}"));
            // And it survives the trip back, which is what autosave does.
            let round: Element =
                serde_json::from_str(&serde_json::to_string(&element).unwrap()).unwrap();
            assert_eq!(
                serde_json::to_value(&element).unwrap(),
                serde_json::to_value(&round).unwrap()
            );
        }
    }

    /// Every font and alignment the editor's dropdowns offer.
    #[test]
    fn the_editors_font_and_alignment_names_are_understood() {
        for name in ["helvetica", "helveticaBold", "courier"] {
            let font: Font = serde_json::from_str(&format!("\"{name}\"")).unwrap();
            assert_eq!(
                serde_json::to_string(&font).unwrap(),
                format!("\"{name}\""),
                "{name} does not round-trip"
            );
        }
        for name in ["left", "centre", "right"] {
            let align: Align = serde_json::from_str(&format!("\"{name}\"")).unwrap();
            assert_eq!(
                serde_json::to_string(&align).unwrap(),
                format!("\"{name}\"")
            );
        }
    }

    #[test]
    fn text_escaping_protects_the_delimiters() {
        assert_eq!(escape_pdf_text("A (B) C"), "A \\(B\\) C");
        assert_eq!(escape_pdf_text(r"back\slash"), r"back\\slash");
        assert_eq!(escape_pdf_text("line\nbreak"), "line break");
        assert_eq!(
            escape_pdf_text("\u{4e2d}"),
            "?",
            "outside the encoding is visible"
        );
    }

    /// A PDF string is bytes, and this stream is built as UTF-8 text. Writing
    /// `é` into it directly put two bytes in, which the reader decoded as
    /// `Ã©`: a name printed wrong on every ticket. Octal escapes keep the
    /// stream ASCII and the printed character right.
    ///
    /// The expected values are the characters' WinAnsi bytes in octal, taken
    /// from the encoding table rather than from what this code produces:
    /// é is 0xE9 = 351, · is 0xB7 = 267, € is 0x80 = 200.
    #[test]
    fn non_ascii_text_goes_in_as_its_winansi_byte() {
        assert_eq!(escape_pdf_text("caf\u{e9}"), r"caf\351");
        assert_eq!(escape_pdf_text("a \u{b7} b"), r"a \267 b");
        assert_eq!(escape_pdf_text("\u{20ac}5"), r"\2005");
        // Every byte written is one character on the page, so the escaped
        // form must never contain a raw byte above 127.
        let escaped = escape_pdf_text("Ramón, Jörg, Łukasz, \u{2019}24");
        assert!(escaped.is_ascii(), "still has raw bytes: {escaped}");
    }

    /// One bit per module, rows padded to whole bytes, margin included.
    #[test]
    fn the_code_image_is_one_bit_per_module() {
        let qr = Qr::encode("https://example.com", Ecc::M).unwrap();
        let stream = qr_image(&qr, 4, Rgb::BLACK, Rgb::WHITE);
        assert_eq!(
            stream.dict.get(b"Width").unwrap().as_i64().unwrap(),
            (qr.size() + 8) as i64
        );
        assert_eq!(
            stream
                .dict
                .get(b"BitsPerComponent")
                .unwrap()
                .as_i64()
                .unwrap(),
            1
        );
    }
}

/// Development aids, ignored by default.
///
/// A PDF is the one thing here that can pass every structural assertion and
/// still print wrong, so these produce a real file and check it the way a door
/// scanner would. The middle step is a script rather than a test because
/// rasterising needs a canvas backend that is not worth carrying as a
/// dependency.
///
///   cargo test --lib sample_ticket_pdf -- --ignored --nocapture
///   npm install --no-save @napi-rs/canvas
///   node scripts/render-pdf-page.mjs %TEMP%/qrgen-sample-tickets.pdf %TEMP%/qrgen-ticket-p1.png 1 3
///   (repeat for pages 2 and 3)
///   cargo test --lib every_rendered_ticket_verifies -- --ignored --nocapture
#[cfg(test)]
mod dev_aids {
    use super::*;
    use crate::render::Ecc;

    const SEED: [u8; 32] = [7u8; 32];
    const EVENT: &str = "Summer Fest";

    /// Builds a stamp from name/value pairs, as the tests module does.
    fn stamp_of<'a>(qr: &'a Qr, pairs: &[(&str, &str)]) -> Stamp<'a> {
        let mut vars = Vars::default();
        for (k, v) in pairs {
            vars.set(*k, *v);
        }
        Stamp { qr, vars }
    }

    /// A layout close to what a real ticket carries, for the visual check.
    fn sample_layout() -> Vec<Element> {
        vec![
            Element::Qr(QrElement {
                id: "qr".into(),
                x: 0.62,
                y: 0.12,
                size: 0.26,
                rotation: 0.0,
                opacity: 1.0,
            }),
            Element::Text(TextElement {
                id: "name".into(),
                x: 0.12,
                y: 0.30,
                template: "{{first_name}} {{last_name}}".into(),
                points: 20.0,
                font: Font::HelveticaBold,
                colour: "#101014".into(),
                align: Align::Left,
                rotation: 0.0,
                opacity: 1.0,
            }),
            Element::Text(TextElement {
                id: "seat".into(),
                x: 0.12,
                y: 0.36,
                template: "Seat {{seat}}".into(),
                points: 12.0,
                font: Font::Helvetica,
                colour: "#44444c".into(),
                align: Align::Left,
                rotation: 0.0,
                opacity: 1.0,
            }),
            Element::Text(TextElement {
                id: "serial".into(),
                x: 0.75,
                y: 0.46,
                template: "No. {{serial}}".into(),
                points: 11.0,
                font: Font::Helvetica,
                colour: "#101014".into(),
                align: Align::Centre,
                rotation: 0.0,
                opacity: 1.0,
            }),
            // A hairline rule under the name. Its y is the top of the box, and
            // it is drawn downwards from there, unlike text.
            Element::Shape(ShapeElement {
                id: "rule".into(),
                x: 0.12,
                y: 0.32,
                width: 0.42,
                height: 0.0015,
                fill: Some("#101014".into()),
                stroke: None,
                stroke_width: 1.0,
                rotation: 0.0,
                opacity: 1.0,
            }),
            // Right-aligned, so its text ends at x rather than starting there.
            Element::Text(TextElement {
                id: "tier".into(),
                x: 0.88,
                y: 0.58,
                template: "{{tier}}".into(),
                points: 13.0,
                font: Font::HelveticaBold,
                colour: "#2288bf".into(),
                align: Align::Right,
                rotation: 0.0,
                opacity: 1.0,
            }),
            // Turned a quarter anticlockwise about its own baseline start, so
            // it reads bottom-to-top up the left edge.
            Element::Text(TextElement {
                id: "stub".into(),
                x: 0.06,
                y: 0.92,
                template: "{{event}} \u{b7} {{serial}}".into(),
                points: 9.0,
                font: Font::Helvetica,
                colour: "#8a8a94".into(),
                align: Align::Left,
                rotation: 90.0,
                opacity: 0.75,
            }),
        ]
    }

    #[test]
    #[ignore]
    fn sample_ticket_pdf() {
        let key = crate::tickets::EventKey::from_seed(&SEED);
        let codes: Vec<Qr> = (1..=3)
            .map(|n| Qr::encode(&key.issue(EVENT, n), Ecc::M).unwrap())
            .collect();
        // Accented on purpose. A PDF string is bytes in WinAnsi, so these are
        // the names that catch an encoder writing UTF-8 into one.
        let people = [
            ("Marieke", "van Dijk", "A12", "Early bird"),
            ("Tomás", "Reyes", "A13", "General"),
            ("Renée", "Sørensen", "B04", "VIP"),
        ];
        let stamps: Vec<Stamp> = codes
            .iter()
            .zip(people)
            .enumerate()
            .map(|(i, (qr, (first, last, seat, tier)))| {
                stamp_of(
                    qr,
                    &[
                        ("first_name", first),
                        ("last_name", last),
                        ("seat", seat),
                        ("tier", tier),
                        ("event", EVENT),
                        ("serial", &format!("{:04}", i + 1)),
                    ],
                )
            })
            .collect();

        let out = Template::blank(595.276, 841.89)
            .stamp(&stamps, &sample_layout(), 4, Rgb::BLACK, Rgb::WHITE)
            .unwrap();

        let path = std::env::temp_dir().join("qrgen-sample-tickets.pdf");
        std::fs::write(&path, &out).unwrap();
        println!("wrote {} ({} bytes)", path.display(), out.len());
    }

    /// The complete chain: issue, stamp, rasterise the real PDF with an
    /// independent renderer, decode the picture, verify the signature.
    #[test]
    #[ignore]
    fn every_rendered_ticket_verifies() {
        let key = crate::tickets::EventKey::from_seed(&SEED);
        let public = key.public_key();
        let mut seen = Vec::new();

        for page in 1..=3u32 {
            let file = std::env::temp_dir().join(format!("qrgen-ticket-p{page}.png"));
            let bytes = std::fs::read(&file)
                .unwrap_or_else(|e| panic!("render {} first: {e}", file.display()));
            let found = crate::decode::decode_bytes(&bytes)
                .unwrap_or_else(|e| panic!("page {page} did not decode: {e}"));
            assert_eq!(found.len(), 1, "one code per ticket");

            let claim = crate::tickets::verify(&public, &found[0])
                .unwrap_or_else(|e| panic!("page {page} failed verification: {e}"));
            println!("page {page}: {} #{}", claim.event, claim.serial);
            assert_eq!(claim.event, EVENT);
            assert_eq!(claim.serial, page);
            seen.push(found[0].clone());
        }

        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), 3, "every ticket must carry a different code");
    }
}
