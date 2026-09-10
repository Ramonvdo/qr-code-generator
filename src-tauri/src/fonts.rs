//! How wide a string is, so text can actually be centred.
//!
//! PDF has no concept of alignment. It draws text from a point, rightwards.
//! Centring means knowing the string's width and subtracting half of it, and
//! knowing the width means having the font's metrics. Without them, a centred
//! name is only centred for names of one particular length, which on a run of
//! tickets means every one of them is crooked by a different amount.
//!
//! These are the Adobe Core 14 metrics, in units of 1/1000 em, for the three
//! fonts the editor offers. All three are built into every PDF reader, so
//! nothing is embedded in the exported files.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Font {
    #[default]
    Helvetica,
    HelveticaBold,
    Courier,
}

impl Font {
    /// The BaseFont name written into the PDF font dictionary.
    pub fn base_name(self) -> &'static str {
        match self {
            Font::Helvetica => "Helvetica",
            Font::HelveticaBold => "Helvetica-Bold",
            Font::Courier => "Courier",
        }
    }

    /// The resource name this font is referenced by in a content stream.
    pub fn resource(self) -> &'static str {
        match self {
            Font::Helvetica => "QRF",
            Font::HelveticaBold => "QRFB",
            Font::Courier => "QRFC",
        }
    }

    pub fn all() -> [Font; 3] {
        [Font::Helvetica, Font::HelveticaBold, Font::Courier]
    }
}

/// Widths for codepoints 32 to 126, the range every ticket is mostly made of.
#[rustfmt::skip]
const HELVETICA: [u16; 95] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278,
    556, 556, 556, 556, 556, 556, 556, 556, 556, 556, 278, 278, 584, 584, 584, 556,
    1015, 667, 667, 722, 722, 667, 611, 778, 722, 278, 500, 667, 556, 833, 722, 778,
    667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 278, 278, 278, 469, 556,
    333, 556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500, 222, 833, 556, 556,
    556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500, 334, 260, 334, 584,
];

#[rustfmt::skip]
const HELVETICA_BOLD: [u16; 95] = [
    278, 333, 474, 556, 556, 889, 722, 238, 333, 333, 389, 584, 278, 333, 278, 278,
    556, 556, 556, 556, 556, 556, 556, 556, 556, 556, 333, 333, 584, 584, 584, 611,
    975, 722, 722, 722, 722, 667, 611, 778, 722, 278, 556, 722, 611, 833, 722, 778,
    667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 333, 278, 333, 584, 556,
    333, 556, 611, 556, 611, 556, 333, 611, 611, 278, 278, 556, 278, 889, 611, 611,
    611, 611, 389, 556, 333, 611, 556, 778, 556, 556, 500, 389, 280, 389, 584,
];

/// Courier is monospaced: every glyph is the same width, which is the whole
/// point of it.
const COURIER_WIDTH: u16 = 600;

/// Non-ASCII glyphs whose width differs from the letter they are built on.
///
/// Accented letters in these fonts are exactly as wide as their base letter,
/// which is why `é` falls through to `e` below rather than needing an entry.
/// These are the ones that genuinely differ.
fn winansi_special(c: char, bold: bool) -> Option<u16> {
    Some(match c {
        // The handful whose bold cut is genuinely a different width. Guarded
        // arms have to come first, or the plain ones below would swallow them.
        '\u{2018}' | '\u{2019}' | '\u{201a}' if bold => 278,
        '\u{201c}' | '\u{201d}' | '\u{201e}' if bold => 500,
        'µ' | 'ø' | 'þ' if bold => 611,
        '¶' if bold => 556,

        '\u{a0}' => 278, // no-break space
        '·' => 278,
        // The typographic quotes, which are what a spreadsheet or a word
        // processor actually puts in a line of copy.
        '\u{2018}' | '\u{2019}' | '\u{201a}' => 222,
        '¡' | '¨' | '¯' | '´' | '¸' | '¹' | '²' | '³' => 333,
        'ˆ' | '˜' | '‹' | '›' => 333,
        '\u{201c}' | '\u{201d}' | '\u{201e}' => 333,
        '•' => 350,
        'º' => 365,
        'ª' => 370,
        '°' => 400,
        '¶' => 537,
        '¢' | '£' | '¤' | '¥' | '§' | 'µ' | 'ø' | 'þ' => 556,
        '«' | '»' | '€' | '–' | 'ƒ' | '†' | '‡' => 556,
        '¬' | '±' | '×' | '÷' => 584,
        '¿' | 'ß' => 611,
        'Þ' => 667,
        '©' | '®' => 737,
        'Ø' => 778,
        '¼' | '½' | '¾' => 834,
        'æ' => 889,
        'œ' => 944,
        'Æ' | 'Œ' | '—' | '…' | '‰' | '™' => 1000,
        _ => return None,
    })
}

/// The WinAnsiEncoding byte for a character, if it has one.
///
/// PDF string literals are bytes, not text. Writing a `String` straight into
/// one puts UTF-8 in a stream the reader decodes as WinAnsi, which is how `é`
/// becomes `Ã©` on a printed ticket. This is the mapping that has to be used
/// instead, and it deliberately covers exactly the characters
/// [`winansi_special`] and [`base_letter`] can measure: anything writable must
/// also be measurable, or alignment drifts on the names that contain it.
pub fn winansi(c: char) -> Option<u8> {
    let code = c as u32;
    // The printable ASCII range is identical in both.
    if (0x20..=0x7e).contains(&code) {
        return Some(code as u8);
    }
    // Latin-1 above the control block, likewise identical.
    if (0xa0..=0xff).contains(&code) {
        return Some(code as u8);
    }
    // Then the block WinAnsi fills with punctuation where Latin-1 has
    // controls, which is where the curly quotes and the dashes live.
    Some(match c {
        '€' => 0x80,
        '\u{201a}' => 0x82,
        'ƒ' => 0x83,
        '\u{201e}' => 0x84,
        '…' => 0x85,
        '†' => 0x86,
        '‡' => 0x87,
        'ˆ' => 0x88,
        '‰' => 0x89,
        'Š' => 0x8a,
        '‹' => 0x8b,
        'Œ' => 0x8c,
        'Ž' => 0x8e,
        '\u{2018}' => 0x91,
        '\u{2019}' => 0x92,
        '\u{201c}' => 0x93,
        '\u{201d}' => 0x94,
        '•' => 0x95,
        '–' => 0x96,
        '—' => 0x97,
        '˜' => 0x98,
        '™' => 0x99,
        'š' => 0x9a,
        '›' => 0x9b,
        'œ' => 0x9c,
        'ž' => 0x9e,
        'Ÿ' => 0x9f,
        _ => return None,
    })
}

/// Strip a glyph down to the one it is drawn over.
///
/// Sound rather than a guess: in these fonts an accented glyph occupies
/// exactly the width of its base letter, so `café` measures as `cafe`. The
/// four non-letters here are the same case: a broken bar is a bar, a soft
/// hyphen is a hyphen, and eth is drawn on a d.
fn base_letter(c: char) -> Option<char> {
    Some(match c {
        '¦' => '|',
        '\u{ad}' => '-', // soft hyphen
        'Ð' => 'D',
        'ð' => 'd',
        'À'..='Å' => 'A',
        'Ç' => 'C',
        'È'..='Ë' => 'E',
        'Ì'..='Ï' => 'I',
        'Ñ' => 'N',
        'Ò'..='Ö' => 'O',
        'Š' => 'S',
        'Ù'..='Ü' => 'U',
        'Ý' | 'Ÿ' => 'Y',
        'Ž' => 'Z',
        'à'..='å' => 'a',
        'ç' => 'c',
        'è'..='ë' => 'e',
        'ì'..='ï' => 'i',
        'ñ' => 'n',
        'ò'..='ö' => 'o',
        'š' => 's',
        'ù'..='ü' => 'u',
        'ý' | 'ÿ' => 'y',
        'ž' => 'z',
        _ => return None,
    })
}

/// Width of one character, or `None` when this font has no metric for it.
fn known_width(font: Font, c: char) -> Option<u16> {
    if font == Font::Courier {
        // Uniform, but still only over the characters that can be written at
        // all, so the two halves of this module agree about what is drawable.
        return winansi(c).map(|_| COURIER_WIDTH);
    }
    let bold = font == Font::HelveticaBold;
    let table = if bold { &HELVETICA_BOLD } else { &HELVETICA };

    let ascii = |c: char| -> Option<u16> {
        let code = c as u32;
        (32..=126)
            .contains(&code)
            .then(|| table[(code - 32) as usize])
    };

    ascii(c)
        .or_else(|| winansi_special(c, bold))
        .or_else(|| base_letter(c).and_then(ascii))
}

/// Width of one character, in 1/1000 em.
fn char_width(font: Font, c: char) -> u16 {
    // Anything with no metric is drawn as `?` by the text escaper, so it has
    // to measure as `?` too, or alignment would drift on the odd character.
    known_width(font, c).unwrap_or_else(|| known_width(font, '?').unwrap_or(556))
}

/// Width of a string at a given point size, in points.
pub fn width(font: Font, text: &str, points: f32) -> f32 {
    let units: u32 = text.chars().map(|c| char_width(font, c) as u32).sum();
    units as f32 * points / 1000.0
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Align {
    #[default]
    Left,
    Centre,
    Right,
}

impl Align {
    /// How far left of the anchor the text starts.
    ///
    /// PDF always draws rightwards from where you put the cursor, so alignment
    /// is entirely a matter of moving that cursor before drawing.
    pub fn offset(self, text_width: f32) -> f32 {
        match self {
            Align::Left => 0.0,
            Align::Centre => text_width / 2.0,
            Align::Right => text_width,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Checked against the published Adobe Helvetica metrics, not against
    /// whatever this table happens to contain. A wrong width is invisible
    /// until a centred name sits crooked on a printed ticket.
    #[test]
    fn helvetica_matches_the_published_metrics() {
        for (c, expected) in [
            (' ', 278),
            ('i', 222),
            ('l', 222),
            ('M', 833),
            ('W', 944),
            ('m', 833),
            ('%', 889),
            ('0', 556),
            ('9', 556),
            ('A', 667),
            ('a', 556),
            ('@', 1015),
        ] {
            assert_eq!(char_width(Font::Helvetica, c), expected, "for {c:?}");
        }
    }

    #[test]
    fn helvetica_bold_matches_the_published_metrics() {
        for (c, expected) in [
            (' ', 278),
            ('i', 278),
            ('M', 833),
            ('W', 944),
            ('a', 556),
            ('b', 611),
            ('!', 333),
        ] {
            assert_eq!(char_width(Font::HelveticaBold, c), expected, "for {c:?}");
        }
    }

    /// A sanity check on the two tables, restricted to letters and digits.
    ///
    /// Not universal: Helvetica's at-sign is 1015 and Helvetica-Bold's is 975,
    /// because the bold one is drawn tighter. That is genuinely how Adobe
    /// published them, and asserting otherwise across the whole range was this
    /// test being wrong rather than the data.
    #[test]
    fn bold_letters_and_digits_are_never_narrower_than_regular() {
        for code in 32u32..=126 {
            let c = char::from_u32(code).unwrap();
            if !c.is_ascii_alphanumeric() {
                continue;
            }
            assert!(
                char_width(Font::HelveticaBold, c) >= char_width(Font::Helvetica, c),
                "bold {c:?} is narrower than regular, which means a table is wrong"
            );
        }
    }

    /// The documented exception, pinned so nobody "fixes" it later.
    #[test]
    fn the_bold_at_sign_is_narrower_and_that_is_correct() {
        assert_eq!(char_width(Font::Helvetica, char::from(64)), 1015);
        assert_eq!(char_width(Font::HelveticaBold, char::from(64)), 975);
    }

    #[test]
    fn courier_is_monospaced() {
        let widths: Vec<u16> = "iMW %"
            .chars()
            .map(|c| char_width(Font::Courier, c))
            .collect();
        assert!(widths.iter().all(|w| *w == 600), "got {widths:?}");
    }

    /// The digits have to be equal width or a run of serial numbers would not
    /// line up when centred.
    #[test]
    fn digits_are_the_same_width_in_every_font() {
        for font in Font::all() {
            let widths: Vec<u16> = "0123456789".chars().map(|c| char_width(font, c)).collect();
            assert!(
                widths.windows(2).all(|w| w[0] == w[1]),
                "{font:?} digits vary: {widths:?}"
            );
        }
    }

    /// An accented letter occupies its base letter's width in these fonts, so
    /// a name with an accent must measure the same as one without.
    #[test]
    fn an_accent_does_not_change_the_width() {
        assert_eq!(
            width(Font::Helvetica, "café", 12.0),
            width(Font::Helvetica, "cafe", 12.0)
        );
        assert_eq!(
            char_width(Font::Helvetica, 'Ü'),
            char_width(Font::Helvetica, 'U')
        );
    }

    /// Unsupported characters are drawn as `?`, so they must measure as `?`
    /// or a name with one in it would come out off centre.
    #[test]
    fn an_unsupported_character_measures_as_the_glyph_it_becomes() {
        assert_eq!(
            char_width(Font::Helvetica, '中'),
            char_width(Font::Helvetica, '?')
        );
    }

    /// Worked by hand: "Hi" is 722 + 222 = 944 units, which at 12pt is
    /// 944 * 12 / 1000 = 11.328 points.
    #[test]
    fn width_scales_with_point_size() {
        let w = width(Font::Helvetica, "Hi", 12.0);
        assert!((w - 11.328).abs() < 0.001, "got {w}");
        assert!(
            (width(Font::Helvetica, "Hi", 24.0) - w * 2.0).abs() < 0.001,
            "doubling the size doubles the width"
        );
    }

    /// The non-ASCII widths, checked against Adobe's own Helvetica.afm and
    /// Helvetica-Bold.afm rather than against this table.
    ///
    /// Worth pinning separately: URW's Nimbus Sans, which is the metric clone
    /// most sources actually serve, disagrees with Adobe on two of these
    /// (`currency` 278 and `mu` 576). PDF readers use Adobe's, so these do.
    #[test]
    fn the_non_ascii_widths_match_adobes_metrics() {
        for (c, regular, bold) in [
            ('¤', 556u16, 556u16),
            ('¦', 260, 280),
            ('§', 556, 556),
            ('¨', 333, 333),
            ('ª', 370, 370),
            ('¬', 584, 584),
            ('\u{ad}', 333, 333),
            ('²', 333, 333),
            ('µ', 556, 611),
            ('¶', 537, 556),
            ('º', 365, 365),
            ('½', 834, 834),
            ('Ð', 722, 722),
            ('ð', 556, 611),
            ('•', 350, 350),
            ('\u{2018}', 222, 278),
            ('\u{201c}', 333, 500),
            ('†', 556, 556),
            ('‰', 1000, 1000),
            ('™', 1000, 1000),
            ('š', 500, 556),
            ('Ž', 611, 611),
        ] {
            assert_eq!(char_width(Font::Helvetica, c), regular, "regular {c:?}");
            assert_eq!(char_width(Font::HelveticaBold, c), bold, "bold {c:?}");
        }
    }

    /// Checked against the WinAnsiEncoding table in the PDF specification,
    /// Annex D, not against this code. A wrong byte prints a different letter.
    #[test]
    fn winansi_matches_the_published_encoding() {
        for (c, expected) in [
            (' ', 0x20u8),
            ('A', 0x41),
            ('~', 0x7e),
            ('\u{a0}', 0xa0),
            ('·', 0xb7),
            ('é', 0xe9),
            ('ü', 0xfc),
            ('ÿ', 0xff),
            ('€', 0x80),
            ('…', 0x85),
            ('Œ', 0x8c),
            ('\u{2018}', 0x91),
            ('\u{2019}', 0x92),
            ('\u{201c}', 0x93),
            ('•', 0x95),
            ('–', 0x96),
            ('—', 0x97),
            ('™', 0x99),
            ('œ', 0x9c),
            ('Ÿ', 0x9f),
        ] {
            assert_eq!(winansi(c), Some(expected), "for {c:?}");
        }
    }

    /// The gaps in the encoding, which have to stay gaps: writing a byte for
    /// one of these would print whatever the reader happens to have there.
    #[test]
    fn characters_outside_the_encoding_have_no_byte() {
        for c in [
            '中',
            '→',
            '\u{81}',
            '\u{8d}',
            '\u{9d}',
            '\u{7f}',
            '\u{1f600}',
        ] {
            assert_eq!(winansi(c), None, "for {c:?}");
        }
    }

    /// The invariant that keeps the two halves of this module honest: a
    /// character that can be written must also be measurable, and one that
    /// cannot be written must fall back to the same `?` in both.
    ///
    /// Without this, a name could be drawn correctly and measured as something
    /// else, which shows up as a centred line sitting crooked.
    #[test]
    fn everything_writable_is_measurable_and_the_reverse() {
        for font in Font::all() {
            for code in 0u32..0x2200 {
                let Some(c) = char::from_u32(code) else {
                    continue;
                };
                assert_eq!(
                    winansi(c).is_some(),
                    known_width(font, c).is_some(),
                    "{font:?} disagrees about {c:?} (U+{code:04X})"
                );
            }
        }
    }

    #[test]
    fn an_empty_string_has_no_width() {
        assert_eq!(width(Font::Helvetica, "", 12.0), 0.0);
    }

    #[test]
    fn alignment_shifts_the_cursor_by_the_expected_amount() {
        assert_eq!(Align::Left.offset(100.0), 0.0);
        assert_eq!(Align::Centre.offset(100.0), 50.0);
        assert_eq!(Align::Right.offset(100.0), 100.0);
    }

    /// Two names of different lengths, centred on the same point, must have
    /// their midpoints in the same place. This is the property the whole
    /// module exists for.
    #[test]
    fn two_different_names_centre_on_the_same_point() {
        let anchor = 300.0;
        for name in ["Bo", "Marieke van Dijk", "A"] {
            let w = width(Font::Helvetica, name, 14.0);
            let start = anchor - Align::Centre.offset(w);
            let middle = start + w / 2.0;
            assert!(
                (middle - anchor).abs() < 0.001,
                "{name} centred at {middle}"
            );
        }
    }
}
