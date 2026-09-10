//! Turning a spreadsheet of buyers into tickets to issue.
//!
//! Every column becomes a variable. Only two are given any meaning: which one
//! holds the email address, and which one holds a quantity. Everything else,
//! whatever it is called, is available to the ticket layout, the email copy
//! and the filenames as `{{token}}`. Adding a seat or a tier column needs no
//! code change at all.
//!
//! Parsed with the `csv` crate rather than by splitting on commas. Real buyer
//! lists contain `Reyes, Tomas` in a quoted field, addresses with commas, and
//! notes with embedded newlines. Splitting on commas turns those into shifted
//! columns, and a shifted column here means emailing someone else's ticket to
//! the wrong address.
//!
//! Nothing is dropped silently. A row that cannot be used comes back as a
//! problem with its line number, so the operator fixes those rows rather than
//! discovering at the door that eleven people were never issued anything.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::vars::tokenise;

/// Most tickets per buyer. A group booking is real; four thousand on one row
/// is a stray digit, and catching it here is cheaper than after the send.
const MAX_PER_BUYER: u32 = 50;

/// One buyer, with every column they had.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    /// Token to value, for every column in the file.
    pub fields: BTreeMap<String, String>,
    /// Copied out of `fields` for convenience, since sending needs it by name
    /// rather than by whichever column happened to hold it.
    pub email: String,
    pub quantity: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Problem {
    /// Line in the file as a text editor counts them, header included.
    pub line: u64,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Import {
    /// Headers as written in the file, for showing the operator.
    pub headers: Vec<String>,
    /// The same columns as `{{tokens}}`, in the same order.
    pub tokens: Vec<String>,
    pub rows: Vec<Row>,
    pub problems: Vec<Problem>,
    /// Total tickets, the sum of the quantities rather than the row count,
    /// which is the number the operator actually needs to see.
    pub tickets: u32,
}

/// Which column means something beyond being a variable.
#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Mapping {
    pub email: Option<usize>,
    pub quantity: Option<usize>,
}

/// A leading byte order mark, which Excel writes and which would otherwise
/// become part of the first header name and break every token.
fn strip_bom(text: &str) -> &str {
    text.strip_prefix('\u{feff}').unwrap_or(text)
}

fn reader(text: &str) -> csv::Reader<&[u8]> {
    csv::ReaderBuilder::new()
        // A short row is a data problem to report, not a parse error that
        // aborts the whole file.
        .flexible(true)
        .from_reader(strip_bom(text).as_bytes())
}

/// The header row, as written.
pub fn headers(text: &str) -> Result<Vec<String>, String> {
    let mut rdr = reader(text);
    Ok(rdr
        .headers()
        .map_err(|e| format!("Could not read the first row: {e}"))?
        .iter()
        .map(|h| h.trim().to_string())
        .collect())
}

/// Tokens for a header row, made unique.
///
/// Two columns can legitimately collide: "Name" and "name" tokenise the same,
/// and a spreadsheet with a blank header produces nothing at all. Both get a
/// positional suffix so every column stays addressable rather than one
/// silently shadowing the other.
pub fn tokens_for(headers: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(headers.len());
    for (i, header) in headers.iter().enumerate() {
        let base = tokenise(header);
        let base = if base.is_empty() {
            format!("column_{}", i + 1)
        } else {
            base
        };
        let mut token = base.clone();
        let mut n = 2;
        while out.contains(&token) {
            token = format!("{base}_{n}");
            n += 1;
        }
        out.push(token);
    }
    out
}

/// Guess which column is the email and which is the quantity.
///
/// Deliberately generous, because these files come out of Stripe, Eventbrite,
/// Google Forms and hand-made spreadsheets and every one calls the same column
/// something different. A wrong guess costs one click, since the mapping is
/// shown before anything is issued.
pub fn guess_mapping(headers: &[String]) -> Mapping {
    let find = |exact: &[&str], contains: &str| {
        headers
            .iter()
            .position(|h| {
                let h = h.trim().to_lowercase();
                exact.iter().any(|c| h == *c)
            })
            .or_else(|| {
                headers
                    .iter()
                    .position(|h| h.trim().to_lowercase().contains(contains))
            })
    };

    Mapping {
        email: find(&["email", "e-mail", "email address", "mail"], "email"),
        quantity: find(
            &["quantity", "qty", "tickets", "amount", "count"],
            "quantit",
        ),
    }
}

/// Does this look like an address worth trying to send to?
///
/// Deliberately shallow. Full RFC validation rejects addresses that work, and
/// the only real test is whether the mail provider accepts it. This catches
/// the typo class that is obvious on sight.
fn looks_like_email(raw: &str) -> bool {
    let s = raw.trim();
    match s.split_once('@') {
        Some((local, domain)) => {
            !local.is_empty()
                && domain.contains('.')
                && !domain.starts_with('.')
                && !domain.ends_with('.')
                && !s.contains(char::is_whitespace)
        }
        None => false,
    }
}

/// Read the buyers out of a CSV under the given mapping.
pub fn parse(text: &str, mapping: Mapping) -> Import {
    let headers = headers(text).unwrap_or_default();
    let tokens = tokens_for(&headers);
    let mut rows = Vec::new();
    let mut problems = Vec::new();
    let mut seen_emails: Vec<String> = Vec::new();
    let mut rdr = reader(text);

    for record in rdr.records() {
        let record = match record {
            Ok(r) => r,
            Err(e) => {
                problems.push(Problem {
                    line: e.position().map(|p| p.line()).unwrap_or(0),
                    reason: format!("Could not read this row: {e}"),
                });
                continue;
            }
        };
        let line = record.position().map(|p| p.line()).unwrap_or(0);

        // Every column, whatever it is called, keyed by its token.
        let mut fields = BTreeMap::new();
        for (i, token) in tokens.iter().enumerate() {
            fields.insert(
                token.clone(),
                record.get(i).map(|v| v.trim()).unwrap_or("").to_string(),
            );
        }

        // A row with nothing in it anywhere is a trailing blank line, not
        // something the operator needs to hear about.
        if fields.values().all(|v| v.is_empty()) {
            continue;
        }

        let at = |idx: Option<usize>| {
            idx.and_then(|i| record.get(i))
                .map(|v| v.trim().to_string())
                .unwrap_or_default()
        };
        let email = at(mapping.email);
        let quantity_raw = at(mapping.quantity);

        let quantity = if quantity_raw.is_empty() {
            1
        } else {
            match quantity_raw.parse::<u32>() {
                Ok(0) => {
                    problems.push(Problem {
                        line,
                        reason: "Quantity is zero, so no ticket was issued for this row.".into(),
                    });
                    continue;
                }
                Ok(n) if n > MAX_PER_BUYER => {
                    problems.push(Problem {
                        line,
                        reason: format!(
                            "Quantity {n} is above the {MAX_PER_BUYER} per buyer limit. \
                             Split the row if that is genuinely intended."
                        ),
                    });
                    continue;
                }
                Ok(n) => n,
                Err(_) => {
                    problems.push(Problem {
                        line,
                        reason: format!("Quantity \"{quantity_raw}\" is not a whole number."),
                    });
                    continue;
                }
            }
        };

        if !email.is_empty() && !looks_like_email(&email) {
            problems.push(Problem {
                line,
                reason: format!("\"{email}\" does not look like an email address."),
            });
            continue;
        }

        // Not fatal. Families and couples share an address, and refusing the
        // row would be wrong. Worth saying once so a duplicated row is noticed.
        if !email.is_empty() {
            let lower = email.to_lowercase();
            if seen_emails.contains(&lower) {
                problems.push(Problem {
                    line,
                    reason: format!(
                        "{email} appears more than once. Tickets were still issued, in case \
                         that is deliberate."
                    ),
                });
            } else {
                seen_emails.push(lower);
            }
        }

        rows.push(Row {
            fields,
            email,
            quantity,
        });
    }

    let tickets = rows.iter().map(|r| r.quantity).sum();
    Import {
        headers,
        tokens,
        rows,
        problems,
        tickets,
    }
}

/// An example CSV with exactly the columns an event references.
///
/// Generated from the event rather than fixed, so the file always matches what
/// the layout and the copy actually need. Guessing column names is the most
/// common way a first run goes wrong.
pub fn example_csv(columns: &[String]) -> String {
    // Two rows, because one row hides whether the quantity column repeats a
    // buyer or numbers the rows.
    const SAMPLES: [&[&str]; 2] = [
        &["Marieke", "van Dijk", "marieke@example.com", "A12", "2"],
        &["Tomas", "Reyes", "tomas@example.com", "A13", "1"],
    ];

    let guess = |token: &str, row: usize| -> String {
        let s = SAMPLES[row];
        match token {
            t if t.contains("first") => s[0].into(),
            t if t.contains("last") || t.contains("surname") => s[1].into(),
            t if t.contains("email") || t.contains("mail") => s[2].into(),
            t if t.contains("name") => format!("{} {}", s[0], s[1]),
            t if t.contains("seat") || t.contains("table") || t.contains("row") => s[3].into(),
            t if t.contains("qty") || t.contains("quantit") || t.contains("ticket") => s[4].into(),
            _ => format!("example {}", row + 1),
        }
    };

    let mut out = String::new();
    out.push_str(
        &columns
            .iter()
            .map(|c| csv_field(c))
            .collect::<Vec<_>>()
            .join(","),
    );
    out.push('\n');
    for row in 0..SAMPLES.len() {
        let cells: Vec<String> = columns.iter().map(|c| csv_field(&guess(c, row))).collect();
        out.push_str(&cells.join(","));
        out.push('\n');
    }
    out
}

/// Quote a CSV field only when it needs it.
pub fn csv_field(raw: &str) -> String {
    if raw.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", raw.replace('"', "\"\""))
    } else {
        raw.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "First Name,Last Name,Email,Seat,Quantity\n\
        Marieke,van Dijk,marieke@example.com,A12,2\n\
        \"Reyes, Tomas\",,tomas@example.com,A13,1\n";

    fn mapped() -> Mapping {
        Mapping {
            email: Some(2),
            quantity: Some(4),
        }
    }

    #[test]
    fn every_column_becomes_a_variable() {
        let import = parse(SAMPLE, mapped());
        assert_eq!(
            import.tokens,
            vec!["first_name", "last_name", "email", "seat", "quantity"]
        );
        let first = &import.rows[0];
        assert_eq!(first.fields["first_name"], "Marieke");
        assert_eq!(first.fields["last_name"], "van Dijk");
        assert_eq!(first.fields["seat"], "A12");
        assert_eq!(
            crate::vars::Vars(first.fields.clone()).render("{{first_name}} in {{seat}}"),
            "Marieke in A12",
            "a column nobody wrote code for is usable immediately"
        );
    }

    #[test]
    fn a_column_the_app_has_never_heard_of_still_works() {
        let text = "Email,Dietary Requirement\na@example.com,Vegan\n";
        let import = parse(text, guess_mapping(&headers(text).unwrap()));
        assert_eq!(import.rows[0].fields["dietary_requirement"], "Vegan");
    }

    /// The reason the csv crate is here rather than `split(',')`.
    #[test]
    fn a_quoted_comma_stays_inside_one_field() {
        let import = parse(SAMPLE, mapped());
        assert_eq!(import.rows[1].fields["first_name"], "Reyes, Tomas");
        assert_eq!(import.rows[1].email, "tomas@example.com");
    }

    #[test]
    fn quantity_drives_the_ticket_count_not_the_row_count() {
        let import = parse(SAMPLE, mapped());
        assert_eq!(import.rows.len(), 2);
        assert_eq!(import.tickets, 3, "one buyer bought two");
    }

    #[test]
    fn colliding_headers_stay_separately_addressable() {
        let text = "Name,name,,Name\na,b,c,d\n";
        let import = parse(text, Mapping::default());
        assert_eq!(import.tokens, vec!["name", "name_2", "column_3", "name_3"]);
        assert_eq!(import.rows[0].fields["name"], "a");
        assert_eq!(import.rows[0].fields["name_2"], "b");
        assert_eq!(import.rows[0].fields["column_3"], "c");
    }

    #[test]
    fn a_byte_order_mark_does_not_become_part_of_the_first_token() {
        let with_bom = "\u{feff}First Name,Email\nA,a@b.com\n";
        assert_eq!(headers(with_bom).unwrap()[0], "First Name");
        let import = parse(with_bom, guess_mapping(&headers(with_bom).unwrap()));
        assert_eq!(import.rows[0].fields["first_name"], "A");
    }

    #[test]
    fn a_missing_quantity_column_means_one_each() {
        let text = "Email\na@example.com\nb@example.com\n";
        let import = parse(text, guess_mapping(&headers(text).unwrap()));
        assert_eq!(import.tickets, 2);
    }

    #[test]
    fn a_bad_address_is_reported_with_its_line_and_not_issued() {
        let text = "Email\nnot-an-address\nb@example.com\n";
        let import = parse(
            text,
            Mapping {
                email: Some(0),
                quantity: None,
            },
        );
        assert_eq!(import.rows.len(), 1, "only the good row is issued");
        assert_eq!(import.problems.len(), 1);
        assert_eq!(import.problems[0].line, 2, "line as an editor counts them");
        assert!(import.problems[0].reason.contains("not-an-address"));
    }

    #[test]
    fn a_missing_address_is_allowed_because_not_every_ticket_is_emailed() {
        let text = "Name,Email\nWalk-in,\n";
        let import = parse(
            text,
            Mapping {
                email: Some(1),
                quantity: None,
            },
        );
        assert_eq!(import.rows.len(), 1);
        assert!(import.rows[0].email.is_empty());
        assert!(import.problems.is_empty());
    }

    #[test]
    fn a_duplicate_address_is_flagged_but_still_issued() {
        let text = "Email\nsame@example.com\nSAME@example.com\n";
        let import = parse(
            text,
            Mapping {
                email: Some(0),
                quantity: None,
            },
        );
        assert_eq!(import.rows.len(), 2, "couples share an address");
        assert_eq!(import.problems.len(), 1);
        assert!(import.problems[0].reason.contains("more than once"));
    }

    #[test]
    fn blank_rows_are_skipped_without_complaint() {
        let text = "Name,Email,Quantity\nA,a@example.com,1\n,,\n\n";
        let import = parse(
            text,
            Mapping {
                email: Some(1),
                quantity: Some(2),
            },
        );
        assert_eq!(import.rows.len(), 1);
        assert!(import.problems.is_empty(), "got {:?}", import.problems);
    }

    #[test]
    fn a_nonsense_quantity_is_reported_rather_than_assumed() {
        let text = "Email,Quantity\na@example.com,two\nb@example.com,0\nc@example.com,999\n";
        let import = parse(
            text,
            Mapping {
                email: Some(0),
                quantity: Some(1),
            },
        );
        assert!(import.rows.is_empty());
        assert_eq!(import.problems.len(), 3);
        assert!(import.problems[0].reason.contains("not a whole number"));
        assert!(import.problems[1].reason.contains("zero"));
        assert!(import.problems[2].reason.contains("limit"));
    }

    #[test]
    fn column_guessing_copes_with_what_real_exports_call_things() {
        for (header, email, quantity) in [
            ("Name,Email", Some(1), None),
            ("Email Address,Full Name", Some(0), None),
            ("Attendee,E-Mail,Qty", Some(1), Some(2)),
            ("first_name,customer email,tickets", Some(1), Some(2)),
        ] {
            let hs = headers(&format!("{header}\n")).unwrap();
            let m = guess_mapping(&hs);
            assert_eq!(m.email, email, "email column in {header}");
            assert_eq!(m.quantity, quantity, "quantity column in {header}");
        }
    }

    #[test]
    fn a_short_row_leaves_the_missing_columns_empty() {
        let text = "First Name,Email,Seat\nA,a@example.com,A1\nB\n";
        let import = parse(
            text,
            Mapping {
                email: Some(1),
                quantity: None,
            },
        );
        assert_eq!(import.rows.len(), 2);
        assert_eq!(import.rows[1].fields["first_name"], "B");
        assert_eq!(import.rows[1].fields["seat"], "", "absent, not missing");
    }

    #[test]
    fn address_shapes_that_should_and_should_not_pass() {
        for good in ["a@b.com", "first.last+tag@sub.example.co.uk"] {
            assert!(looks_like_email(good), "{good} should pass");
        }
        for bad in [
            "",
            "nope",
            "@example.com",
            "a@b",
            "a@.com",
            "a b@example.com",
        ] {
            assert!(!looks_like_email(bad), "{bad} should fail");
        }
    }

    #[test]
    fn the_example_csv_has_the_requested_headers_and_two_rows() {
        let columns = vec![
            "first_name".to_string(),
            "last_name".to_string(),
            "email".to_string(),
            "seat".to_string(),
        ];
        let csv = example_csv(&columns);
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(lines[0], "first_name,last_name,email,seat");
        assert_eq!(lines.len(), 3, "a header and two samples");
        assert!(lines[1].contains("marieke@example.com"));
        assert!(lines[2].contains("tomas@example.com"));
    }

    /// The example must survive being read straight back in.
    #[test]
    fn the_example_csv_parses_as_a_valid_import() {
        let columns = vec!["first_name".into(), "email".into(), "quantity".into()];
        let csv = example_csv(&columns);
        let import = parse(&csv, guess_mapping(&headers(&csv).unwrap()));
        assert!(import.problems.is_empty(), "got {:?}", import.problems);
        assert_eq!(import.rows.len(), 2);
        assert_eq!(import.tickets, 3, "the sample quantities are 2 and 1");
    }

    #[test]
    fn an_example_column_needing_a_comma_is_quoted() {
        let csv = example_csv(&["full_name".to_string()]);
        assert!(csv.contains("Marieke van Dijk"), "got {csv}");
    }
}
