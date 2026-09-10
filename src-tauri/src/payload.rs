//! What the user typed, and the exact string that ends up inside the QR code.
//!
//! Pure: no IO, no Tauri, no globals. Everything here is a candidate for the
//! test suite, which is the point, because the encoded string is invisible. A
//! wrong one produces a QR that scans perfectly and then does the wrong thing,
//! which is far worse than one that fails to scan.
//!
//! The escaping rules are the whole reason this module exists. A Wi-Fi
//! password containing a semicolon, encoded naively, yields a code that reads
//! cleanly and then silently fails to join the network.

use serde::{Deserialize, Serialize};

/// How the raw input should be interpreted.
///
/// The user can always override the guess (see [`detect`]), so this is a
/// starting point rather than a verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    Link,
    Text,
}

/// URI schemes that are already complete and must never be rewritten.
///
/// `mailto:` and friends have no `//`, so the generic "does it contain `://`"
/// check misses them and they would get an `https://` glued on the front.
const BARE_SCHEMES: [&str; 6] = ["mailto:", "tel:", "sms:", "geo:", "bitcoin:", "magnet:"];

/// Suffixes common enough that a dotted string ending in one is almost
/// certainly meant as a web address.
///
/// Deliberately a short list rather than the full IANA registry. A miss costs
/// one click on the Link/Text toggle, and the encoded string is always on
/// screen, so a wrong guess is obvious and cheap. Carrying 1,400 TLDs to avoid
/// that click is not a trade worth making.
const COMMON_TLDS: [&str; 34] = [
    "com", "org", "net", "edu", "gov", "int", "io", "dev", "app", "co", "ai", "me", "info", "xyz",
    "online", "site", "shop", "blog", "cloud", "tech", "nl", "de", "uk", "fr", "es", "it", "be",
    "eu", "ch", "at", "se", "no", "dk", "pl",
];

/// Everything the app knows how to put inside a code.
///
/// Internally tagged, so the webview sends one flat object per type and serde
/// picks the variant from `kind`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Payload {
    Link { input: String },
    Text { input: String },
    Wifi(Wifi),
    Contact(Contact),
}

impl Payload {
    pub fn encode(&self) -> String {
        match self {
            Payload::Link { input } => normalize_url(input),
            // Passed through untouched, whitespace included: a code carrying a
            // snippet must reproduce it byte for byte.
            Payload::Text { input } => input.clone(),
            Payload::Wifi(w) => w.encode(),
            Payload::Contact(c) => c.encode(),
        }
    }

    /// The same interpretation applied to different content.
    ///
    /// Batch export uses this so every line inherits the template's kind, and
    /// a list of bare domains all gain their scheme the same way one would.
    /// The structured types return `None`: a Wi-Fi network or a contact card
    /// cannot be driven from a single line of text, and quietly encoding one
    /// as plain text would be worse than refusing.
    pub fn with_input(&self, input: String) -> Option<Payload> {
        match self {
            Payload::Link { .. } => Some(Payload::Link { input }),
            Payload::Text { .. } => Some(Payload::Text { input }),
            Payload::Wifi(_) | Payload::Contact(_) => None,
        }
    }
}

// ------------------------------------------------------------------ links --

/// Guess whether `input` is meant as a web address.
///
/// Three accepting cases, in order of confidence: an explicit scheme, a `www.`
/// prefix, and a dotted host whose last label is a common TLD.
pub fn detect(input: &str) -> Kind {
    let s = input.trim();
    if s.is_empty() || s.contains(char::is_whitespace) {
        return Kind::Text;
    }

    let lower = s.to_ascii_lowercase();

    // An explicit scheme settles it. Checked before `://` so that
    // `mailto:a@b` is recognised even though it has no slashes.
    if BARE_SCHEMES.iter().any(|p| lower.starts_with(p)) {
        return Kind::Link;
    }
    // `://` must come before any path separator, or `a/b://c` would qualify.
    if let Some(i) = lower.find("://") {
        if !lower[..i].contains('/') {
            return Kind::Link;
        }
    }
    if lower.starts_with("www.") {
        return Kind::Link;
    }

    let host = lower
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("")
        .trim_end_matches('.');
    // Strip a :port so `example.com:8080` still resolves to the `com` label.
    let host = host.split(':').next().unwrap_or("");

    match host.rsplit_once('.') {
        Some((prefix, tld)) if !prefix.is_empty() && COMMON_TLDS.contains(&tld) => Kind::Link,
        _ => Kind::Text,
    }
}

/// Add a scheme to a bare address, and nothing else.
///
/// Phones treat a code containing `example.com` as a search term and one
/// containing `https://example.com` as a destination, so the missing scheme is
/// the difference between the code working and not. Beyond that the string is
/// untouched: no case folding, no trailing-slash rules, no percent encoding.
/// The user pasted a URL that works and must get that URL back.
pub fn normalize_url(input: &str) -> String {
    let s = input.trim();
    if s.is_empty() {
        return String::new();
    }
    let lower = s.to_ascii_lowercase();

    if BARE_SCHEMES.iter().any(|p| lower.starts_with(p)) {
        return s.to_string();
    }
    if let Some(i) = lower.find("://") {
        if !lower[..i].contains('/') {
            return s.to_string();
        }
    }
    format!("https://{s}")
}

// ------------------------------------------------------------------ wi-fi --

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Security {
    Wpa,
    Wep,
    Nopass,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Wifi {
    pub ssid: String,
    pub password: String,
    pub security: Security,
    pub hidden: bool,
}

/// Backslash-escape the characters that structure a `WIFI:` string.
///
/// All five round-trip through the reference ZXing parser, which unescapes
/// exactly this set. Escaping is applied before any quoting so the wrapping
/// quotes of a hex value do not get escaped themselves.
fn wifi_escape(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for c in raw.chars() {
        if matches!(c, '\\' | ';' | ',' | ':' | '"') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Lengths at which an all-hex string is a valid raw key: 10 and 26 hex
/// digits for 64- and 128-bit WEP, 64 for a WPA PSK.
const HEX_KEY_LENGTHS: [usize; 3] = [10, 26, 64];

/// Quote a password a scanner could otherwise read as a raw hex key.
///
/// The format allows quoting to force string interpretation, but applying it
/// to anything that merely looks hexadecimal is far too eager: `Cafe`, `Face`,
/// `Beef`, `Decade` and `Facade` are all valid hex, and quoting an ordinary
/// SSID breaks every scanner that does not strip the quotes back off.
///
/// The ambiguity is only genuine when the string could actually be a key, so
/// the length has to match one too. Below that, no scanner is going to read
/// `abc123` as a six-digit key, because there is no such thing.
fn wifi_password(raw: &str) -> String {
    let escaped = wifi_escape(raw);
    let ambiguous =
        HEX_KEY_LENGTHS.contains(&raw.len()) && raw.chars().all(|c| c.is_ascii_hexdigit());
    if ambiguous {
        format!("\"{escaped}\"")
    } else {
        escaped
    }
}

impl Wifi {
    pub fn encode(&self) -> String {
        let t = match self.security {
            Security::Wpa => "WPA",
            Security::Wep => "WEP",
            Security::Nopass => "nopass",
        };
        // The SSID is escaped but never quoted: it is a name, not a key, so
        // the hex ambiguity that justifies quoting does not arise.
        let mut s = format!("WIFI:T:{t};S:{};", wifi_escape(&self.ssid));
        // An open network carries no password field at all. Sending an empty
        // `P:;` makes some Android builds prompt for a key that does not exist.
        if self.security != Security::Nopass {
            s.push_str(&format!("P:{};", wifi_password(&self.password)));
        }
        if self.hidden {
            s.push_str("H:true;");
        }
        // The trailing empty field terminates the record; the format needs the
        // second semicolon and scanners reject the string without it.
        s.push(';');
        s
    }
}

// ---------------------------------------------------------------- contact --

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Contact {
    pub first_name: String,
    pub last_name: String,
    pub org: String,
    pub title: String,
    pub phone: String,
    pub email: String,
    pub url: String,
}

/// Escape a vCard property value per RFC 6350 section 3.4.
///
/// Backslash first, or it would double-escape the backslashes introduced by
/// the other rules. A literal newline becomes `\n`, since a raw CRLF would
/// start a new property line and truncate the card.
fn vcard_escape(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for c in raw.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            ';' => out.push_str("\\;"),
            ',' => out.push_str("\\,"),
            '\n' => out.push_str("\\n"),
            '\r' => {}
            _ => out.push(c),
        }
    }
    out
}

impl Contact {
    /// vCard 3.0 rather than the more compact MECARD.
    ///
    /// MECARD produces a smaller code, but iOS does not offer to create a
    /// contact from it. A card that only works on half of all phones is not
    /// worth the handful of saved modules.
    pub fn encode(&self) -> String {
        let full = format!("{} {}", self.first_name.trim(), self.last_name.trim());
        let full = full.trim();

        let mut lines = vec![
            "BEGIN:VCARD".to_string(),
            "VERSION:3.0".to_string(),
            // N is structured: last;first;middle;prefix;suffix. The separators
            // are literal, so only the components get escaped.
            format!(
                "N:{};{};;;",
                vcard_escape(self.last_name.trim()),
                vcard_escape(self.first_name.trim())
            ),
        ];
        if !full.is_empty() {
            lines.push(format!("FN:{}", vcard_escape(full)));
        }
        for (prop, value) in [
            ("ORG", &self.org),
            ("TITLE", &self.title),
            ("TEL;TYPE=CELL", &self.phone),
            ("EMAIL", &self.email),
            ("URL", &self.url),
        ] {
            let v = value.trim();
            if !v.is_empty() {
                lines.push(format!("{prop}:{}", vcard_escape(v)));
            }
        }
        lines.push("END:VCARD".to_string());
        // CRLF is what the RFC specifies. Readers tolerate bare LF, but there
        // is no reason to hand them something out of spec.
        lines.join("\r\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------------------------------------------------- links --

    #[test]
    fn explicit_schemes_are_left_alone() {
        for url in [
            "https://example.com",
            "http://example.com/a?b=c#d",
            "HTTPS://EXAMPLE.COM",
            "ftp://files.example.org",
            "mailto:someone@example.com",
            "tel:+31612345678",
            "geo:52.3676,4.9041",
        ] {
            assert_eq!(detect(url), Kind::Link, "{url} should be a link");
            assert_eq!(normalize_url(url), url, "{url} must not be rewritten");
        }
    }

    #[test]
    fn bare_domains_get_https() {
        assert_eq!(normalize_url("example.com"), "https://example.com");
        assert_eq!(normalize_url("www.example.com"), "https://www.example.com");
        assert_eq!(
            normalize_url("sub.example.co/path?q=1&r=2#frag"),
            "https://sub.example.co/path?q=1&r=2#frag"
        );
    }

    #[test]
    fn url_detection_accepts_real_addresses() {
        for s in [
            "example.com",
            "www.anything.whatever",
            "sub.example.co.uk/path",
            "example.com:8080/x",
            "example.com.",
        ] {
            assert_eq!(detect(s), Kind::Link, "{s} should be a link");
        }
    }

    #[test]
    fn url_detection_rejects_prose_and_bare_words() {
        for s in [
            "",
            "   ",
            "hello world",
            "Meet me at 8.",
            "3.14159",
            "notes.txt",
            "version 1.2",
            "just-a-word",
            "a/b://c",
        ] {
            assert_eq!(detect(s), Kind::Text, "{s:?} should be text");
        }
    }

    #[test]
    fn text_is_passed_through_byte_for_byte() {
        let snippet = "line one\n  indented\ttab\nemoji \u{1F600} and \"quotes\"";
        let p = Payload::Text {
            input: snippet.into(),
        };
        assert_eq!(p.encode(), snippet);
    }

    // ---------------------------------------------------------- wi-fi --

    fn wifi(ssid: &str, password: &str) -> Wifi {
        Wifi {
            ssid: ssid.into(),
            password: password.into(),
            security: Security::Wpa,
            hidden: false,
        }
    }

    /// Literal expected strings, taken from the format's own documentation
    /// rather than from what this code happens to produce.
    #[test]
    fn wifi_matches_the_documented_format() {
        assert_eq!(
            wifi("mynetwork", "mypass").encode(),
            "WIFI:T:WPA;S:mynetwork;P:mypass;;"
        );
    }

    #[test]
    fn wifi_escapes_every_structural_character() {
        let w = wifi("My;Net", r#"pa\ss,wo:rd"#);
        assert_eq!(w.encode(), r#"WIFI:T:WPA;S:My\;Net;P:pa\\ss\,wo\:rd;;"#);
    }

    /// Ordinary words made of hex letters are extremely common as network
    /// names, and quoting them breaks scanners that do not strip the quotes.
    #[test]
    fn everyday_ssids_that_happen_to_be_hex_are_not_quoted() {
        for ssid in ["Cafe", "Face", "Beef", "Decade", "Facade", "beef1234"] {
            let out = wifi(ssid, "flatwhite").encode();
            assert_eq!(out, format!("WIFI:T:WPA;S:{ssid};P:flatwhite;;"));
        }
    }

    #[test]
    fn a_password_at_a_real_key_length_is_quoted() {
        // 26 hex digits is a 128-bit WEP key, so the string is genuinely
        // ambiguous and the quotes settle it.
        let key = "0123456789abcdef0123456789";
        assert_eq!(key.len(), 26);
        assert_eq!(
            wifi("Office", key).encode(),
            format!("WIFI:T:WPA;S:Office;P:\"{key}\";;")
        );
    }

    #[test]
    fn a_short_hex_password_is_left_alone() {
        // No scanner reads `abc123` as a six-digit key, because there is no
        // key of that length.
        assert_eq!(
            wifi("Office", "abc123").encode(),
            "WIFI:T:WPA;S:Office;P:abc123;;"
        );
    }

    #[test]
    fn an_open_network_omits_the_password_field_entirely() {
        let w = Wifi {
            security: Security::Nopass,
            ..wifi("Guest", "ignored")
        };
        assert_eq!(w.encode(), "WIFI:T:nopass;S:Guest;;");
        assert!(!w.encode().contains("P:"));
    }

    #[test]
    fn hidden_networks_carry_the_flag() {
        let w = Wifi {
            hidden: true,
            ..wifi("Backroom", "secret")
        };
        assert_eq!(w.encode(), "WIFI:T:WPA;S:Backroom;P:secret;H:true;;");
    }

    #[test]
    fn wep_is_spelled_the_way_scanners_expect() {
        let w = Wifi {
            security: Security::Wep,
            ..wifi("Old", "key")
        };
        assert!(w.encode().starts_with("WIFI:T:WEP;"));
    }

    // -------------------------------------------------------- contact --

    #[test]
    fn contact_produces_a_valid_minimal_vcard() {
        let c = Contact {
            first_name: "Marieke".into(),
            last_name: "van Dijk".into(),
            ..Default::default()
        };
        assert_eq!(
            c.encode(),
            "BEGIN:VCARD\r\nVERSION:3.0\r\nN:van Dijk;Marieke;;;\r\nFN:Marieke van Dijk\r\nEND:VCARD"
        );
    }

    #[test]
    fn contact_includes_only_the_fields_that_were_filled() {
        let c = Contact {
            first_name: "Tomas".into(),
            last_name: "Reyes".into(),
            email: "tomas@example.com".into(),
            ..Default::default()
        };
        let out = c.encode();
        assert!(out.contains("EMAIL:tomas@example.com"));
        assert!(!out.contains("ORG:"), "empty fields must be omitted");
        assert!(!out.contains("TEL"), "empty fields must be omitted");
    }

    #[test]
    fn contact_escapes_vcard_separators() {
        let c = Contact {
            first_name: "A;B".into(),
            last_name: "C,D".into(),
            org: r"Back\Slash".into(),
            ..Default::default()
        };
        let out = c.encode();
        assert!(out.contains(r"N:C\,D;A\;B;;;"), "got {out}");
        assert!(out.contains(r"ORG:Back\\Slash"), "got {out}");
    }

    #[test]
    fn contact_folds_newlines_so_the_card_is_not_truncated() {
        let c = Contact {
            org: "Line one\r\nLine two".into(),
            ..Default::default()
        };
        let out = c.encode();
        assert!(out.contains(r"ORG:Line one\nLine two"), "got {out}");
        // Exactly one CRLF per property, so no stray blank lines.
        assert_eq!(out.matches("\r\n").count(), out.lines().count() - 1);
    }
}
