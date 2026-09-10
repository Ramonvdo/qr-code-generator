//! Sending a ticket to the person who bought it.
//!
//! This is the only part of the app that touches the network, and it only does
//! so when someone presses Send. Nothing is scheduled, nothing polls, and no
//! other feature opens a connection. That is a deliberate limit: a laptop is
//! the wrong place for delivery that must not fail, because it only works
//! while the machine is awake, online and running this app.
//!
//! What leaves the machine is the buyer's address, the subject and body, and
//! their ticket PDF, addressed to Resend. The signing key never does.
//!
//! The transport sits behind a trait so the tests can assert exactly what
//! would be sent without sending anything.

use serde::{Deserialize, Serialize};

use crate::b64;
use crate::vars::Vars;

/// Resend's documented ceiling for one request, attachments included.
const MAX_ATTACHMENT_BYTES: usize = 38 * 1024 * 1024;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Config {
    pub api_key: String,
    /// Must be an address on a domain verified with Resend, or every send
    /// fails identically.
    pub from: String,
    pub reply_to: String,
}

impl Config {
    pub fn is_ready(&self) -> bool {
        !self.api_key.trim().is_empty() && !self.from.trim().is_empty()
    }
}

/// One message, already rendered and ready to hand to a transport.
#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub to: String,
    pub subject: String,
    pub body: String,
    pub attachment: Option<(String, Vec<u8>)>,
}

#[derive(Debug)]
pub enum SendError {
    NotConfigured(&'static str),
    TooLarge(usize),
    /// Resend answered, and this is what it said. Surfaced verbatim: a
    /// generic "sending failed" hides the one detail that matters, which is
    /// almost always an unverified sending domain.
    Rejected(String),
    Transport(String),
}

impl std::fmt::Display for SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            SendError::NotConfigured(what) => write!(f, "{what}"),
            SendError::TooLarge(bytes) => write!(
                f,
                "That ticket is {:.1} MB, over the {} MB limit for one message.",
                *bytes as f64 / 1_048_576.0,
                MAX_ATTACHMENT_BYTES / 1_048_576
            ),
            SendError::Rejected(msg) => write!(f, "{msg}"),
            SendError::Transport(e) => write!(f, "Could not reach the mail service: {e}"),
        }
    }
}

impl std::error::Error for SendError {}

/// Fill a subject or body from a ticket's variables.
///
/// The same engine the ticket layout and the filenames use, so a column that
/// prints on the ticket can be written into the email without anything here
/// knowing what it is called.
///
/// The one special case: a greeting has to read properly for a buyer whose
/// name column is blank, so an empty `{{name}}` or `{{first_name}}` becomes
/// "there" rather than leaving "Hi ,".
pub fn render(template: &str, vars: &Vars) -> String {
    let mut vars = vars.clone();
    for token in ["name", "first_name"] {
        if vars
            .get(token)
            .map(|v| v.trim().is_empty())
            .unwrap_or(false)
        {
            vars.set(token, "there");
        }
    }
    vars.render(template)
}

/// Turn plain text into the HTML Resend expects, without inventing a layout.
///
/// Escaped first, because a buyer called `A & B <Ltd>` would otherwise arrive
/// with a broken tag in the middle of their own name.
pub fn to_html(body: &str) -> String {
    let escaped = body
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    format!(
        "<div style=\"font-family:system-ui,-apple-system,'Segoe UI',sans-serif;\
         font-size:15px;line-height:1.55\">{}</div>",
        escaped.replace('\n', "<br>")
    )
}

/// The request body Resend receives.
///
/// Built as a value rather than a formatted string so the tests can inspect it
/// field by field, and so a name containing a quote cannot break the JSON.
pub fn request_body(config: &Config, message: &Message) -> serde_json::Value {
    let mut body = serde_json::json!({
        "from": config.from,
        "to": [message.to],
        "subject": message.subject,
        "html": to_html(&message.body),
        "text": message.body,
    });

    if !config.reply_to.trim().is_empty() {
        body["reply_to"] = serde_json::json!(config.reply_to.trim());
    }
    if let Some((filename, bytes)) = &message.attachment {
        body["attachments"] = serde_json::json!([{
            "filename": filename,
            "content": b64::encode(bytes),
        }]);
    }
    body
}

/// Something that can deliver a message. Implemented once for real, and once
/// in the tests so nothing is sent while checking what would be.
pub trait Transport {
    fn send(&self, config: &Config, message: &Message) -> Result<(), SendError>;
}

pub struct Resend;

impl Transport for Resend {
    fn send(&self, config: &Config, message: &Message) -> Result<(), SendError> {
        if !config.is_ready() {
            return Err(SendError::NotConfigured(
                "Set the Resend API key and a from address in Settings first.",
            ));
        }
        if let Some((_, bytes)) = &message.attachment {
            if bytes.len() > MAX_ATTACHMENT_BYTES {
                return Err(SendError::TooLarge(bytes.len()));
            }
        }

        let response = ureq::post("https://api.resend.com/emails")
            .set(
                "Authorization",
                &format!("Bearer {}", config.api_key.trim()),
            )
            .set("Content-Type", "application/json")
            .send_json(request_body(config, message));

        match response {
            Ok(_) => Ok(()),
            Err(ureq::Error::Status(code, response)) => {
                let detail = response
                    .into_string()
                    .ok()
                    .and_then(|raw| {
                        serde_json::from_str::<serde_json::Value>(&raw)
                            .ok()
                            .and_then(|v| {
                                v.get("message")
                                    .and_then(|m| m.as_str())
                                    .map(|s| s.to_string())
                            })
                            .or(Some(raw))
                    })
                    .unwrap_or_default();
                Err(SendError::Rejected(if detail.is_empty() {
                    format!("The mail service refused the message (HTTP {code}).")
                } else {
                    detail
                }))
            }
            Err(e) => Err(SendError::Transport(e.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> Config {
        Config {
            api_key: "re_test".into(),
            from: "Tickets <tickets@example.com>".into(),
            reply_to: String::new(),
        }
    }

    fn message() -> Message {
        Message {
            to: "marieke@example.com".into(),
            subject: "Your ticket for Summer Fest".into(),
            body: "Hi Marieke,\n\nTicket 0007.".into(),
            attachment: Some(("0007-marieke.pdf".into(), b"%PDF-1.5 fake".to_vec())),
        }
    }

    fn sample_vars(name: &str) -> Vars {
        let mut v = Vars::default();
        v.set("name", name)
            .set("event", "Fest")
            .set("serial", "0007")
            .set("seat", "A12");
        v
    }

    #[test]
    fn placeholders_are_filled() {
        assert_eq!(
            render(
                "Hi {{name}}, {{event}} ticket {{serial}}.",
                &sample_vars("Marieke")
            ),
            "Hi Marieke, Fest ticket 0007."
        );
    }

    /// The point of the shared engine: a column nobody wrote code for is
    /// usable in the copy the moment it exists in the CSV.
    #[test]
    fn any_column_can_be_written_into_the_message() {
        assert_eq!(
            render("You are in seat {{seat}}.", &sample_vars("A")),
            "You are in seat A12."
        );
    }

    /// A buyer list with a blank name column should not produce "Hi ,".
    #[test]
    fn a_missing_name_still_reads_as_a_sentence() {
        assert_eq!(render("Hi {{name}},", &sample_vars("")), "Hi there,");
        assert_eq!(render("Hi {{name}},", &sample_vars("   ")), "Hi there,");
    }

    #[test]
    fn an_unknown_placeholder_is_left_alone_rather_than_blanked() {
        assert_eq!(render("{{nope}}", &sample_vars("A")), "{{nope}}");
    }

    /// A name is untrusted text arriving from a spreadsheet.
    #[test]
    fn html_escaping_survives_a_name_with_markup_in_it() {
        let html = to_html("Hi A & B <Ltd>");
        assert!(html.contains("A &amp; B &lt;Ltd&gt;"), "got {html}");
        assert!(!html.contains("<Ltd>"), "no raw tag reaches the message");
    }

    #[test]
    fn newlines_become_line_breaks() {
        assert!(to_html("one\ntwo").contains("one<br>two"));
    }

    #[test]
    fn the_request_carries_everything_resend_needs() {
        let body = request_body(&config(), &message());
        assert_eq!(body["from"], "Tickets <tickets@example.com>");
        assert_eq!(body["to"][0], "marieke@example.com");
        assert_eq!(body["subject"], "Your ticket for Summer Fest");
        assert!(body["text"].as_str().unwrap().contains("Ticket 0007"));
        assert!(body["html"].as_str().unwrap().contains("<br>"));

        let attachment = &body["attachments"][0];
        assert_eq!(attachment["filename"], "0007-marieke.pdf");
        assert_eq!(
            attachment["content"],
            b64::encode(b"%PDF-1.5 fake"),
            "the PDF is base64 encoded, standard alphabet with padding"
        );
    }

    #[test]
    fn reply_to_is_omitted_rather_than_sent_empty() {
        let body = request_body(&config(), &message());
        assert!(body.get("reply_to").is_none());

        let with = Config {
            reply_to: "hello@example.com".into(),
            ..config()
        };
        assert_eq!(
            request_body(&with, &message())["reply_to"],
            "hello@example.com"
        );
    }

    /// A quote in a buyer's name must not be able to break the request.
    #[test]
    fn quotes_in_a_name_cannot_break_the_json() {
        let msg = Message {
            subject: "Ticket for \"Reyes\", Tomas".into(),
            ..message()
        };
        let body = request_body(&config(), &msg);
        let round_tripped: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&body).unwrap()).unwrap();
        assert_eq!(round_tripped["subject"], "Ticket for \"Reyes\", Tomas");
    }

    #[test]
    fn sending_without_configuration_is_refused_before_any_request() {
        let err = Resend
            .send(&Config::default(), &message())
            .expect_err("must refuse");
        assert!(matches!(err, SendError::NotConfigured(_)));
        assert!(err.to_string().contains("Settings"));
    }

    #[test]
    fn a_key_without_a_from_address_is_not_ready() {
        assert!(!Config {
            api_key: "re_x".into(),
            ..Config::default()
        }
        .is_ready());
        assert!(config().is_ready());
    }

    #[test]
    fn an_oversized_attachment_is_refused_locally() {
        let msg = Message {
            attachment: Some(("big.pdf".into(), vec![0u8; MAX_ATTACHMENT_BYTES + 1])),
            ..message()
        };
        let err = Resend.send(&config(), &msg).expect_err("must refuse");
        assert!(matches!(err, SendError::TooLarge(_)));
        assert!(err.to_string().contains("MB"));
    }
}
