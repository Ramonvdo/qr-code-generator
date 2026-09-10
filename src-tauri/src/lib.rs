//! Command surface. Everything here is thin: parse, delegate, serialise.
//!
//! The logic lives in `payload`, `render`, `verify` and `export`, none of
//! which know Tauri exists. That is what keeps the test suite fast and lets
//! the interesting parts be exercised without a webview.

mod b64;
mod batch;
mod buyers;
mod checkin;
mod decode;
mod email;
mod events;
mod export;
mod fonts;
mod payload;
mod pdf;
mod render;
mod storage;
mod tickets;
mod vars;
mod verify;

use email::Transport as _;
use payload::{Kind, Payload};
use render::{Ecc, Logo, Qr, RenderError, Rgb, Style};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{Emitter, Manager};
use tauri_plugin_global_shortcut::GlobalShortcutExt;

/// Refuse to load a logo larger than this.
///
/// The file is base64'd into every exported SVG and into every preview that
/// crosses the IPC boundary, so a 40MB photograph would make the app crawl
/// rather than fail.
const MAX_LOGO_BYTES: u64 = 4 * 1024 * 1024;

/// The decoded logo, cached so it survives a keystroke.
///
/// Without this, every character typed would re-read the file, re-decode it
/// and re-base64 it. Decoding is the expensive half of a render and the file
/// almost never changes, so it is cached by path and rebuilt only on request.
#[derive(Default)]
struct LogoState(Mutex<Option<Logo>>);

/// Everything needed to produce one QR code.
///
/// Sent whole on every keystroke rather than kept as server-side state, so
/// there is no way for the preview and the export to disagree about options.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderRequest {
    payload: Payload,
    ecc: Ecc,
    scale: u32,
    quiet_zone: u32,
    /// Hex strings straight from the webview. Parsed, never trusted.
    dark: String,
    light: String,
    /// `None` means no logo. `Some(fraction)` draws the currently loaded one
    /// at that fraction of the code's width.
    #[serde(default)]
    logo_fraction: Option<f32>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    /// The exact document that gets written on save, so what is on screen and
    /// what lands on disk cannot drift apart.
    svg: String,
    /// What the code actually contains, shown in the UI so an auto-detected
    /// scheme is never a surprise.
    encoded: String,
    modules: usize,
    size_px: u32,
    verdict: verify::Verdict,
}

/// A batch run: the same options as a single render, plus where the list and
/// the files go. The base request supplies the template payload, so a batch of
/// links normalises exactly the way one link does.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchRequest {
    #[serde(flatten)]
    base: RenderRequest,
    text: String,
    directory: String,
    format: Format,
    naming: batch::Naming,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogoInfo {
    name: String,
    width: u32,
    height: u32,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Svg,
    Png,
}

impl Format {
    fn ext(self) -> &'static str {
        match self {
            Format::Svg => "svg",
            Format::Png => "png",
        }
    }
}

/// Resolve a request into everything the renderers need, then hand it to `f`.
///
/// One funnel, so the preview, the export and the clipboard cannot end up
/// interpreting the same request differently. The logo is borrowed from the
/// cache for the duration of the call rather than cloned.
fn with_render<R>(
    req: &RenderRequest,
    logos: &LogoState,
    f: impl FnOnce(&str, &Qr, &Style, Option<&Logo>) -> R,
) -> Result<R, RenderError> {
    let guard = logos.0.lock().unwrap();
    let logo = req.logo_fraction.and(guard.as_ref());

    let encoded = req.payload.encode();
    // A logo destroys modules, and only the highest error correction level
    // reliably recovers from that. Forcing it here as well as in the UI means
    // a stale request cannot slip a logo onto a level that cannot carry it.
    let ecc = if logo.is_some() { Ecc::H } else { req.ecc };
    let qr = Qr::encode(&encoded, ecc)?;

    let style = Style {
        scale: req.scale,
        quiet_zone: req.quiet_zone,
        // A malformed colour falls back rather than failing the render: the
        // user is mid-edit in a colour field, not misusing the app.
        dark: Rgb::from_hex(&req.dark).unwrap_or(Rgb::BLACK),
        light: Rgb::from_hex(&req.light).unwrap_or(Rgb::WHITE),
        logo_fraction: req.logo_fraction.unwrap_or(0.20),
    }
    .sanitized(qr.size());

    Ok(f(&encoded, &qr, &style, logo))
}

/// Guess whether the input is a link or plain text.
///
/// Exposed as a command so the frontend does not carry a second, drifting
/// copy of the rules.
#[tauri::command]
fn detect_kind(input: String) -> Kind {
    payload::detect(&input)
}

#[tauri::command]
fn preview(req: RenderRequest, logos: tauri::State<LogoState>) -> Result<Preview, String> {
    with_render(&req, &logos, |encoded, qr, style, logo| Preview {
        svg: render::to_svg(qr, style, logo),
        encoded: encoded.to_string(),
        modules: qr.size(),
        size_px: (qr.size() as u32 + 2 * style.quiet_zone) * style.scale,
        verdict: verify::check(qr, style, logo, encoded),
    })
    .map_err(|e| e.to_string())
}

/// A filename stem for the save dialog to open with.
#[tauri::command]
fn suggest_filename(req: RenderRequest) -> String {
    export::suggested_stem(&req.payload.encode())
}

/// Render one code to the bytes of the chosen format, with its payload.
fn render_bytes(
    req: &RenderRequest,
    logos: &LogoState,
    format: Format,
) -> Result<(String, Vec<u8>), RenderError> {
    with_render(req, logos, |encoded, qr, style, logo| match format {
        Format::Svg => Ok((
            encoded.to_string(),
            render::to_svg(qr, style, logo).into_bytes(),
        )),
        Format::Png => render::to_png(qr, style, logo).map(|b| (encoded.to_string(), b)),
    })?
}

/// Render and write in one call.
///
/// The bytes never cross the IPC boundary, which keeps a 4000px PNG export off
/// the JSON path entirely.
#[tauri::command]
fn save(
    req: RenderRequest,
    format: Format,
    path: String,
    logos: tauri::State<LogoState>,
) -> Result<String, String> {
    let (_, bytes) = render_bytes(&req, &logos, format).map_err(|e| e.to_string())?;

    let mut path = PathBuf::from(path);
    // A save dialog can hand back a path with no extension, or the wrong one
    // if the user switched format after the dialog opened.
    if path.extension().and_then(|e| e.to_str()) != Some(format.ext()) {
        path.set_extension(format.ext());
    }
    export::write_atomic(&path, &bytes).map_err(|e| format!("Could not save: {e}"))?;
    Ok(path.to_string_lossy().into_owned())
}

/// PNG bytes for the clipboard.
///
/// Returned as a raw binary response so it arrives in JS as an ArrayBuffer.
/// Serialising a megapixel PNG as a JSON array of numbers would be roughly an
/// order of magnitude larger and slower.
#[tauri::command]
fn png_bytes(
    req: RenderRequest,
    logos: tauri::State<LogoState>,
) -> Result<tauri::ipc::Response, String> {
    let (_, bytes) = render_bytes(&req, &logos, Format::Png).map_err(|e| e.to_string())?;
    Ok(tauri::ipc::Response::new(bytes))
}

/// Write one file per pasted line.
///
/// Individual failures are collected rather than aborting the run: a thousand
/// good rows should not be lost because row 412 is too long to encode. Every
/// failure comes back with its line number so the user can fix exactly those.
#[tauri::command]
fn batch_export(
    req: BatchRequest,
    logos: tauri::State<LogoState>,
) -> Result<batch::Outcome, String> {
    let items = batch::parse_lines(&req.text);
    if items.is_empty() {
        return Err("Nothing to export. Paste one entry per line.".into());
    }
    if items.len() > batch::MAX_ITEMS {
        return Err(format!(
            "That is {} entries. Split it into runs of {} or fewer.",
            items.len(),
            batch::MAX_ITEMS
        ));
    }
    if req.base.payload.with_input(String::new()).is_none() {
        return Err("Batch export works from the Link or Text tab.".into());
    }

    let dir = PathBuf::from(&req.directory);
    let width = batch::number_width(items.len());
    let ext = req.format.ext();
    let mut written = 0usize;
    let mut failures = Vec::new();

    for (index, (line, content)) in items.iter().enumerate() {
        let payload = match req.base.payload.with_input(content.clone()) {
            Some(p) => p,
            None => continue,
        };
        let item = RenderRequest {
            payload,
            ..req.base.clone()
        };

        match render_bytes(&item, &logos, req.format) {
            Ok((encoded, bytes)) => {
                let path = match req.naming {
                    batch::Naming::Numbered => {
                        dir.join(format!("{:0width$}.{ext}", index + 1, width = width))
                    }
                    batch::Naming::Content => {
                        export::unique_path(&dir, &export::suggested_stem(&encoded), ext)
                    }
                };
                match export::write_atomic(&path, &bytes) {
                    Ok(()) => written += 1,
                    Err(e) => failures.push(batch::Failure {
                        line: *line,
                        content: content.clone(),
                        reason: format!("Could not write the file: {e}"),
                    }),
                }
            }
            Err(e) => failures.push(batch::Failure {
                line: *line,
                content: content.clone(),
                reason: e.to_string(),
            }),
        }
    }

    Ok(batch::Outcome {
        written,
        failures,
        directory: dir.to_string_lossy().into_owned(),
    })
}

/// Read and decode a logo, replacing whatever was loaded before.
#[tauri::command]
fn load_logo(path: String, logos: tauri::State<LogoState>) -> Result<LogoInfo, String> {
    let path = PathBuf::from(path);
    let size = std::fs::metadata(&path)
        .map_err(|e| format!("Could not open that file: {e}"))?
        .len();
    if size > MAX_LOGO_BYTES {
        return Err(format!(
            "That image is {:.1} MB. Use one under {} MB.",
            size as f64 / 1_048_576.0,
            MAX_LOGO_BYTES / 1_048_576
        ));
    }

    let bytes = std::fs::read(&path).map_err(|e| format!("Could not read that file: {e}"))?;
    let logo = Logo::load(&bytes).map_err(|e| e.to_string())?;
    let (width, height) = logo.dimensions();
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();

    *logos.0.lock().unwrap() = Some(logo);
    Ok(LogoInfo {
        name,
        width,
        height,
    })
}

#[tauri::command]
fn clear_logo(logos: tauri::State<LogoState>) {
    *logos.0.lock().unwrap() = None;
}

/// A4 in points, the default when no template is supplied.
const A4: (f32, f32) = (595.276, 841.89);

/// Modules of light margin around a code stamped onto a page.
///
/// The QR spec calls for four. On a ticket it matters more than usual: the
/// code lands on artwork rather than blank paper, and without its own margin
/// the surrounding design runs straight into the finder patterns.
const TICKET_QUIET_ZONE: usize = 4;

/// The events directory, resolved once at startup.
struct Events(events::Store);

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateInfo {
    name: String,
    width_pt: f32,
    height_pt: f32,
    pages: usize,
}

/// Validate a template and report its page size.
///
/// Run when the file is chosen rather than at export time, so a rotated or
/// unreadable template is rejected while the user is still looking at the file
/// picker, not after they have positioned everything.
#[tauri::command]
fn load_template(path: String) -> Result<TemplateInfo, String> {
    let path = PathBuf::from(path);
    let bytes = std::fs::read(&path).map_err(|e| format!("Could not read that file: {e}"))?;
    let template = pdf::Template::load(&bytes).map_err(|e| e.to_string())?;
    let (width_pt, height_pt) = template.page_size();
    Ok(TemplateInfo {
        pages: template.page_count(),
        name: path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        width_pt,
        height_pt,
    })
}

/// The raw PDF, for pdf.js to draw in the positioning stage.
///
/// The webview has no filesystem access by design, so the bytes have to come
/// through here rather than being opened directly.
#[tauri::command]
fn template_bytes(path: String) -> Result<tauri::ipc::Response, String> {
    let bytes = std::fs::read(&path).map_err(|e| format!("Could not read that file: {e}"))?;
    Ok(tauri::ipc::Response::new(bytes))
}

// ---------------------------------------------------------------- events --

#[tauri::command]
fn list_events(events: tauri::State<Events>) -> Vec<events::Summary> {
    events.0.list()
}

#[tauri::command]
fn load_event(events: tauri::State<Events>, id: String) -> Result<events::Event, String> {
    let (event, warning) = events.0.load(&id);
    event.ok_or_else(|| warning.unwrap_or_else(|| format!("No event called \"{id}\".")))
}

/// Create an event and its signing key.
///
/// The id is derived from the name once and then never changes, even if the
/// event is renamed later. The key, the ledger and the redemption log are all
/// filed under it, and moving them because someone fixed a typo in the title
/// is how tickets stop verifying.
#[tauri::command]
fn create_event(events: tauri::State<Events>, name: String) -> Result<events::Event, String> {
    if name.trim().is_empty() {
        return Err("Give the event a name.".into());
    }
    let mut event = events::Event::new(&name);

    // Two events can legitimately share a name across years, and they must not
    // share a key.
    let base = event.id.clone();
    let mut n = 2;
    while events.0.dir(&event.id).exists() {
        event.id = format!("{base}-{n}");
        n += 1;
    }

    events.0.key(&event.id)?;
    events.0.save(&event)?;
    Ok(event)
}

/// Persist an event's settings. The id is never taken from the caller's copy.
#[tauri::command]
fn save_event(events: tauri::State<Events>, event: events::Event) -> Result<events::Event, String> {
    let (existing, _) = events.0.load(&event.id);
    let existing = existing.ok_or("That event no longer exists.")?;
    // Mutated rather than rebuilt with struct-update syntax, because the
    // legacy migration field is private and must stay that way.
    let mut saved = event;
    saved.id = existing.id;
    saved.created_at = existing.created_at;
    // Serial allocation belongs to issuing, not to the settings form, or a
    // stale window could rewind it and reissue serials already in the wild.
    saved.next_serial = existing.next_serial;
    // Likewise the remembered columns, which only an issue run updates.
    if saved.columns.is_empty() {
        saved.columns = existing.columns;
    }
    events.0.save(&saved)?;
    Ok(saved)
}

#[tauri::command]
fn event_tickets(events: tauri::State<Events>, id: String) -> Vec<events::Ticket> {
    events.0.tickets(&id).0
}

#[tauri::command]
fn event_public_key(events: tauri::State<Events>, id: String) -> Result<String, String> {
    Ok(events.0.key(&id)?.public_key())
}

// ---------------------------------------------------------------- issuing --

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueRequest {
    event_id: String,
    /// Who to issue for. Empty means a plain numbered run of `quantity`.
    #[serde(default)]
    rows: Vec<buyers::Row>,
    /// The imported columns, in file order, so the manifest can carry them.
    #[serde(default)]
    columns: Vec<String>,
    /// Used only when there are no rows.
    #[serde(default)]
    quantity: u32,
    directory: String,
    /// Also write every ticket into one file, for printing a roll.
    #[serde(default)]
    combined: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueOutcome {
    count: usize,
    directory: String,
    tickets_dir: String,
    manifest_path: String,
    key_path: String,
    combined_path: Option<String>,
    public_key: String,
    /// One payload, so the UI can show what a scanner will actually read.
    sample: String,
    first_serial: u32,
    last_serial: u32,
}

/// Issue a run: one PDF per ticket, a manifest, and the public key.
///
/// Per ticket rather than one combined file, because the combined file cannot
/// be emailed to anybody. The combined version is still written on request,
/// since printing a roll at a venue is a real use.
#[tauri::command]
fn issue(events: tauri::State<Events>, req: IssueRequest) -> Result<IssueOutcome, String> {
    let (event, warning) = events.0.load(&req.event_id);
    let mut event =
        event.ok_or_else(|| warning.unwrap_or_else(|| "That event no longer exists.".into()))?;

    // Expand rows into one entry per ticket. A buyer who bought three gets
    // three separate tickets, each its own file, because they will be handed
    // to three different people.
    let entries: Vec<(std::collections::BTreeMap<String, String>, String)> = if req.rows.is_empty()
    {
        (0..req.quantity)
            .map(|_| (std::collections::BTreeMap::new(), String::new()))
            .collect()
    } else {
        req.rows
            .iter()
            .flat_map(|r| {
                let mut fields = r.fields.clone();
                fields.insert("quantity".into(), r.quantity.to_string());
                std::iter::repeat((fields, r.email.clone())).take(r.quantity as usize)
            })
            .collect()
    };

    if entries.is_empty() {
        return Err("Nothing to issue. Set a quantity or import a buyer list.".into());
    }
    if entries.len() > batch::MAX_ITEMS {
        return Err(format!(
            "That is {} tickets. Issue them in runs of {} or fewer.",
            entries.len(),
            batch::MAX_ITEMS
        ));
    }

    let key = events.0.key(&event.id)?;
    let template = match &event.template {
        Some(t) => {
            let bytes = std::fs::read(&t.path).map_err(|e| {
                format!(
                    "Could not read the template at {}: {e}. Choose it again if it moved.",
                    t.path
                )
            })?;
            pdf::Template::load(&bytes)
                .and_then(|loaded| loaded.stamp_on(t.page))
                .map_err(|e| e.to_string())?
        }
        None => pdf::Template::blank(A4.0, A4.1),
    };

    let (first_serial, last_serial) = event.allocate(entries.len() as u32);

    // Everything is signed and encoded before anything is written, so a
    // payload that will not fit fails the run before it has scattered half a
    // folder of files.
    let mut records = Vec::with_capacity(entries.len());
    let mut codes = Vec::with_capacity(entries.len());
    for (i, (fields, email)) in entries.into_iter().enumerate() {
        let serial = first_serial + i as u32;
        let payload = key.issue(&event.name, serial);
        codes.push(Qr::encode(&payload, event.ecc).map_err(|e| e.to_string())?);
        records.push(events::Ticket::new(serial, payload, fields, email));
    }

    let dir = PathBuf::from(&req.directory);
    let tickets_dir = dir.join("tickets");
    std::fs::create_dir_all(&tickets_dir)
        .map_err(|e| format!("Could not create {tickets_dir:?}: {e}"))?;

    let dark = Rgb::from_hex(&event.dark).unwrap_or(Rgb::BLACK);
    let light = Rgb::from_hex(&event.light).unwrap_or(Rgb::WHITE);

    for (ticket, qr) in records.iter_mut().zip(&codes) {
        let vars = ticket.vars(&event.name);
        let bytes = template
            .stamp(
                &[pdf::Stamp {
                    qr,
                    vars: vars.clone(),
                }],
                &event.layout,
                TICKET_QUIET_ZONE,
                dark,
                light,
            )
            .map_err(|e| e.to_string())?;

        // The filename is a template like everything else, so a run can be
        // filed by surname or seat rather than only by number.
        let stem = export::sanitize_stem(&vars.render(&event.filename));
        let path = export::unique_path(&tickets_dir, &stem, "pdf");
        export::write_atomic(&path, &bytes).map_err(|e| format!("Could not save: {e}"))?;
        ticket.file = path.to_string_lossy().into_owned();
    }

    let combined_path = if req.combined {
        let stamps: Vec<pdf::Stamp> = records
            .iter()
            .zip(&codes)
            .map(|(ticket, qr)| pdf::Stamp {
                qr,
                vars: ticket.vars(&event.name),
            })
            .collect();
        let bytes = template
            .stamp(&stamps, &event.layout, TICKET_QUIET_ZONE, dark, light)
            .map_err(|e| e.to_string())?;
        let path = dir.join(format!("{}-all-tickets.pdf", event.id));
        export::write_atomic(&path, &bytes).map_err(|e| format!("Could not save: {e}"))?;
        Some(path.to_string_lossy().into_owned())
    } else {
        None
    };

    // The ledger is appended to, not replaced, so a second run keeps the first.
    let (mut ledger, _) = events.0.tickets(&event.id);
    ledger.extend(records.iter().cloned());
    events.0.save_tickets(&event.id, &ledger)?;

    // Columns are remembered so a manifest rewritten after sending, when the
    // import is long gone, still carries the same columns in the same order.
    if !req.columns.is_empty() {
        event.columns = req.columns.clone();
    }
    events.0.save(&event)?;

    let manifest_path = dir.join(format!("{}-tickets.csv", event.id));
    export::write_atomic(
        &manifest_path,
        events::manifest_csv(&event.name, &event.columns, &records).as_bytes(),
    )
    .map_err(|e| format!("Could not save: {e}"))?;

    let key_path = dir.join(format!("{}-public.key", event.id));
    export::write_atomic(
        &key_path,
        format!(
            "# Public key for \"{}\". Give this to whoever checks tickets.\n\
             # A ticket is genuine when its signature verifies against this key.\n\
             # This file contains no secret: it cannot be used to make tickets.\n\
             {}\n",
            event.name,
            key.public_key()
        )
        .as_bytes(),
    )
    .map_err(|e| format!("Could not save: {e}"))?;

    Ok(IssueOutcome {
        count: records.len(),
        directory: dir.to_string_lossy().into_owned(),
        tickets_dir: tickets_dir.to_string_lossy().into_owned(),
        manifest_path: manifest_path.to_string_lossy().into_owned(),
        key_path: key_path.to_string_lossy().into_owned(),
        combined_path,
        public_key: key.public_key(),
        sample: records
            .first()
            .map(|r| r.payload.clone())
            .unwrap_or_default(),
        first_serial,
        last_serial,
    })
}

// ----------------------------------------------------------------- door --

/// Check one scanned ticket against an event, recording the admission.
///
/// The record is written before the answer is returned. Admitting someone and
/// then failing to save would let the same ticket through again, which is the
/// one failure this whole feature exists to prevent.
#[tauri::command]
fn check_ticket(
    events: tauri::State<Events>,
    event_id: String,
    scanned: String,
) -> Result<checkin::Outcome, String> {
    let public_key = events.0.key(&event_id)?.public_key();

    // Every other event this machine knows about, so a ticket for the wrong
    // night can be named instead of being called a forgery.
    let others: Vec<checkin::OtherEvent> = events
        .0
        .list()
        .into_iter()
        .filter(|e| e.id != event_id)
        .filter_map(|e| {
            events.0.key(&e.id).ok().map(|k| checkin::OtherEvent {
                name: e.name,
                public_key: k.public_key(),
            })
        })
        .collect();

    let (tickets, _) = events.0.tickets(&event_id);
    let (mut log, warning) = events.0.redemptions(&event_id);
    if let Some(warning) = warning {
        // A lost log means previously admitted tickets would be let through
        // again. Refuse rather than quietly reopen the door.
        return Err(format!(
            "{warning} Checking is paused until that is resolved."
        ));
    }

    let before = log.len();
    let outcome = checkin::check(&public_key, &others, &mut log, &tickets, &scanned);
    if log.len() != before {
        events.0.save_redemptions(&event_id, &log)?;
    }
    Ok(outcome)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DoorState {
    redeemed: usize,
    issued: usize,
    public_key: String,
}

#[tauri::command]
fn door_state(events: tauri::State<Events>, event_id: String) -> Result<DoorState, String> {
    Ok(DoorState {
        redeemed: events.0.redemptions(&event_id).0.len(),
        issued: events.0.tickets(&event_id).0.len(),
        public_key: events.0.key(&event_id)?.public_key(),
    })
}

/// Write the redemption log, so two doors can be reconciled afterwards.
#[tauri::command]
fn export_redemptions(
    events: tauri::State<Events>,
    event_id: String,
    path: String,
) -> Result<String, String> {
    let (log, _) = events.0.redemptions(&event_id);
    let mut path = PathBuf::from(path);
    if path.extension().and_then(|e| e.to_str()) != Some("csv") {
        path.set_extension("csv");
    }
    export::write_atomic(&path, events::redemption_csv(&log).as_bytes())
        .map_err(|e| format!("Could not save: {e}"))?;
    Ok(path.to_string_lossy().into_owned())
}

// ------------------------------------------------------------------ send --

/// Pause between messages.
///
/// Resend's default allowance is two requests a second. Going flat out gets a
/// run rate limited part way through, which turns one failed send into a
/// hundred, so the gap is built in rather than left to be discovered.
const SEND_GAP: std::time::Duration = std::time::Duration::from_millis(600);

/// Email settings, held in memory and mirrored to disk.
struct SettingsState {
    path: PathBuf,
    config: Mutex<email::Config>,
}

/// Raised to ask an in-flight run to stop.
///
/// An `Arc` so the flag itself can be cloned into the sending thread. The
/// thread then reads the same bool the command writes, with no shared lock a
/// stuck send could hold.
#[derive(Default)]
struct SendCancel(std::sync::Arc<std::sync::atomic::AtomicBool>);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SendProgress {
    sent: usize,
    failed: usize,
    done: usize,
    total: usize,
    /// Who the run is on, so the UI shows movement rather than a spinner.
    current: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SendReport {
    sent: usize,
    failed: usize,
    /// Every failure with its address and reason, in full. A run that says
    /// "14 failed" without saying which ones is not actionable.
    failures: Vec<SendFailure>,
    cancelled: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SendFailure {
    serial: u32,
    email: String,
    reason: String,
}

#[tauri::command]
fn load_settings(settings: tauri::State<SettingsState>) -> email::Config {
    settings.config.lock().unwrap().clone()
}

#[tauri::command]
fn save_settings(
    settings: tauri::State<SettingsState>,
    config: email::Config,
) -> Result<(), String> {
    storage::save_json(&settings.path, &config)?;
    *settings.config.lock().unwrap() = config;
    Ok(())
}

/// Send one message to an address of the operator's choosing.
///
/// Exists because a wrong from-address, or a domain that was never verified
/// with Resend, fails every message in a run identically. Finding that out
/// once is the difference between a five second fix and five hundred failures.
#[tauri::command]
fn send_test(settings: tauri::State<SettingsState>, to: String) -> Result<(), String> {
    let config = settings.config.lock().unwrap().clone();
    email::Resend
        .send(
            &config,
            &email::Message {
                to,
                subject: "Test from QR Code Generator".into(),
                body: "If you are reading this, sending is configured correctly.".into(),
                attachment: None,
            },
        )
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn cancel_send(cancel: tauri::State<SendCancel>) {
    cancel.0.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// Email every ticket that has not gone out yet.
///
/// Runs on its own thread and reports progress by event, because five hundred
/// messages at the required pace is five minutes of work and blocking the
/// window for that would look like a crash.
///
/// Each ticket's delivery status is written as it goes, so a run that stops
/// halfway resumes by sending only what is still pending rather than mailing
/// three hundred people a second copy.
#[tauri::command]
fn send_tickets(
    app: tauri::AppHandle,
    events: tauri::State<Events>,
    settings: tauri::State<SettingsState>,
    cancel: tauri::State<SendCancel>,
    event_id: String,
    retry_failed: bool,
) -> Result<usize, String> {
    let config = settings.config.lock().unwrap().clone();
    if !config.is_ready() {
        return Err("Set the Resend API key and a from address in Settings first.".into());
    }

    let (event, _) = events.0.load(&event_id);
    let event = event.ok_or("That event no longer exists.")?;
    let (tickets, warning) = events.0.tickets(&event_id);
    if let Some(warning) = warning {
        return Err(format!(
            "{warning} Sending is paused until that is resolved."
        ));
    }

    let queued: Vec<usize> = tickets
        .iter()
        .enumerate()
        .filter(|(_, t)| match &t.delivery {
            events::Delivery::Pending => true,
            events::Delivery::Failed { .. } => retry_failed,
            // Already delivered, or nobody to deliver to.
            events::Delivery::Sent { .. } | events::Delivery::NoAddress => false,
        })
        .map(|(i, _)| i)
        .collect();

    if queued.is_empty() {
        return Err("Nothing to send. Every ticket with an address has already gone out.".into());
    }

    cancel.0.store(false, std::sync::atomic::Ordering::Relaxed);
    let flag = cancel.0.clone();
    let store = events.0.clone();
    let total = queued.len();
    let event_name = event.name.clone();
    let template = event.email.clone();
    let columns = event.columns.clone();

    std::thread::spawn(move || {
        let mut tickets = tickets;
        let mut report = SendReport {
            sent: 0,
            failed: 0,
            failures: Vec::new(),
            cancelled: false,
        };

        for (done, index) in queued.into_iter().enumerate() {
            if flag.load(std::sync::atomic::Ordering::Relaxed) {
                report.cancelled = true;
                break;
            }

            let ticket = tickets[index].clone();
            // Every column this ticket was issued with, so the copy can name a
            // seat or a tier without this loop knowing they exist.
            let mut message_vars = ticket.vars(&event_name);
            message_vars.set("name", ticket.display_name());
            let _ = app.emit(
                "send-progress",
                SendProgress {
                    sent: report.sent,
                    failed: report.failed,
                    done,
                    total,
                    current: ticket.email.clone(),
                },
            );

            let attachment = std::fs::read(&ticket.file)
                .map(|bytes| {
                    let name = PathBuf::from(&ticket.file)
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| format!("ticket-{}.pdf", ticket.serial));
                    (name, bytes)
                })
                .ok();

            let outcome = match attachment {
                None => Err(format!(
                    "The ticket file is missing: {}. Issue the run again, or move the folder back.",
                    ticket.file
                )),
                Some(attachment) => email::Resend
                    .send(
                        &config,
                        &email::Message {
                            to: ticket.email.clone(),
                            subject: email::render(&template.subject, &message_vars),
                            body: email::render(&template.body, &message_vars),
                            attachment: Some(attachment),
                        },
                    )
                    .map_err(|e| e.to_string()),
            };

            tickets[index].delivery = match &outcome {
                Ok(()) => {
                    report.sent += 1;
                    events::Delivery::Sent { at: events::now() }
                }
                Err(error) => {
                    report.failed += 1;
                    report.failures.push(SendFailure {
                        serial: ticket.serial,
                        email: ticket.email.clone(),
                        reason: error.clone(),
                    });
                    events::Delivery::Failed {
                        at: events::now(),
                        error: error.clone(),
                    }
                }
            };

            // Written after every message rather than at the end. A crash or a
            // pulled power cable mid-run must not cost the record of who has
            // already been emailed.
            let _ = store.save_tickets(&event_id, &tickets);

            // The manifest beside the tickets is refreshed too, so the
            // ticket_sent column is current at any moment rather than only
            // once the run finishes.
            if let Some(folder) = PathBuf::from(&ticket.file)
                .parent()
                .and_then(|p| p.parent())
            {
                let _ = export::write_atomic(
                    &folder.join(format!("{event_id}-tickets.csv")),
                    events::manifest_csv(&event_name, &columns, &tickets).as_bytes(),
                );
            }
            std::thread::sleep(SEND_GAP);
        }

        let _ = app.emit("send-done", report);
    });

    Ok(total)
}

/// Copy an event for another run of the same thing.
///
/// The duplicate gets its own signing key, created on first use under its new
/// id. Sharing one would mean last year's tickets opening this year's door.
#[tauri::command]
fn duplicate_event(
    events: tauri::State<Events>,
    id: String,
    name: String,
) -> Result<events::Event, String> {
    if name.trim().is_empty() {
        return Err("Give the new event a name.".into());
    }
    let (original, warning) = events.0.load(&id);
    let original =
        original.ok_or_else(|| warning.unwrap_or_else(|| "That event no longer exists.".into()))?;

    let mut copy = original.duplicated(&name);
    let base = copy.id.clone();
    let mut n = 2;
    while events.0.dir(&copy.id).exists() {
        copy.id = format!("{base}-{n}");
        n += 1;
    }

    events.0.key(&copy.id)?;
    events.0.save(&copy)?;
    Ok(copy)
}

/// The CSV columns this event's design and copy refer to.
#[tauri::command]
fn required_columns(events: tauri::State<Events>, id: String) -> Vec<String> {
    events
        .0
        .load(&id)
        .0
        .map(|e| e.required_columns())
        .unwrap_or_default()
}

/// Write an example CSV with exactly the columns this event needs.
///
/// Generated rather than fixed, because guessing a column name is the most
/// common way a first run goes wrong.
#[tauri::command]
fn write_example_csv(
    events: tauri::State<Events>,
    id: String,
    path: String,
) -> Result<String, String> {
    let (event, _) = events.0.load(&id);
    let event = event.ok_or("That event no longer exists.")?;

    let mut columns = event.required_columns();
    // An address column is always needed to send anything, even when no
    // template happens to mention it.
    if !columns.iter().any(|c| c.contains("email")) {
        columns.push("email".into());
    }

    let mut path = PathBuf::from(path);
    if path.extension().and_then(|e| e.to_str()) != Some("csv") {
        path.set_extension("csv");
    }
    export::write_atomic(&path, buyers::example_csv(&columns).as_bytes())
        .map_err(|e| format!("Could not save: {e}"))?;
    Ok(path.to_string_lossy().into_owned())
}

// ---------------------------------------------------------------- buyers --

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuyerFile {
    path: String,
    name: String,
    headers: Vec<String>,
    mapping: buyers::Mapping,
    import: buyers::Import,
}

/// Read a buyer CSV and guess its columns.
#[tauri::command]
fn load_buyers(path: String) -> Result<BuyerFile, String> {
    let text = read_text_file(&path)?;
    let headers = buyers::headers(&text)?;
    let mapping = buyers::guess_mapping(&headers);
    let import = buyers::parse(&text, mapping);
    Ok(BuyerFile {
        name: PathBuf::from(&path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        path,
        headers,
        mapping,
        import,
    })
}

/// Re-read the same file under a mapping the user corrected.
#[tauri::command]
fn remap_buyers(path: String, mapping: buyers::Mapping) -> Result<buyers::Import, String> {
    Ok(buyers::parse(&read_text_file(&path)?, mapping))
}

/// Read a text file, tolerating the encodings spreadsheets actually emit.
///
/// Excel still writes UTF-16 with a BOM from some export paths, and a lossy
/// UTF-8 read of that produces a file of interleaved null characters that
/// parses into nonsense rather than failing.
fn read_text_file(path: &str) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("Could not read that file: {e}"))?;
    if bytes.starts_with(&[0xff, 0xfe]) || bytes.starts_with(&[0xfe, 0xff]) {
        return Err(
            "That file is UTF-16. Re-save it as CSV UTF-8 from your spreadsheet and try again."
                .into(),
        );
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Check a ticket the way a door scanner would.
///
/// Present so the claim that tickets verify offline is something the user can
/// test in the app rather than take on faith.
#[tauri::command]
fn verify_ticket(public_key: String, payload: String) -> Result<tickets::Claim, String> {
    tickets::verify(&public_key, &payload).map_err(|e| e.to_string())
}

/// Read whatever codes are in an image file.
#[tauri::command]
fn decode_image(path: String) -> Result<Vec<String>, String> {
    let bytes = std::fs::read(&path).map_err(|e| format!("Could not read that file: {e}"))?;
    decode::decode_bytes(&bytes).map_err(|e| e.to_string())
}

/// The shortcut that turns whatever is on the clipboard into a code.
///
/// The whole point of the app is the paste-and-go path, and this removes the
/// last two steps from it: the window comes forward with the code already
/// rendered, from anywhere.
const CLIPBOARD_SHORTCUT: &str = "CmdOrCtrl+Alt+Q";

/// Bring the window forward carrying the clipboard's contents.
///
/// DEADLOCK GUARD: the global-shortcut plugin holds its internal mutex for the
/// whole duration of a handler, and handlers run inline on the main thread's
/// message pump. Anything that re-enters the plugin, or blocks waiting on the
/// main thread, deadlocks from here. So the work is pushed onto a fresh
/// thread. `run_on_main_thread` is not a substitute: its main-thread fast path
/// runs inline and deadlocks identically.
fn on_clipboard_shortcut(app: &tauri::AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        use tauri_plugin_clipboard_manager::ClipboardExt;
        let text = app.clipboard().read_text().unwrap_or_default();
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.show();
            let _ = window.unminimize();
            let _ = window.set_focus();
        }
        // Sent even when empty, so the window still comes forward and the
        // user can see that there was nothing to paste.
        let _ = app.emit("clipboard-code", text);
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // Registered first so a second launch focuses the window that is
        // already open rather than starting a rival copy that would fail to
        // claim the global shortcut.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state() == tauri_plugin_global_shortcut::ShortcutState::Pressed {
                        on_clipboard_shortcut(app);
                    }
                })
                .build(),
        )
        .setup(|app| {
            app.manage(LogoState::default());

            let root = app
                .path()
                .app_data_dir()
                .map_err(|e| format!("No app data directory: {e}"))?
                .join("events");
            std::fs::create_dir_all(&root).ok();
            let store = events::Store::new(root);
            // Keys written by the previous layout are already signing tickets
            // in the wild. Moving them before anything else runs is what keeps
            // those tickets verifying after an upgrade.
            for warning in store.migrate_legacy_keys() {
                eprintln!("{warning}");
            }
            app.manage(Events(store));
            app.manage(SendCancel::default());

            let settings_path = app
                .path()
                .app_config_dir()
                .map_err(|e| format!("No config directory: {e}"))?
                .join("settings.json");
            let (config, warning) =
                storage::load_json::<email::Config>(&settings_path).or_default();
            if let Some(warning) = warning {
                eprintln!("{warning}");
            }
            app.manage(SettingsState {
                path: settings_path,
                config: Mutex::new(config),
            });
            // A failure here is not fatal: another app may already own the
            // combination, and everything except the shortcut still works.
            if let Err(e) = app
                .global_shortcut()
                .register(CLIPBOARD_SHORTCUT.parse::<tauri_plugin_global_shortcut::Shortcut>()?)
            {
                eprintln!("Could not register {CLIPBOARD_SHORTCUT}: {e}");
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            detect_kind,
            preview,
            suggest_filename,
            save,
            png_bytes,
            load_logo,
            clear_logo,
            batch_export,
            load_template,
            template_bytes,
            list_events,
            load_event,
            create_event,
            save_event,
            duplicate_event,
            required_columns,
            write_example_csv,
            event_tickets,
            event_public_key,
            issue,
            load_buyers,
            remap_buyers,
            check_ticket,
            door_state,
            export_redemptions,
            load_settings,
            save_settings,
            send_test,
            send_tickets,
            cancel_send,
            verify_ticket,
            decode_image
        ])
        .run(tauri::generate_context!())
        .expect("error while running QR Code Generator");
}

/// The IPC contract, pinned against the literal JSON the frontend sends.
///
/// Everything else in this crate is tested through Rust types, which cannot
/// catch the one mistake that actually breaks the app: a field named
/// `quietZone` on one side and `quiet_zone` on the other. That failure only
/// appears at runtime, in a webview, as a silent empty preview.
#[cfg(test)]
mod contract {
    use super::*;

    /// Must stay in step with the object built in `src/lib/api.js`.
    const REQUEST_JSON: &str = r##"{
        "payload": { "kind": "link", "input": "https://example.com" },
        "ecc": "m",
        "scale": 8,
        "quietZone": 4,
        "dark": "#000000",
        "light": "#ffffff",
        "logoFraction": null
    }"##;

    fn parse(json: &str) -> RenderRequest {
        serde_json::from_str(json).unwrap_or_else(|e| panic!("rejected: {e}\n{json}"))
    }

    /// Swap in a different payload object, keeping the rest of the request.
    fn with_payload(payload: &str) -> RenderRequest {
        parse(&REQUEST_JSON.replace(
            r#"{ "kind": "link", "input": "https://example.com" }"#,
            payload,
        ))
    }

    fn preview_of(req: RenderRequest) -> serde_json::Value {
        let logos = LogoState::default();
        let out = with_render(&req, &logos, |encoded, qr, style, logo| Preview {
            svg: render::to_svg(qr, style, logo),
            encoded: encoded.to_string(),
            modules: qr.size(),
            size_px: (qr.size() as u32 + 2 * style.quiet_zone) * style.scale,
            verdict: verify::check(qr, style, logo, encoded),
        })
        .expect("a valid request must render");
        serde_json::to_value(out).unwrap()
    }

    #[test]
    fn the_frontend_request_deserialises() {
        let req = parse(REQUEST_JSON);
        assert_eq!(req.quiet_zone, 4);
        assert_eq!(req.scale, 8);
        assert!(req.logo_fraction.is_none());
        assert_eq!(req.payload.encode(), "https://example.com");
    }

    /// The field is `#[serde(default)]`, so an older or simpler caller that
    /// omits it entirely must still work.
    #[test]
    fn a_request_without_a_logo_field_is_accepted() {
        let json = REQUEST_JSON.replace(r#","logoFraction": null"#, "");
        assert!(parse(&json).logo_fraction.is_none());
    }

    #[test]
    fn every_option_the_ui_offers_is_accepted() {
        for ecc in ["l", "m", "q", "h"] {
            parse(&REQUEST_JSON.replace(r#""ecc": "m""#, &format!(r#""ecc": "{ecc}""#)));
        }
        for format in ["svg", "png"] {
            serde_json::from_str::<Format>(&format!("\"{format}\""))
                .unwrap_or_else(|e| panic!("format {format} rejected: {e}"));
        }
    }

    /// Each mode component sends a differently shaped payload object. A key
    /// renamed on one side shows up here rather than as a dead preview.
    #[test]
    fn every_payload_shape_the_ui_sends_deserialises() {
        assert_eq!(
            with_payload(r#"{ "kind": "text", "input": "hello there" }"#)
                .payload
                .encode(),
            "hello there"
        );
        assert_eq!(
            with_payload(
                r#"{ "kind": "wifi", "ssid": "Cafe", "password": "flatwhite",
                     "security": "wpa", "hidden": false }"#
            )
            .payload
            .encode(),
            "WIFI:T:WPA;S:Cafe;P:flatwhite;;"
        );
        let card = with_payload(
            r#"{ "kind": "contact", "firstName": "Marieke", "lastName": "van Dijk",
                 "org": "", "title": "", "phone": "+31 20 555 0134",
                 "email": "", "url": "" }"#,
        )
        .payload
        .encode();
        assert!(card.starts_with("BEGIN:VCARD"), "got {card}");
        assert!(card.contains("TEL;TYPE=CELL:+31 20 555 0134"), "got {card}");
    }

    /// Contact fields are `#[serde(default)]`, so a component that omits an
    /// untouched field must still produce a card rather than a parse error.
    #[test]
    fn a_partial_contact_payload_is_accepted() {
        let card = with_payload(r#"{ "kind": "contact", "firstName": "Sole" }"#)
            .payload
            .encode();
        assert!(card.contains("FN:Sole"), "got {card}");
    }

    /// The Preview component reads these keys by name.
    #[test]
    fn the_preview_response_carries_the_keys_the_ui_reads() {
        let value = preview_of(parse(REQUEST_JSON));

        for key in ["svg", "encoded", "modules", "sizePx", "verdict"] {
            assert!(value.get(key).is_some(), "preview is missing {key}");
        }
        let verdict = &value["verdict"];
        for key in ["level", "decoded", "contrast", "notes"] {
            assert!(verdict.get(key).is_some(), "verdict is missing {key}");
        }
        // The UI indexes a lookup table with this value, so an unexpected
        // spelling renders an empty badge rather than throwing.
        assert_eq!(verdict["level"], "good");
        assert_eq!(value["encoded"], "https://example.com");
    }

    /// A bare domain must come back with the scheme the UI then displays.
    #[test]
    fn detection_and_normalisation_survive_the_round_trip() {
        assert_eq!(detect_kind("example.com".into()), Kind::Link);
        let value = preview_of(with_payload(
            r#"{ "kind": "link", "input": "example.com" }"#,
        ));
        assert_eq!(value["encoded"], "https://example.com");
    }

    /// Asking for a logo when none is loaded must render a plain code rather
    /// than fail. The UI can send a stale fraction after a clear.
    #[test]
    fn a_logo_fraction_without_a_loaded_logo_renders_plainly() {
        let json = REQUEST_JSON.replace(r#""logoFraction": null"#, r#""logoFraction": 0.2"#);
        let value = preview_of(parse(&json));
        assert_eq!(value["verdict"]["level"], "good");
        assert!(!value["svg"].as_str().unwrap().contains("<image"));
    }
}
