//! Modules in, pixels out.
//!
//! This is the deep module of the app. Its interface is small (a matrix plus
//! a style) and it hides every output format behind that. The preview, the
//! saved SVG, the saved PNG and the scannability check are all driven from one
//! matrix, so they cannot disagree with each other.
//!
//! Rendering happens here rather than through the `qrcode` crate's own SVG
//! writer because that writer emits one rect per module and has no way to
//! express a logo hole or rounded modules, both of which are on the roadmap.

use qrcode::{EcLevel, QrCode};
use serde::{Deserialize, Serialize};
use std::fmt::Write as _;

/// Refuse to allocate a raster larger than this on a side.
///
/// A stray zero in a size field should produce an error, not a multi-gigabyte
/// allocation that takes the whole app down with it.
const MAX_PIXELS_PER_SIDE: u32 = 8192;

/// Resolution used for the scannability check.
///
/// Four pixels per module is well above what rqrr needs to lock on, and keeps
/// the check fast enough to run on every keystroke.
const VERIFY_SCALE: u32 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Ecc {
    L,
    M,
    Q,
    H,
}

impl From<Ecc> for EcLevel {
    fn from(e: Ecc) -> Self {
        match e {
            Ecc::L => EcLevel::L,
            Ecc::M => EcLevel::M,
            Ecc::Q => EcLevel::Q,
            Ecc::H => EcLevel::H,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub const BLACK: Rgb = Rgb(0, 0, 0);
    pub const WHITE: Rgb = Rgb(255, 255, 255);

    /// Parse `#rgb` or `#rrggbb`, with or without the leading hash.
    ///
    /// Returns `None` rather than a default so the caller decides what a bad
    /// value means. Colours arrive from the webview, which is a trust
    /// boundary: this is the only place a colour string becomes a colour.
    pub fn from_hex(s: &str) -> Option<Rgb> {
        let h = s.trim().trim_start_matches('#');
        if !h.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let pair = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
        let nibble = |i: usize| u8::from_str_radix(&h[i..i + 1], 16).ok();
        match h.len() {
            // Shorthand doubles each nibble, so f -> ff and a -> aa.
            3 => Some(Rgb(nibble(0)? * 17, nibble(1)? * 17, nibble(2)? * 17)),
            6 => Some(Rgb(pair(0)?, pair(2)?, pair(4)?)),
            _ => None,
        }
    }

    pub fn to_hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.0, self.1, self.2)
    }

    /// Rec. 601 luma, which is what a greyscale conversion produces.
    pub fn luma(self) -> u8 {
        (0.299 * self.0 as f32 + 0.587 * self.1 as f32 + 0.114 * self.2 as f32).round() as u8
    }

    /// WCAG relative luminance, the input to the contrast ratio.
    fn relative_luminance(self) -> f32 {
        fn ch(c: u8) -> f32 {
            let s = c as f32 / 255.0;
            if s <= 0.03928 {
                s / 12.92
            } else {
                ((s + 0.055) / 1.055).powf(2.4)
            }
        }
        0.2126 * ch(self.0) + 0.7152 * ch(self.1) + 0.0722 * ch(self.2)
    }
}

/// WCAG contrast ratio between two colours, from 1.0 (identical) to 21.0.
pub fn contrast_ratio(a: Rgb, b: Rgb) -> f32 {
    let (la, lb) = (a.relative_luminance(), b.relative_luminance());
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

#[derive(Debug, Clone, Copy)]
pub struct Style {
    /// Pixels per module in the PNG. The SVG is resolution independent and
    /// uses this only to set its width and height attributes.
    pub scale: u32,
    /// Margin in modules. The QR spec calls for 4; below that, scanners start
    /// failing against busy backgrounds.
    pub quiet_zone: u32,
    pub dark: Rgb,
    pub light: Rgb,
    /// Fraction of the code's width taken up by the logo box, when there is a
    /// logo. Ignored otherwise.
    pub logo_fraction: f32,
}

impl Default for Style {
    fn default() -> Self {
        Style {
            scale: 8,
            quiet_zone: 4,
            dark: Rgb::BLACK,
            light: Rgb::WHITE,
            logo_fraction: 0.20,
        }
    }
}

impl Style {
    /// Clamp every field into a range that can actually be rendered.
    ///
    /// Runs on values that came from the webview, before any allocation is
    /// sized from them.
    pub fn sanitized(mut self, modules: usize) -> Style {
        self.quiet_zone = self.quiet_zone.min(16);
        let total = modules as u32 + 2 * self.quiet_zone;
        let max_scale = (MAX_PIXELS_PER_SIDE / total).max(1);
        self.scale = self.scale.clamp(1, max_scale);
        // Above roughly a third, the logo eats the timing patterns and the
        // code stops being recoverable at any error correction level.
        self.logo_fraction = self.logo_fraction.clamp(0.05, 0.35);
        self
    }

    /// The logo box in module units: `(x, y, side)`, square and centred.
    ///
    /// Returned in module units rather than pixels so the SVG and the raster
    /// place it identically, which is what keeps the preview honest.
    fn logo_box(&self, modules: usize) -> (f32, f32, f32) {
        let size = modules as f32;
        let side = (size * self.logo_fraction).round().max(1.0);
        let origin = self.quiet_zone as f32 + (size - side) / 2.0;
        (origin, origin, side)
    }
}

/// An image to drop in the middle of the code.
///
/// Holds both the decoded pixels, for the raster outputs, and the original
/// file bytes as a data URI, so the exported SVG stays a single self-contained
/// file rather than a document with a broken external reference.
pub struct Logo {
    rgba: image::RgbaImage,
    data_uri: String,
}

impl Logo {
    /// Decode an image and prepare it for both output paths.
    ///
    /// The original bytes are kept verbatim for the SVG rather than
    /// re-encoded, so a logo the designer supplied lands in the file exactly
    /// as they made it.
    pub fn load(bytes: &[u8]) -> Result<Logo, RenderError> {
        let format = image::guess_format(bytes)
            .map_err(|_| RenderError::Logo("Unrecognised image format.".into()))?;
        let mime = match format {
            image::ImageFormat::Png => "image/png",
            image::ImageFormat::Jpeg => "image/jpeg",
            image::ImageFormat::Gif => "image/gif",
            image::ImageFormat::WebP => "image/webp",
            _ => {
                return Err(RenderError::Logo(
                    "Use a PNG, JPEG, GIF or WebP file.".into(),
                ))
            }
        };
        let rgba = image::load_from_memory_with_format(bytes, format)
            .map_err(|e| RenderError::Logo(format!("Could not read the image: {e}")))?
            .to_rgba8();

        Ok(Logo {
            rgba,
            data_uri: format!("data:{mime};base64,{}", crate::b64::encode(bytes)),
        })
    }

    pub fn dimensions(&self) -> (u32, u32) {
        self.rgba.dimensions()
    }
}

#[derive(Debug)]
pub enum RenderError {
    Empty,
    TooLong,
    Encode(String),
    Png(String),
    Logo(String),
}

impl std::fmt::Display for RenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            RenderError::Empty => write!(f, "Nothing to encode yet."),
            RenderError::TooLong => write!(
                f,
                "Too much data for one QR code. Shorten the text, or lower the error correction level."
            ),
            RenderError::Encode(e) => write!(f, "Could not build the QR code: {e}"),
            RenderError::Png(e) => write!(f, "Could not write the PNG: {e}"),
            RenderError::Logo(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for RenderError {}

/// A finished QR symbol: a square matrix of dark and light modules.
pub struct Qr {
    size: usize,
    modules: Vec<bool>,
}

impl Qr {
    pub fn encode(data: &str, ecc: Ecc) -> Result<Qr, RenderError> {
        if data.is_empty() {
            return Err(RenderError::Empty);
        }
        let code = QrCode::with_error_correction_level(data.as_bytes(), ecc.into()).map_err(
            |e| match e {
                qrcode::types::QrError::DataTooLong => RenderError::TooLong,
                other => RenderError::Encode(other.to_string()),
            },
        )?;
        let size = code.width();
        let modules = code
            .to_colors()
            .into_iter()
            .map(|c| c == qrcode::Color::Dark)
            .collect();
        Ok(Qr { size, modules })
    }

    pub fn size(&self) -> usize {
        self.size
    }

    #[inline]
    pub fn dark(&self, x: usize, y: usize) -> bool {
        self.modules[y * self.size + x]
    }

    /// Side length in modules, including the quiet zone on both sides.
    fn total(&self, style: &Style) -> u32 {
        self.size as u32 + 2 * style.quiet_zone
    }

    /// Call `f(x, y, run_length)` for every horizontal run of dark modules.
    ///
    /// Merging runs matters: a 33-module code holds roughly 500 dark modules
    /// but only about 150 runs, so both the SVG path and the raster loop get
    /// noticeably shorter for free.
    fn for_each_run(&self, mut f: impl FnMut(usize, usize, usize)) {
        for y in 0..self.size {
            let mut x = 0;
            while x < self.size {
                if !self.dark(x, y) {
                    x += 1;
                    continue;
                }
                let start = x;
                while x < self.size && self.dark(x, y) {
                    x += 1;
                }
                f(start, y, x - start);
            }
        }
    }
}

/// Render to a standalone SVG document.
///
/// The viewBox is in module units, so the file scales to any size without
/// resampling. `shape-rendering="crispEdges"` is load bearing: without it,
/// renderers antialias the seams between adjacent module rectangles and leave
/// pale hairlines that some scanners read as light modules.
pub fn to_svg(qr: &Qr, style: &Style, logo: Option<&Logo>) -> String {
    let style = style.sanitized(qr.size());
    let total = qr.total(&style);
    let px = total * style.scale;
    let q = style.quiet_zone as usize;

    let mut d = String::new();
    qr.for_each_run(|x, y, len| {
        let _ = write!(d, "M{} {}h{len}v1h-{len}z", x + q, y + q);
    });

    let mut overlay = String::new();
    if let Some(logo) = logo {
        let (x, y, side) = style.logo_box(qr.size());
        // A one-module margin of background around the logo. Without it the
        // logo abuts live modules and scanners lose the run-length rhythm at
        // the boundary even when the payload is still recoverable.
        let pad = 1.0;
        let (bx, by, bside) = (x - pad, y - pad, side + pad * 2.0);
        let _ = write!(
            overlay,
            "<rect x=\"{bx}\" y=\"{by}\" width=\"{bside}\" height=\"{bside}\" rx=\"{rx}\" fill=\"{light}\"/>\
             <image x=\"{x}\" y=\"{y}\" width=\"{side}\" height=\"{side}\" \
             preserveAspectRatio=\"xMidYMid meet\" href=\"{href}\"/>",
            rx = bside * 0.12,
            light = style.light.to_hex(),
            href = logo.data_uri,
        );
    }

    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{px}\" height=\"{px}\" \
         viewBox=\"0 0 {total} {total}\" shape-rendering=\"crispEdges\">\
         <rect width=\"{total}\" height=\"{total}\" fill=\"{light}\"/>\
         <path d=\"{d}\" fill=\"{dark}\"/>\
         {overlay}</svg>",
        px = px,
        total = total,
        light = style.light.to_hex(),
        dark = style.dark.to_hex(),
        d = d,
        overlay = overlay,
    )
}

/// Scale the logo into a square of `side` pixels and alpha-blend it over
/// `blend`, which receives `(x, y, r, g, b)` for each covered pixel.
///
/// Shared by the PNG and greyscale paths so the scannability check sees
/// exactly the coverage the exported image has.
fn draw_logo(
    logo: &Logo,
    style: &Style,
    modules: usize,
    mut blend: impl FnMut(usize, usize, [u8; 3], u8),
) {
    let (bx, by, bside) = style.logo_box(modules);
    let s = style.scale as f32;
    let box_px = (bside * s).round().max(1.0) as u32;

    let (lw, lh) = logo.rgba.dimensions();
    if lw == 0 || lh == 0 {
        return;
    }
    // Fit inside the box preserving aspect, matching the SVG's
    // `preserveAspectRatio="xMidYMid meet"`.
    let ratio = (box_px as f32 / lw as f32).min(box_px as f32 / lh as f32);
    let (dw, dh) = (
        (lw as f32 * ratio).round().max(1.0) as u32,
        (lh as f32 * ratio).round().max(1.0) as u32,
    );
    let scaled = image::imageops::resize(&logo.rgba, dw, dh, image::imageops::FilterType::Triangle);

    let x0 = (bx * s).round() as i64 + ((box_px - dw) / 2) as i64;
    let y0 = (by * s).round() as i64 + ((box_px - dh) / 2) as i64;

    for (lx, ly, px) in scaled.enumerate_pixels() {
        if px.0[3] == 0 {
            continue;
        }
        let (tx, ty) = (x0 + lx as i64, y0 + ly as i64);
        if tx < 0 || ty < 0 {
            continue;
        }
        blend(
            tx as usize,
            ty as usize,
            [px.0[0], px.0[1], px.0[2]],
            px.0[3],
        );
    }
}

/// Straight-alpha composite of one channel.
#[inline]
fn over(src: u8, dst: u8, alpha: u8) -> u8 {
    let a = alpha as u32;
    ((src as u32 * a + dst as u32 * (255 - a)) / 255) as u8
}

/// Fill the background plate the logo sits on, in pixel space.
fn logo_plate(style: &Style, modules: usize) -> (usize, usize, usize) {
    let (bx, by, bside) = style.logo_box(modules);
    let s = style.scale as f32;
    let pad = s;
    let x = ((bx * s) - pad).round().max(0.0) as usize;
    let y = ((by * s) - pad).round().max(0.0) as usize;
    let side = ((bside * s) + pad * 2.0).round() as usize;
    (x, y, side)
}

/// Render to PNG bytes.
pub fn to_png(qr: &Qr, style: &Style, logo: Option<&Logo>) -> Result<Vec<u8>, RenderError> {
    let style = style.sanitized(qr.size());
    let px = qr.total(&style) * style.scale;

    let light_px = image::Rgb([style.light.0, style.light.1, style.light.2]);
    let mut img = image::RgbImage::from_pixel(px, px, light_px);
    let dark = [style.dark.0, style.dark.1, style.dark.2];
    let light = [style.light.0, style.light.1, style.light.2];
    let (q, s) = (style.quiet_zone, style.scale);
    let side = px as usize;
    let stride = side * 3;
    let buf = img.as_mut();

    qr.for_each_run(|x, y, len| {
        let x0 = ((x as u32 + q) * s) as usize;
        let y0 = ((y as u32 + q) * s) as usize;
        let width = len * s as usize;
        for row in y0..y0 + s as usize {
            let start = row * stride + x0 * 3;
            let end = start + width * 3;
            for px in buf[start..end].chunks_exact_mut(3) {
                px.copy_from_slice(&dark);
            }
        }
    });

    if let Some(logo) = logo {
        let (px0, py0, plate) = logo_plate(&style, qr.size());
        for row in py0..(py0 + plate).min(side) {
            let start = row * stride + px0.min(side) * 3;
            let end = (start + plate * 3).min(row * stride + stride);
            for p in buf[start..end].chunks_exact_mut(3) {
                p.copy_from_slice(&light);
            }
        }
        draw_logo(logo, &style, qr.size(), |x, y, rgb, alpha| {
            if x >= side || y >= side {
                return;
            }
            let i = y * stride + x * 3;
            for c in 0..3 {
                buf[i + c] = over(rgb[c], buf[i + c], alpha);
            }
        });
    }

    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png)
        .map_err(|e| RenderError::Png(e.to_string()))?;
    Ok(out.into_inner())
}

/// A greyscale raster, the input the scannability check reads.
pub struct Luma {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u8>,
}

/// Render to greyscale at a fixed small scale, for the scannability check.
///
/// The two colours are converted to luma exactly as a camera would see them,
/// so a low-contrast pair produces a raster that genuinely is hard to read,
/// rather than one quietly normalised back to black on white.
pub fn to_luma(qr: &Qr, style: &Style, logo: Option<&Logo>) -> Luma {
    let style = Style {
        scale: VERIFY_SCALE,
        ..*style
    }
    .sanitized(qr.size());
    let side = (qr.total(&style) * style.scale) as usize;
    let light = style.light.luma();
    let mut pixels = vec![light; side * side];
    let dark = style.dark.luma();
    let (q, s) = (style.quiet_zone, style.scale);

    qr.for_each_run(|x, y, len| {
        let x0 = ((x as u32 + q) * s) as usize;
        let y0 = ((y as u32 + q) * s) as usize;
        let width = len * s as usize;
        for row in y0..y0 + s as usize {
            let start = row * side + x0;
            pixels[start..start + width].fill(dark);
        }
    });

    // The logo has to be in the raster the check reads, or an overlay that
    // covers half the code would still be reported as scanning perfectly.
    if let Some(logo) = logo {
        let (px0, py0, plate) = logo_plate(&style, qr.size());
        for row in py0..(py0 + plate).min(side) {
            let start = row * side + px0.min(side);
            let end = (start + plate).min(row * side + side);
            pixels[start..end].fill(light);
        }
        draw_logo(logo, &style, qr.size(), |x, y, rgb, alpha| {
            if x >= side || y >= side {
                return;
            }
            let src = Rgb(rgb[0], rgb[1], rgb[2]).luma();
            let i = y * side + x;
            pixels[i] = over(src, pixels[i], alpha);
        });
    }

    Luma {
        width: side,
        height: side,
        pixels,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_parsing_accepts_both_lengths_and_rejects_junk() {
        assert_eq!(Rgb::from_hex("#ffffff"), Some(Rgb::WHITE));
        assert_eq!(Rgb::from_hex("000000"), Some(Rgb::BLACK));
        assert_eq!(Rgb::from_hex("#f00"), Some(Rgb(255, 0, 0)));
        assert_eq!(Rgb::from_hex("#abc"), Some(Rgb(170, 187, 204)));
        for bad in ["", "#", "#ff", "#fffff", "#gggggg", "not a colour"] {
            assert_eq!(Rgb::from_hex(bad), None, "{bad:?} should not parse");
        }
    }

    #[test]
    fn hex_round_trips() {
        let c = Rgb(0x22, 0x88, 0xbf);
        assert_eq!(Rgb::from_hex(&c.to_hex()), Some(c));
    }

    /// Anchored to the ratios WCAG defines, not to what the code computes.
    #[test]
    fn contrast_matches_known_wcag_values() {
        let bw = contrast_ratio(Rgb::BLACK, Rgb::WHITE);
        assert!((bw - 21.0).abs() < 0.01, "black on white is 21:1, got {bw}");
        let same = contrast_ratio(Rgb::WHITE, Rgb::WHITE);
        assert!(
            (same - 1.0).abs() < 0.01,
            "identical colours are 1:1, got {same}"
        );
        assert_eq!(
            contrast_ratio(Rgb::BLACK, Rgb::WHITE),
            contrast_ratio(Rgb::WHITE, Rgb::BLACK),
            "order must not matter"
        );
    }

    #[test]
    fn empty_input_is_an_error_not_a_blank_code() {
        assert!(matches!(Qr::encode("", Ecc::M), Err(RenderError::Empty)));
    }

    #[test]
    fn oversized_input_reports_too_long() {
        let huge = "x".repeat(8000);
        assert!(matches!(
            Qr::encode(&huge, Ecc::H),
            Err(RenderError::TooLong)
        ));
    }

    /// Version 1 is 21x21 and every later version adds 4 modules per side.
    #[test]
    fn module_count_follows_the_qr_spec() {
        let qr = Qr::encode("HELLO", Ecc::L).unwrap();
        assert_eq!(qr.size(), 21, "a short payload at ECC L fits version 1");
        let big = Qr::encode(&"x".repeat(400), Ecc::L).unwrap();
        assert_eq!((big.size() - 21) % 4, 0, "sizes step by 4");
    }

    #[test]
    fn quiet_zone_is_light_all_the_way_round() {
        let qr = Qr::encode("https://example.com", Ecc::M).unwrap();
        let style = Style::default();
        let luma = to_luma(&qr, &style, None);
        let light = Rgb::WHITE.luma();
        for x in 0..luma.width {
            assert_eq!(luma.pixels[x], light, "top edge must be quiet");
            let bottom = (luma.height - 1) * luma.width + x;
            assert_eq!(luma.pixels[bottom], light, "bottom edge must be quiet");
        }
        for y in 0..luma.height {
            assert_eq!(
                luma.pixels[y * luma.width],
                light,
                "left edge must be quiet"
            );
            let right = y * luma.width + luma.width - 1;
            assert_eq!(luma.pixels[right], light, "right edge must be quiet");
        }
    }

    #[test]
    fn svg_is_well_formed_and_carries_the_chosen_colours() {
        let qr = Qr::encode("https://example.com", Ecc::M).unwrap();
        let style = Style {
            dark: Rgb(0x11, 0x22, 0x33),
            light: Rgb(0xee, 0xdd, 0xcc),
            ..Default::default()
        };
        let svg = to_svg(&qr, &style, None);
        assert!(svg.starts_with("<svg xmlns="));
        assert!(svg.ends_with("</svg>"));
        assert!(svg.contains("shape-rendering=\"crispEdges\""));
        assert!(svg.contains("#112233"), "dark colour must reach the path");
        assert!(
            svg.contains("#eeddcc"),
            "light colour must reach the background"
        );
        let total = qr.size() + 2 * style.quiet_zone as usize;
        assert!(svg.contains(&format!("viewBox=\"0 0 {total} {total}\"")));
    }

    #[test]
    fn png_has_the_expected_dimensions() {
        let qr = Qr::encode("https://example.com", Ecc::M).unwrap();
        let style = Style {
            scale: 10,
            quiet_zone: 4,
            ..Default::default()
        };
        let bytes = to_png(&qr, &style, None).unwrap();
        let expected = (qr.size() as u32 + 8) * 10;
        // A PNG IHDR carries width and height as big-endian u32 at bytes 16..24.
        let w = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
        let h = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
        assert_eq!((w, h), (expected, expected));
    }

    /// A solid square of the given size, as PNG bytes.
    fn test_logo(px: u32) -> Logo {
        let img = image::RgbaImage::from_pixel(px, px, image::Rgba([220, 40, 40, 255]));
        let mut bytes = std::io::Cursor::new(Vec::new());
        img.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        Logo::load(&bytes.into_inner()).unwrap()
    }

    #[test]
    fn a_logo_is_embedded_in_the_svg_rather_than_linked() {
        let qr = Qr::encode("https://example.com", Ecc::H).unwrap();
        let logo = test_logo(64);
        let svg = to_svg(&qr, &Style::default(), Some(&logo));
        assert!(svg.contains("<image"), "logo must be drawn");
        assert!(
            svg.contains("href=\"data:image/png;base64,"),
            "the SVG has to stand alone, with no external reference"
        );
        assert!(
            svg.contains("preserveAspectRatio=\"xMidYMid meet\""),
            "a non-square logo must not be stretched"
        );
    }

    #[test]
    fn no_logo_means_no_image_element() {
        let qr = Qr::encode("https://example.com", Ecc::M).unwrap();
        assert!(!to_svg(&qr, &Style::default(), None).contains("<image"));
    }

    #[test]
    fn a_logo_actually_covers_pixels_in_the_raster() {
        let qr = Qr::encode("https://example.com", Ecc::H).unwrap();
        let style = Style::default();
        let plain = to_luma(&qr, &style, None);
        let with = to_luma(&qr, &style, Some(&test_logo(64)));
        assert_eq!(plain.pixels.len(), with.pixels.len());
        let changed = plain
            .pixels
            .iter()
            .zip(&with.pixels)
            .filter(|(a, b)| a != b)
            .count();
        assert!(changed > 0, "the logo must reach the verification raster");
    }

    #[test]
    fn rejects_a_file_that_is_not_an_image() {
        assert!(matches!(
            Logo::load(b"this is not an image"),
            Err(RenderError::Logo(_))
        ));
    }

    #[test]
    fn logo_fraction_is_clamped_to_a_recoverable_range() {
        let qr = Qr::encode("https://example.com", Ecc::H).unwrap();
        for requested in [-5.0, 0.0, 0.9, 100.0] {
            let s = Style {
                logo_fraction: requested,
                ..Default::default()
            }
            .sanitized(qr.size());
            assert!(
                (0.05..=0.35).contains(&s.logo_fraction),
                "{requested} became {}",
                s.logo_fraction
            );
        }
    }

    #[test]
    fn absurd_scale_is_clamped_instead_of_allocating() {
        let qr = Qr::encode("https://example.com", Ecc::M).unwrap();
        let style = Style {
            scale: u32::MAX,
            ..Default::default()
        }
        .sanitized(qr.size());
        let px = (qr.size() as u32 + 2 * style.quiet_zone) * style.scale;
        assert!(px <= MAX_PIXELS_PER_SIDE, "clamped to {px}px");
        assert!(style.scale >= 1);
    }
}
