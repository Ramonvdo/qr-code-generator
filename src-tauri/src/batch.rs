//! Many codes in one pass, from a pasted list.
//!
//! The list is deliberately a plain textarea rather than a CSV importer with
//! column mapping. Selecting a column in a spreadsheet and pasting it produces
//! exactly one value per line, which is what people actually do, and it means
//! there is no delimiter guessing, no encoding detection and no header row to
//! get wrong.

use serde::{Deserialize, Serialize};

/// Upper bound on one batch.
///
/// Not a technical limit. It is the point past which a mistyped paste stops
/// looking like a mistake and starts looking like a filled folder, so the run
/// is refused while it is still easy to undo.
pub const MAX_ITEMS: usize = 5_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Naming {
    /// Derive each filename from what the code contains.
    Content,
    /// Number the files in input order, zero padded.
    Numbered,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Failure {
    /// 1-based, matching what the user sees in the textarea.
    pub line: usize,
    pub content: String,
    pub reason: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Outcome {
    pub written: usize,
    /// Every line that produced no file, with the reason. Reported in full
    /// rather than as a count: a batch that silently drops nine of a thousand
    /// rows is worse than one that refuses to run.
    pub failures: Vec<Failure>,
    /// Where the files went, so the UI can offer to open it.
    pub directory: String,
}

/// Split pasted text into the entries to encode.
///
/// Blank lines are dropped rather than treated as empty codes, because a
/// trailing newline is present in essentially every paste. Surrounding
/// whitespace goes too: a spreadsheet column often arrives padded, and a
/// trailing space would silently change the encoded URL.
pub fn parse_lines(text: &str) -> Vec<(usize, String)> {
    text.lines()
        .enumerate()
        .map(|(i, l)| (i + 1, l.trim().to_string()))
        .filter(|(_, l)| !l.is_empty())
        .collect()
}

/// Zero-padded width for numbered output, so files sort correctly.
///
/// A folder of `1.svg, 2.svg, ... 10.svg` sorts as 1, 10, 2 in every file
/// manager and every print queue, which ruins a numbered ticket run.
pub fn number_width(count: usize) -> usize {
    count.to_string().len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_and_padded_lines_are_cleaned_up() {
        let text = "  https://example.com  \n\n\thttps://example.org\n   \n";
        let items = parse_lines(text);
        assert_eq!(
            items,
            vec![
                (1, "https://example.com".to_string()),
                (3, "https://example.org".to_string()),
            ]
        );
    }

    /// The reported line number has to match the textarea, or a failure list
    /// pointing at the wrong row is worse than no failure list.
    #[test]
    fn line_numbers_survive_blank_lines() {
        let items = parse_lines("a\n\n\nb");
        assert_eq!(items[0].0, 1);
        assert_eq!(items[1].0, 4);
    }

    #[test]
    fn empty_input_produces_no_items() {
        assert!(parse_lines("").is_empty());
        assert!(parse_lines("\n\n  \n").is_empty());
    }

    #[test]
    fn numbering_is_wide_enough_to_sort() {
        assert_eq!(number_width(9), 1);
        assert_eq!(number_width(10), 2);
        assert_eq!(number_width(999), 3);
        assert_eq!(number_width(1000), 4);
        // 001 before 010 before 100, which is the whole point.
        let w = number_width(100);
        let mut names: Vec<String> = [1, 10, 100].iter().map(|n| format!("{n:0w$}")).collect();
        names.sort();
        assert_eq!(names, vec!["001", "010", "100"]);
    }
}
