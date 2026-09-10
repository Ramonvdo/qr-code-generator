//! An event the app remembers between runs.
//!
//! Before this, issuing was a one-shot: the signing key was filed by name and
//! everything else was retyped. That made a second batch a guessing game about
//! where the code went last time and which serial to start from, and it left
//! nowhere for the door to record who had already come in.
//!
//! An event owns its key, its issuance settings, its ticket ledger and its
//! redemption log, all under one directory so a whole event can be copied to
//! another machine or backed up by dragging one folder.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use crate::export::sanitize_stem;
use crate::pdf::{Element, QrElement, TextElement};
use crate::render::Ecc;
use crate::storage::{load_json, save_json, Loaded};
use crate::tickets::EventKey;
use crate::vars::{self, Vars};

/// The layout written by the version that had exactly one code and one
/// number.
///
/// Kept solely so those records can be read once and converted. Never written
/// back: `skip_serializing` on the field means a migrated event is saved in
/// the new shape and this shape disappears from disk.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegacyPlacement {
    qr_x: f32,
    qr_y: f32,
    qr_size: f32,
    show_number: bool,
    number_x: f32,
    number_y: f32,
    number_points: f32,
}

impl LegacyPlacement {
    /// The same design, expressed as elements.
    ///
    /// Positions carry across untouched, so an event issued before the upgrade
    /// and again after it puts the code in exactly the same place.
    fn into_elements(self) -> Vec<Element> {
        let mut out = vec![Element::Qr(QrElement {
            id: "qr".into(),
            x: self.qr_x,
            y: self.qr_y,
            size: self.qr_size,
            rotation: 0.0,
            opacity: 1.0,
        })];
        if self.show_number {
            out.push(Element::Text(TextElement {
                id: "serial".into(),
                x: self.number_x,
                y: self.number_y,
                template: "{{serial}}".into(),
                points: self.number_points,
                font: Default::default(),
                colour: "#000000".into(),
                align: Default::default(),
                rotation: 0.0,
                opacity: 1.0,
            }));
        }
        out
    }
}

/// What a new event starts with: a code, and its number under it.
pub fn default_layout() -> Vec<Element> {
    LegacyPlacement {
        qr_x: 0.62,
        qr_y: 0.12,
        qr_size: 0.26,
        show_number: true,
        number_x: 0.62,
        number_y: 0.46,
        number_points: 11.0,
    }
    .into_elements()
}

/// What each ticket's PDF is named. A template like everything else, so a run
/// can be filed by seat or by surname rather than only by number.
pub fn default_filename() -> String {
    "{{serial}}".into()
}

/// The template chosen for this event, if any.
///
/// The path is remembered rather than the file copied, so editing the design
/// and issuing again picks up the change. The consequence is that a moved or
/// deleted template is only discovered at issue time, which is why issuing
/// reports a missing template as a normal error rather than a panic.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateRef {
    pub path: String,
    pub name: String,
    pub page: usize,
    pub pages: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailTemplate {
    pub subject: String,
    pub body: String,
}

impl Default for EmailTemplate {
    fn default() -> Self {
        EmailTemplate {
            subject: "Your ticket for {{event}}".into(),
            body: "Hi {{name}},\n\nYour ticket for {{event}} is attached. \
                   Ticket number {{serial}}.\n\nSee you there."
                .into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    pub id: String,
    pub name: String,
    pub created_at: String,
    /// The serial the next issued ticket will carry.
    pub next_serial: u32,
    /// Everything drawn on a ticket, in stacking order.
    #[serde(default)]
    pub layout: Vec<Element>,
    /// Template for each ticket's filename, without the extension.
    #[serde(default = "default_filename")]
    pub filename: String,
    /// The imported columns in file order, remembered so a manifest rebuilt
    /// after the import is gone still carries the same columns in the same
    /// places.
    #[serde(default)]
    pub columns: Vec<String>,
    /// Only present in records written before layouts existed. Folded into
    /// `layout` on load and never written back.
    #[serde(default, skip_serializing)]
    placement: Option<LegacyPlacement>,
    pub template: Option<TemplateRef>,
    pub ecc: Ecc,
    pub dark: String,
    pub light: String,
    pub email: EmailTemplate,
}

impl Event {
    /// Bring a record written by an older version up to the current shape.
    ///
    /// Runs on every load. An event whose layout is empty either predates
    /// layouts or was saved by something that dropped it, and in both cases
    /// falling back to the default beats issuing a page with nothing on it.
    fn migrate(&mut self) {
        if let Some(old) = self.placement.take() {
            if self.layout.is_empty() {
                self.layout = old.into_elements();
            }
        }
        if self.layout.is_empty() {
            self.layout = default_layout();
        }
        if self.filename.trim().is_empty() {
            self.filename = default_filename();
        }
    }

    /// Every variable this event's design and copy refer to.
    ///
    /// The union across the layout, the subject, the body and the filename,
    /// minus what the app supplies itself. This is exactly the header row the
    /// imported CSV needs, which is why the example file is generated from it
    /// rather than written by hand.
    pub fn required_columns(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let mut add = |tokens: Vec<String>| {
            for t in tokens {
                if !vars::is_built_in(&t) && !out.contains(&t) {
                    out.push(t);
                }
            }
        };
        for element in &self.layout {
            add(element.referenced());
        }
        add(vars::referenced(&self.email.subject));
        add(vars::referenced(&self.email.body));
        add(vars::referenced(&self.filename));
        out
    }

    /// A copy for another run of the same thing.
    ///
    /// The design, the copy and the filename carry over; the key, the ledger
    /// and the admission log do not, and numbering restarts. A shared key
    /// would mean last year's tickets opening this year's door, which is
    /// exactly the failure the signature exists to prevent.
    pub fn duplicated(&self, name: &str) -> Event {
        Event {
            id: slug(name),
            name: name.trim().to_string(),
            created_at: now(),
            next_serial: 1,
            layout: self.layout.clone(),
            filename: self.filename.clone(),
            columns: self.columns.clone(),
            placement: None,
            template: self.template.clone(),
            ecc: self.ecc,
            dark: self.dark.clone(),
            light: self.light.clone(),
            email: self.email.clone(),
        }
    }

    /// Claim `count` serial numbers and advance the counter past them.
    ///
    /// Kept here rather than inline in the issuing command so the arithmetic
    /// that decides whether two runs collide is testable on its own. Two
    /// tickets sharing a serial is not a cosmetic problem: the door would
    /// admit the first and refuse the second as already used.
    pub fn allocate(&mut self, count: u32) -> (u32, u32) {
        let first = self.next_serial;
        let last = first + count - 1;
        self.next_serial = last + 1;
        (first, last)
    }

    pub fn new(name: &str) -> Event {
        Event {
            id: slug(name),
            name: name.trim().to_string(),
            created_at: now(),
            next_serial: 1,
            layout: default_layout(),
            filename: default_filename(),
            columns: Vec::new(),
            placement: None,
            template: None,
            ecc: Ecc::M,
            dark: "#000000".into(),
            light: "#ffffff".into(),
            email: EmailTemplate::default(),
        }
    }
}

/// Whether a ticket has reached its buyer.
///
/// Recorded per ticket rather than per run, so a send that stops halfway can
/// resume by looking at what is still pending instead of starting again and
/// emailing three hundred people twice.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum Delivery {
    #[default]
    Pending,
    Sent {
        at: String,
    },
    Failed {
        at: String,
        error: String,
    },
    /// Issued without an address, so there is nothing to send.
    NoAddress,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ticket {
    pub serial: u32,
    pub payload: String,
    /// Every column this ticket was issued from, so the manifest can be
    /// rebuilt later and a re-send says the same things the first one did.
    #[serde(default)]
    pub fields: BTreeMap<String, String>,
    #[serde(default)]
    pub email: String,
    /// Records written before columns were arbitrary carried a bare name.
    /// Folded into `fields` on load and never written back.
    #[serde(default, skip_serializing)]
    name: Option<String>,
    /// Path of this ticket's PDF, as written at issue time.
    #[serde(default)]
    pub file: String,
    pub issued_at: String,
    #[serde(default)]
    pub delivery: Delivery,
}

impl Ticket {
    pub fn new(
        serial: u32,
        payload: String,
        fields: BTreeMap<String, String>,
        email: String,
    ) -> Ticket {
        let mut fields = fields;
        // The address is a column like any other, so the manifest can carry it
        // in whichever position the import had it.
        if !email.trim().is_empty() {
            fields
                .entry("email".into())
                .or_insert_with(|| email.clone());
        }
        Ticket {
            serial,
            payload,
            delivery: if email.trim().is_empty() {
                Delivery::NoAddress
            } else {
                Delivery::Pending
            },
            fields,
            email,
            file: String::new(),
            issued_at: now(),
            name: None,
        }
    }

    fn migrate(&mut self) {
        if let Some(name) = self.name.take() {
            self.fields.entry("name".into()).or_insert(name);
        }
        if !self.email.is_empty() {
            self.fields
                .entry("email".into())
                .or_insert(self.email.clone());
        }
    }

    /// The name to show a person, from whichever column looks like one.
    ///
    /// Best effort by design: with arbitrary columns there is no guaranteed
    /// name field, and the door showing a blank is better than refusing to
    /// display anything at all.
    pub fn display_name(&self) -> String {
        for key in ["name", "full_name", "first_name"] {
            if let Some(v) = self.fields.get(key).filter(|v| !v.trim().is_empty()) {
                if key == "first_name" {
                    if let Some(last) = self
                        .fields
                        .get("last_name")
                        .filter(|l| !l.trim().is_empty())
                    {
                        return format!("{v} {last}");
                    }
                }
                return v.clone();
            }
        }
        String::new()
    }

    /// The values every template for this ticket resolves against.
    pub fn vars(&self, event_name: &str) -> Vars {
        let mut v = Vars(self.fields.clone());
        v.set("serial", format!("{:04}", self.serial))
            .set("serial_plain", self.serial.to_string())
            .set("ticket_code", self.payload.clone())
            .set("event", event_name)
            .set("issued_at", self.issued_at.clone());
        v
    }
}

/// One admission, keyed in the log by the payload that was scanned.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Redemption {
    pub serial: u32,
    pub at: String,
    /// Copied from the ticket when it is known, so an exported log is readable
    /// on its own without joining it back to the ledger.
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub id: String,
    pub name: String,
    pub issued: usize,
    pub redeemed: usize,
    pub next_serial: u32,
}

pub fn slug(name: &str) -> String {
    sanitize_stem(name)
}

pub fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// The events directory, and everything that reads or writes inside it.
///
/// Cloneable because it holds only a path: the sending thread gets its own
/// handle rather than borrowing one across a thread boundary.
#[derive(Clone)]
pub struct Store {
    root: PathBuf,
}

impl Store {
    pub fn new(root: PathBuf) -> Store {
        Store { root }
    }

    pub fn dir(&self, id: &str) -> PathBuf {
        self.root.join(id)
    }

    /// Move keys written by the version that filed them as `<slug>.key`.
    ///
    /// Those keys are already signing real tickets. Leaving them behind would
    /// silently mint a new key for the same event name, and every ticket
    /// issued before the upgrade would stop verifying at the door.
    pub fn migrate_legacy_keys(&self) -> Vec<String> {
        let mut warnings = Vec::new();
        let Ok(entries) = fs::read_dir(&self.root) else {
            return warnings;
        };

        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("key") {
                continue;
            }
            let Some(id) = path.file_stem().map(|s| s.to_string_lossy().into_owned()) else {
                continue;
            };

            let target = self.dir(&id).join("key.bin");
            if target.exists() {
                continue;
            }
            if let Err(e) =
                fs::create_dir_all(self.dir(&id)).and_then(|_| fs::rename(&path, &target))
            {
                warnings.push(format!(
                    "Could not move the existing signing key for \"{id}\" into its event folder: {e}. \
                     Tickets issued for that event before this version may not verify."
                ));
                continue;
            }

            // A key with no event record around it would be invisible in the
            // list, so give it one and carry the name forward.
            let event_path = self.dir(&id).join("event.json");
            if !event_path.exists() {
                let mut event = Event::new(&id);
                event.id = id.clone();
                let _ = save_json(&event_path, &event);
            }
        }
        warnings
    }

    pub fn list(&self) -> Vec<Summary> {
        let Ok(entries) = fs::read_dir(&self.root) else {
            return Vec::new();
        };
        let mut out: Vec<Summary> = entries
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir())
            .filter_map(|e| {
                let id = e.file_name().to_string_lossy().into_owned();
                match load_json::<Event>(&self.dir(&id).join("event.json")) {
                    Loaded::Parsed(event) => Some((id, event)),
                    _ => None,
                }
            })
            .map(|(id, event)| Summary {
                issued: self.tickets(&id).0.len(),
                redeemed: self.redemptions(&id).0.len(),
                next_serial: event.next_serial,
                name: event.name,
                id,
            })
            .collect();
        out.sort_by_key(|e| e.name.to_lowercase());
        out
    }

    pub fn load(&self, id: &str) -> (Option<Event>, Option<String>) {
        match load_json::<Event>(&self.dir(id).join("event.json")) {
            Loaded::Parsed(mut e) => {
                e.migrate();
                (Some(e), None)
            }
            Loaded::Missing => (None, None),
            Loaded::Corrupt { backup, error } => (
                None,
                Some(format!(
                    "The record for \"{id}\" could not be read ({error}). It was kept as {}.",
                    backup.display()
                )),
            ),
        }
    }

    pub fn save(&self, event: &Event) -> Result<(), String> {
        save_json(&self.dir(&event.id).join("event.json"), event)
    }

    /// This event's signing key, created on first use.
    pub fn key(&self, id: &str) -> Result<EventKey, String> {
        let path = self.dir(id).join("key.bin");
        if path.exists() {
            let seed = fs::read(&path).map_err(|e| format!("Could not read the key: {e}"))?;
            let seed: [u8; 32] = seed
                .try_into()
                .map_err(|_| format!("The stored key for \"{id}\" is the wrong size."))?;
            return Ok(EventKey::from_seed(&seed));
        }

        let key = EventKey::generate().map_err(|e| e.to_string())?;
        fs::create_dir_all(self.dir(id))
            .map_err(|e| format!("Could not create the folder: {e}"))?;
        crate::export::write_atomic(&path, &key.seed())
            .map_err(|e| format!("Could not save the key: {e}"))?;
        Ok(key)
    }

    pub fn tickets(&self, id: &str) -> (Vec<Ticket>, Option<String>) {
        let (mut tickets, warning) =
            load_json::<Vec<Ticket>>(&self.dir(id).join("tickets.json")).or_default();
        for ticket in &mut tickets {
            ticket.migrate();
        }
        (tickets, warning)
    }

    pub fn save_tickets(&self, id: &str, tickets: &[Ticket]) -> Result<(), String> {
        save_json(&self.dir(id).join("tickets.json"), &tickets.to_vec())
    }

    pub fn redemptions(&self, id: &str) -> (BTreeMap<String, Redemption>, Option<String>) {
        load_json::<BTreeMap<String, Redemption>>(&self.dir(id).join("redeemed.json")).or_default()
    }

    pub fn save_redemptions(
        &self,
        id: &str,
        log: &BTreeMap<String, Redemption>,
    ) -> Result<(), String> {
        save_json(&self.dir(id).join("redeemed.json"), log)
    }
}

/// The ticket manifest as CSV.
///
/// Every column that was imported, in the order they appeared, then what the
/// app knows. That makes this a drop-in replacement for the list that was
/// imported rather than a separate report to reconcile against it, which is
/// why the original file is never touched.
///
/// `ticket_sent` is empty until the message goes out and then holds the
/// ticket number that person received, so a glance down the column shows who
/// is still waiting.
pub fn manifest_csv(event_name: &str, columns: &[String], tickets: &[Ticket]) -> String {
    let mut header: Vec<String> = columns.to_vec();
    header.extend(
        [
            "serial",
            "ticket_code",
            "ticket_sent",
            "sent_at",
            "event",
            "file",
        ]
        .iter()
        .map(|s| s.to_string()),
    );
    let mut out = header
        .iter()
        .map(|h| csv_field(h))
        .collect::<Vec<_>>()
        .join(",");
    out.push('\n');

    for t in tickets {
        let (sent, sent_at) = match &t.delivery {
            Delivery::Sent { at } => (t.serial.to_string(), at.clone()),
            _ => (String::new(), String::new()),
        };
        let mut row: Vec<String> = columns
            .iter()
            .map(|c| csv_field(t.fields.get(c).map(|s| s.as_str()).unwrap_or("")))
            .collect();
        row.push(t.serial.to_string());
        row.push(csv_field(&t.payload));
        row.push(sent);
        row.push(sent_at);
        row.push(csv_field(event_name));
        row.push(csv_field(&t.file));
        out.push_str(&row.join(","));
        out.push('\n');
    }
    out
}

/// The redemption log as CSV, for reconciling two doors after the fact.
pub fn redemption_csv(log: &BTreeMap<String, Redemption>) -> String {
    let mut out = String::from("serial,name,scanned_at,payload\n");
    for (payload, r) in log {
        out.push_str(&format!(
            "{},{},{},{}\n",
            r.serial,
            csv_field(&r.name),
            r.at,
            payload
        ));
    }
    out
}

fn csv_field(raw: &str) -> String {
    if raw.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", raw.replace('"', "\"\""))
    } else {
        raw.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(tag: &str) -> Store {
        let root = std::env::temp_dir().join(format!("qrgen-events-{tag}"));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        Store::new(root)
    }

    /// Exactly what the previous version wrote, so the migration is tested
    /// against the real shape rather than one reconstructed from memory.
    const LEGACY_EVENT: &str = r##"{
        "id": "summer-fest",
        "name": "Summer Fest",
        "createdAt": "2026-09-01T10:00:00Z",
        "nextSerial": 138,
        "placement": {
            "qrX": 0.62, "qrY": 0.12, "qrSize": 0.26,
            "showNumber": true,
            "numberX": 0.62, "numberY": 0.46, "numberPoints": 11.0
        },
        "template": null,
        "ecc": "m",
        "dark": "#000000",
        "light": "#ffffff",
        "email": { "subject": "Your ticket", "body": "Hi" }
    }"##;

    /// An upgrade must not move anybody's code. The positions carry across
    /// untouched, so a second batch lands where the first one did.
    #[test]
    fn an_old_placement_becomes_the_same_design_as_elements() {
        let s = store("legacy-event");
        std::fs::create_dir_all(s.dir("summer-fest")).unwrap();
        std::fs::write(s.dir("summer-fest").join("event.json"), LEGACY_EVENT).unwrap();

        let event = s.load("summer-fest").0.expect("must load");
        assert_eq!(event.next_serial, 138, "numbering is untouched");
        assert_eq!(event.layout.len(), 2, "a code and its number");

        match &event.layout[0] {
            Element::Qr(qr) => {
                assert!((qr.x - 0.62).abs() < 1e-6);
                assert!((qr.y - 0.12).abs() < 1e-6);
                assert!((qr.size - 0.26).abs() < 1e-6);
            }
            other => panic!("expected the code first, got {other:?}"),
        }
        match &event.layout[1] {
            Element::Text(t) => {
                assert_eq!(t.template, "{{serial}}");
                assert!((t.x - 0.62).abs() < 1e-6);
                assert!((t.points - 11.0).abs() < 1e-6);
            }
            other => panic!("expected the number second, got {other:?}"),
        }
    }

    #[test]
    fn an_old_placement_without_a_number_migrates_to_the_code_alone() {
        let s = store("legacy-nonumber");
        std::fs::create_dir_all(s.dir("summer-fest")).unwrap();
        std::fs::write(
            s.dir("summer-fest").join("event.json"),
            LEGACY_EVENT.replace("\"showNumber\": true", "\"showNumber\": false"),
        )
        .unwrap();

        let event = s.load("summer-fest").0.unwrap();
        assert_eq!(event.layout.len(), 1);
        assert!(matches!(event.layout[0], Element::Qr(_)));
    }

    /// Once migrated, the old shape must not be written back out, or every
    /// load would keep re-running the migration.
    #[test]
    fn a_migrated_event_is_saved_in_the_new_shape() {
        let s = store("legacy-save");
        std::fs::create_dir_all(s.dir("summer-fest")).unwrap();
        std::fs::write(s.dir("summer-fest").join("event.json"), LEGACY_EVENT).unwrap();

        let event = s.load("summer-fest").0.unwrap();
        s.save(&event).unwrap();
        let raw = std::fs::read_to_string(s.dir("summer-fest").join("event.json")).unwrap();
        assert!(!raw.contains("placement"), "the old shape is gone");
        assert!(raw.contains("layout"), "and replaced by the new one");
    }

    /// Tickets issued before columns were arbitrary keep their details.
    #[test]
    fn an_old_ticket_keeps_its_name_and_address_as_fields() {
        let s = store("legacy-ticket");
        std::fs::create_dir_all(s.dir("fest")).unwrap();
        std::fs::write(
            s.dir("fest").join("tickets.json"),
            r#"[{
                "serial": 7,
                "payload": "TKT1.a.b",
                "name": "Marieke van Dijk",
                "email": "m@example.com",
                "file": "tickets/0007.pdf",
                "issuedAt": "2026-09-01T10:00:00Z",
                "delivery": { "status": "sent", "at": "2026-09-01T11:00:00Z" }
            }]"#,
        )
        .unwrap();

        let (tickets, warning) = s.tickets("fest");
        assert!(warning.is_none());
        assert_eq!(tickets[0].fields["name"], "Marieke van Dijk");
        assert_eq!(tickets[0].fields["email"], "m@example.com");
        assert_eq!(tickets[0].display_name(), "Marieke van Dijk");
        assert!(
            matches!(tickets[0].delivery, Delivery::Sent { .. }),
            "a ticket already sent must not look unsent afterwards"
        );
    }

    #[test]
    fn a_duplicate_keeps_the_design_but_starts_fresh() {
        let mut original = Event::new("Summer Fest 2026");
        original.filename = "{{last_name}}-{{serial}}".into();
        original.columns = vec!["last_name".into()];
        original.allocate(400);

        let copy = original.duplicated("Summer Fest 2027");
        assert_eq!(copy.next_serial, 1, "a fresh batch starts at one");
        assert_eq!(copy.filename, original.filename);
        assert_eq!(copy.columns, original.columns);
        assert_eq!(copy.layout.len(), original.layout.len());
        assert_ne!(copy.id, original.id);
    }

    #[test]
    fn required_columns_are_what_the_design_and_copy_ask_for() {
        let mut event = Event::new("Fest");
        event.layout = vec![
            Element::Qr(QrElement {
                id: "qr".into(),
                x: 0.1,
                y: 0.1,
                size: 0.2,
                rotation: 0.0,
                opacity: 1.0,
            }),
            Element::Text(TextElement {
                id: "n".into(),
                x: 0.1,
                y: 0.4,
                template: "{{first_name}} {{last_name}}".into(),
                points: 12.0,
                font: Default::default(),
                colour: "#000000".into(),
                align: Default::default(),
                rotation: 0.0,
                opacity: 1.0,
            }),
        ];
        event.email.subject = "{{event}} ticket for {{first_name}}".into();
        event.email.body = "Seat {{seat}}. Number {{serial}}.".into();
        event.filename = "{{last_name}}".into();

        assert_eq!(
            event.required_columns(),
            vec!["first_name", "last_name", "seat"],
            "each once, in the order encountered, and no built-ins"
        );
    }

    #[test]
    fn a_new_event_starts_at_serial_one() {
        let event = Event::new("Summer Fest 2026");
        assert_eq!(event.next_serial, 1);
        assert_eq!(event.id, "summer-fest-2026");
        assert_eq!(event.name, "Summer Fest 2026");
    }

    /// Two runs must never hand out the same number.
    #[test]
    fn a_second_run_continues_where_the_first_stopped() {
        let mut event = Event::new("Fest");
        assert_eq!(event.allocate(100), (1, 100));
        assert_eq!(event.next_serial, 101);
        assert_eq!(event.allocate(50), (101, 150));
        assert_eq!(event.next_serial, 151);
    }

    #[test]
    fn allocating_one_ticket_gives_a_single_serial() {
        let mut event = Event::new("Fest");
        assert_eq!(event.allocate(1), (1, 1));
        assert_eq!(event.next_serial, 2);
    }

    /// Reopening an event has to read the counter back, or the second batch
    /// would start at one and duplicate every number in the first.
    #[test]
    fn the_counter_survives_a_save_and_reload() {
        let s = store("serials");
        let mut event = Event::new("Fest");
        event.allocate(137);
        s.save(&event).unwrap();

        let mut reloaded = s.load("fest").0.expect("must load");
        assert_eq!(reloaded.allocate(1), (138, 138));
    }

    #[test]
    fn events_round_trip_through_the_store() {
        let s = store("roundtrip");
        let mut event = Event::new("Summer Fest");
        event.next_serial = 138;
        s.save(&event).unwrap();

        let (loaded, warning) = s.load("summer-fest");
        assert!(warning.is_none());
        let loaded = loaded.expect("saved events must load");
        assert_eq!(loaded.next_serial, 138);
        assert_eq!(loaded.name, "Summer Fest");
    }

    #[test]
    fn a_key_is_created_once_and_then_reused() {
        let s = store("key");
        let first = s.key("summer-fest").unwrap();
        let second = s.key("summer-fest").unwrap();
        assert_eq!(
            first.public_key(),
            second.public_key(),
            "reopening an event must not mint a new key, or tickets already \
             issued would stop verifying"
        );
    }

    /// The migration exists so an upgrade does not orphan a key that is
    /// already signing tickets in the wild.
    #[test]
    fn a_legacy_key_is_moved_and_still_verifies_its_old_tickets() {
        let s = store("migrate");
        let key = EventKey::from_seed(&[3u8; 32]);
        let ticket = key.issue("Summer Fest", 1);
        let public = key.public_key();

        // The layout the previous version wrote.
        fs::write(s.root.join("summer-fest.key"), key.seed()).unwrap();

        let warnings = s.migrate_legacy_keys();
        assert!(warnings.is_empty(), "got {warnings:?}");
        assert!(
            !s.root.join("summer-fest.key").exists(),
            "the old file moved"
        );
        assert!(s.dir("summer-fest").join("key.bin").exists());

        let reloaded = s.key("summer-fest").unwrap();
        assert_eq!(reloaded.public_key(), public, "same key, same public half");
        assert!(
            crate::tickets::verify(&reloaded.public_key(), &ticket).is_ok(),
            "tickets issued before the upgrade must still verify"
        );
        assert!(
            s.list().iter().any(|e| e.id == "summer-fest"),
            "and the event becomes visible in the list"
        );
    }

    #[test]
    fn migration_leaves_an_already_migrated_event_alone() {
        let s = store("migrate-twice");
        fs::write(s.root.join("fest.key"), [1u8; 32]).unwrap();
        s.migrate_legacy_keys();
        let first = s.key("fest").unwrap().public_key();
        s.migrate_legacy_keys();
        assert_eq!(s.key("fest").unwrap().public_key(), first);
    }

    #[test]
    fn tickets_and_redemptions_persist() {
        let s = store("ledger");
        let tickets = vec![Ticket::new(
            1,
            "TKT1.a.b".into(),
            BTreeMap::from([("name".to_string(), "Marieke".to_string())]),
            "m@example.com".into(),
        )];
        s.save_tickets("fest", &tickets).unwrap();
        let (back, warning) = s.tickets("fest");
        assert!(warning.is_none());
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].delivery, Delivery::Pending);

        let mut log = BTreeMap::new();
        log.insert(
            "TKT1.a.b".to_string(),
            Redemption {
                serial: 1,
                at: now(),
                name: "Marieke".into(),
            },
        );
        s.save_redemptions("fest", &log).unwrap();
        assert_eq!(s.redemptions("fest").0.len(), 1);
    }

    #[test]
    fn an_empty_store_lists_nothing_rather_than_failing() {
        assert!(store("empty").list().is_empty());
    }

    #[test]
    fn the_summary_counts_what_is_on_disk() {
        let s = store("summary");
        let mut event = Event::new("Fest");
        event.next_serial = 4;
        s.save(&event).unwrap();
        s.save_tickets(
            "fest",
            &(1..=3)
                .map(|n| Ticket::new(n, format!("TKT1.{n}.x"), BTreeMap::new(), String::new()))
                .collect::<Vec<_>>(),
        )
        .unwrap();

        let list = s.list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].issued, 3);
        assert_eq!(list[0].redeemed, 0);
        assert_eq!(list[0].next_serial, 4);
    }

    fn sample_ticket(serial: u32, name: &str, email: &str) -> Ticket {
        let mut t = Ticket::new(
            serial,
            format!("TKT1.body{serial}.sig"),
            BTreeMap::from([("name".to_string(), name.to_string())]),
            email.to_string(),
        );
        t.file = format!("tickets/{serial:04}.pdf");
        t.issued_at = "2026-09-07T12:00:00Z".into();
        t
    }

    #[test]
    fn the_manifest_has_a_header_and_one_row_per_ticket() {
        let tickets = vec![
            sample_ticket(1, "Marieke", "m@example.com"),
            sample_ticket(2, "", ""),
        ];
        let columns = vec!["name".to_string(), "email".to_string()];
        let csv = manifest_csv("Fest", &columns, &tickets);
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(
            lines[0], "name,email,serial,ticket_code,ticket_sent,sent_at,event,file",
            "the imported columns come first, so this replaces the source list"
        );
        assert_eq!(lines.len(), 3);
        assert!(lines[1].starts_with("Marieke,m@example.com,1,"));
    }

    /// The column that will contain a comma in real data is the name.
    #[test]
    fn a_name_with_a_comma_is_quoted_in_the_manifest() {
        let csv = manifest_csv(
            "Fest",
            &["name".to_string()],
            &[sample_ticket(1, "Reyes, Tomas", "t@example.com")],
        );
        assert!(csv.contains("\"Reyes, Tomas\""), "got {csv}");
        assert_eq!(
            csv.lines().count(),
            2,
            "the quoted comma is not a new field"
        );
    }

    #[test]
    fn the_redemption_export_quotes_a_name_with_a_comma() {
        let mut log = BTreeMap::new();
        log.insert(
            "TKT1.a.b".to_string(),
            Redemption {
                serial: 7,
                at: "2026-09-07T20:14:00Z".into(),
                name: "Reyes, Tomas".into(),
            },
        );
        let csv = redemption_csv(&log);
        assert!(csv.starts_with("serial,name,scanned_at,payload\n"));
        assert!(
            csv.contains("7,\"Reyes, Tomas\",2026-09-07T20:14:00Z,TKT1.a.b"),
            "got {csv}"
        );
    }
}
