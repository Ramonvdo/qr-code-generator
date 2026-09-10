//! The one place a `{{variable}}` becomes text.
//!
//! Every template in the app runs through here: what a ticket prints, the
//! email subject and body, and the filename each PDF is saved under. One
//! engine means adding a column to a CSV makes it available everywhere at
//! once, and it means there is a single answer to "what can I write here".
//!
//! Deliberately not a template language. There are no conditionals, no loops
//! and no filters, because each of those would need escaping rules and an
//! error surface of its own, and a ticket does not need them.

use serde::Serialize;
use std::collections::BTreeMap;

/// Turn a spreadsheet header into the token used inside `{{ }}`.
///
/// "First Name" and "first_name" and "FIRST NAME" all collapse to the same
/// token, because a buyer list exported from one system and a template written
/// against another should still line up. The original header is kept
/// separately for display, so nothing the operator sees is renamed.
pub fn tokenise(header: &str) -> String {
    let mut out = String::with_capacity(header.len());
    let mut last_underscore = true; // suppresses a leading underscore
    for c in header.trim().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            last_underscore = false;
        } else if !last_underscore {
            out.push('_');
            last_underscore = true;
        }
    }
    out.trim_end_matches('_').to_string()
}

/// The values one ticket is rendered with.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(transparent)]
pub struct Vars(pub BTreeMap<String, String>);

impl Vars {
    pub fn set(&mut self, token: impl Into<String>, value: impl Into<String>) -> &mut Self {
        self.0.insert(token.into(), value.into());
        self
    }

    pub fn get(&self, token: &str) -> Option<&str> {
        self.0.get(token).map(|s| s.as_str())
    }

    /// Fill every `{{token}}` this knows about.
    ///
    /// A single pass over the input, so a value that itself contains `{{...}}`
    /// is inserted literally rather than expanded again. Someone whose company
    /// name is `{{Braces}} Ltd` gets their name on the ticket, not a second
    /// round of substitution.
    ///
    /// An unknown variable is left standing rather than blanked, because a
    /// visible `{{sest}}` on a proof is a typo you can see and fix, while an
    /// empty gap is one you cannot.
    pub fn render(&self, template: &str) -> String {
        let mut out = String::with_capacity(template.len());
        let mut rest = template;
        while let Some(start) = rest.find("{{") {
            out.push_str(&rest[..start]);
            let after = &rest[start + 2..];
            match after.find("}}") {
                Some(end) => {
                    let token = after[..end].trim();
                    match self.0.get(token) {
                        Some(value) => out.push_str(value),
                        None => {
                            out.push_str("{{");
                            out.push_str(&after[..end]);
                            out.push_str("}}");
                        }
                    }
                    rest = &after[end + 2..];
                }
                // An unclosed brace is literal text, not an error.
                None => {
                    out.push_str("{{");
                    rest = after;
                    break;
                }
            }
        }
        out.push_str(rest);
        out
    }
}

/// Every variable a template mentions, in order of first appearance.
///
/// This is what lets an event say which columns it needs: the same scan runs
/// across every element, the subject, the body and the filename, and the union
/// is exactly the header row of the example CSV it hands you.
pub fn referenced(template: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else { break };
        let token = after[..end].trim();
        if !token.is_empty() && !found.iter().any(|t| t == token) {
            found.push(token.to_string());
        }
        rest = &after[end + 2..];
    }
    found
}

/// Variables the app supplies itself, which a CSV therefore need not carry.
pub const BUILT_IN: [&str; 6] = [
    "serial",
    "serial_plain",
    "ticket_code",
    "event",
    "issued_at",
    "quantity",
];

pub fn is_built_in(token: &str) -> bool {
    BUILT_IN.contains(&token)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars() -> Vars {
        let mut v = Vars::default();
        v.set("first_name", "Marieke")
            .set("last_name", "van Dijk")
            .set("seat", "A12")
            .set("serial", "7");
        v
    }

    #[test]
    fn headers_become_predictable_tokens() {
        for (header, token) in [
            ("First Name", "first_name"),
            ("first_name", "first_name"),
            ("FIRST NAME", "first_name"),
            ("  Email Address  ", "email_address"),
            ("Seat #", "seat"),
            ("Ticket-Type", "ticket_type"),
            ("Qty.", "qty"),
            ("naïve café", "na_ve_caf"),
        ] {
            assert_eq!(tokenise(header), token, "for {header:?}");
        }
    }

    #[test]
    fn a_header_of_only_punctuation_tokenises_to_nothing() {
        assert_eq!(tokenise("###"), "");
        assert_eq!(tokenise(""), "");
    }

    #[test]
    fn known_variables_are_filled() {
        assert_eq!(
            vars().render("{{first_name}} {{last_name}}, seat {{seat}}"),
            "Marieke van Dijk, seat A12"
        );
    }

    #[test]
    fn the_same_variable_twice_is_filled_twice() {
        assert_eq!(vars().render("{{seat}}/{{seat}}"), "A12/A12");
    }

    #[test]
    fn surrounding_space_inside_the_braces_is_ignored() {
        assert_eq!(vars().render("{{ seat }}"), "A12");
    }

    /// A blank gap on a proof is a mistake you cannot see. A visible token is.
    #[test]
    fn an_unknown_variable_stays_visible_rather_than_vanishing() {
        assert_eq!(vars().render("seat {{sest}}"), "seat {{sest}}");
    }

    /// One pass only. A value containing braces is data, not a template.
    #[test]
    fn a_value_containing_braces_is_not_expanded_again() {
        let mut v = Vars::default();
        v.set("company", "{{seat}} Ltd").set("seat", "A12");
        assert_eq!(v.render("{{company}}"), "{{seat}} Ltd");
    }

    #[test]
    fn text_without_variables_passes_through_untouched() {
        let text = "Doors 19:00. No re-entry.";
        assert_eq!(vars().render(text), text);
    }

    #[test]
    fn an_unclosed_brace_is_treated_as_text() {
        assert_eq!(vars().render("50{{ off"), "50{{ off");
        assert_eq!(vars().render("a {{b"), "a {{b");
    }

    #[test]
    fn an_empty_template_renders_empty() {
        assert_eq!(vars().render(""), "");
    }

    #[test]
    fn referenced_lists_each_variable_once_in_order() {
        assert_eq!(
            referenced("{{last_name}}, {{first_name}} in {{seat}}. {{first_name}} again."),
            vec!["last_name", "first_name", "seat"]
        );
    }

    #[test]
    fn referenced_ignores_text_and_empty_braces() {
        assert!(referenced("no variables here").is_empty());
        assert!(referenced("{{}} {{   }}").is_empty());
        assert!(referenced("{{unclosed").is_empty());
    }

    #[test]
    fn referenced_trims_the_way_render_does() {
        assert_eq!(referenced("{{ seat }}"), vec!["seat"]);
    }

    /// The columns an event needs are what it references minus what the app
    /// supplies, so the two lists have to agree on the names.
    #[test]
    fn built_ins_are_recognised_by_the_names_render_uses() {
        for token in BUILT_IN {
            assert!(is_built_in(token));
            let mut v = Vars::default();
            v.set(token, "x");
            assert_eq!(v.render(&format!("{{{{{token}}}}}")), "x");
        }
        assert!(!is_built_in("first_name"));
    }
}
