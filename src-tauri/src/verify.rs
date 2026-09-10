//! Proves the code actually scans before the user saves it.
//!
//! Most generators will happily hand you an unscannable QR: pick a pale
//! foreground, or a background barely darker than it, and you get a clean
//! looking image that no phone can read. The failure only shows up after the
//! stickers are printed.
//!
//! So every render is read back here. Two independent signals are combined,
//! because neither is sufficient alone:
//!
//! 1. A decode by `rqrr`, which is a different implementation from the
//!    `qrcode` encoder. Agreement between them is real evidence.
//! 2. A WCAG contrast ratio. `rqrr` binarises adaptively on a synthetic,
//!    perfectly sharp raster, so it is considerably more forgiving than a
//!    phone camera pointed at a print. A code can decode here and still fail
//!    in the world, and contrast is what catches that.

use crate::render::{self, Logo, Luma, Qr, Style};
use serde::Serialize;

/// Below this ratio, treat the code as broken rather than merely risky.
const CONTRAST_BAD: f32 = 2.0;

/// Below this, the code usually still decodes on a good scanner in good light
/// and fails on a cheap one in bad light. That is worth a warning, not a stop.
const CONTRAST_RISKY: f32 = 3.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Level {
    Good,
    Risky,
    Bad,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Verdict {
    pub level: Level,
    /// Whether a decoder read the exact payload back out of our own raster.
    pub decoded: bool,
    pub contrast: f32,
    /// Plain sentences for the UI. Ordered most severe first.
    pub notes: Vec<String>,
}

/// Decode a greyscale raster, returning the payload if exactly one code reads.
///
/// More than one grid means the raster is ambiguous, which for our own
/// single-code output means something is wrong with it.
pub fn decode_luma(luma: &Luma) -> Option<String> {
    let (w, h) = (luma.width, luma.height);
    let mut prepared =
        rqrr::PreparedImage::prepare_from_greyscale(w, h, |x, y| luma.pixels[y * w + x]);
    let grids = prepared.detect_grids();
    if grids.len() != 1 {
        return None;
    }
    grids[0].decode().ok().map(|(_meta, content)| content)
}

/// Read the rendered code back and judge whether it will scan in the world.
pub fn check(qr: &Qr, style: &Style, logo: Option<&Logo>, expected: &str) -> Verdict {
    let contrast = render::contrast_ratio(style.dark, style.light);
    let inverted = style.dark.luma() > style.light.luma();

    let mut probe = render::to_luma(qr, style, logo);
    if inverted {
        // rqrr refuses inverted codes outright, but plenty of real scanners
        // (every recent phone camera among them) read them fine. Without this
        // flip, a perfectly good white-on-black code would be reported as
        // structurally broken, which is both wrong and unhelpful.
        //
        // Inverting the values rather than re-rendering in black and white
        // keeps the actual contrast magnitude intact, so a washed-out palette
        // still fails the decode instead of being quietly normalised.
        for p in probe.pixels.iter_mut() {
            *p = 255 - *p;
        }
    }
    let decoded = decode_luma(&probe).as_deref() == Some(expected);

    let mut notes = Vec::new();
    let mut level = Level::Good;

    if !decoded {
        level = Level::Bad;
        notes.push("This code did not read back correctly. Do not use it.".into());
    }

    if contrast < CONTRAST_BAD {
        level = Level::Bad;
        notes.push(format!(
            "The two colours are almost identical ({contrast:.1}:1). Scanners cannot separate the modules."
        ));
    } else if contrast < CONTRAST_RISKY {
        if level == Level::Good {
            level = Level::Risky;
        }
        notes.push(format!(
            "Low contrast ({contrast:.1}:1). This may fail on cheaper scanners or in poor light."
        ));
    }

    if inverted {
        if level == Level::Good {
            level = Level::Risky;
        }
        notes.push(
            "The foreground is lighter than the background. Many scanners refuse inverted codes."
                .into(),
        );
    }

    Verdict {
        level,
        decoded,
        contrast,
        notes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::{Ecc, Rgb};

    /// The core guarantee of the whole app: what goes in comes back out.
    ///
    /// Encoding is `qrcode`, decoding is `rqrr`. They share no code, so a
    /// match is evidence rather than the implementation agreeing with itself.
    fn round_trip(payload: &str, ecc: Ecc) {
        let qr = Qr::encode(payload, ecc).unwrap();
        let style = Style::default();
        let got = decode_luma(&render::to_luma(&qr, &style, None));
        assert_eq!(
            got.as_deref(),
            Some(payload),
            "round trip failed at ECC {ecc:?} for {payload:?}"
        );
    }

    #[test]
    fn round_trips_a_plain_url() {
        round_trip("https://example.com", Ecc::M);
    }

    #[test]
    fn round_trips_urls_with_query_and_fragment() {
        round_trip("https://example.com/a/b?x=1&y=two#section-3", Ecc::M);
    }

    #[test]
    fn round_trips_at_every_error_correction_level() {
        for ecc in [Ecc::L, Ecc::M, Ecc::Q, Ecc::H] {
            round_trip("https://example.com/some/path", ecc);
        }
    }

    #[test]
    fn round_trips_punctuation_that_breaks_naive_escaping() {
        round_trip(r#"pass;word\with,quotes":and;semis"#, Ecc::M);
    }

    #[test]
    fn round_trips_multiline_text() {
        round_trip("BEGIN:VCARD\nVERSION:3.0\nN:Doe;Jane\nEND:VCARD", Ecc::M);
    }

    #[test]
    fn round_trips_non_ascii() {
        round_trip("Grüße aus Amsterdam, café €5", Ecc::M);
    }

    #[test]
    fn round_trips_a_long_payload() {
        let long = "https://example.com/?q=".to_string() + &"a".repeat(1200);
        round_trip(&long, Ecc::L);
    }

    #[test]
    fn round_trips_a_single_character() {
        round_trip("x", Ecc::H);
    }

    #[test]
    fn black_on_white_is_good() {
        let qr = Qr::encode("https://example.com", Ecc::M).unwrap();
        let v = check(&qr, &Style::default(), None, "https://example.com");
        assert_eq!(v.level, Level::Good, "notes were {:?}", v.notes);
        assert!(v.decoded);
        assert!(v.notes.is_empty());
    }

    #[test]
    fn near_identical_colours_are_reported_bad() {
        let qr = Qr::encode("https://example.com", Ecc::M).unwrap();
        let style = Style {
            dark: Rgb(0xee, 0xee, 0xee),
            light: Rgb(0xff, 0xff, 0xff),
            ..Default::default()
        };
        let v = check(&qr, &style, None, "https://example.com");
        assert_eq!(v.level, Level::Bad, "notes were {:?}", v.notes);
        assert!(!v.notes.is_empty());
    }

    #[test]
    fn inverted_colours_are_flagged_risky_but_still_decode() {
        let qr = Qr::encode("https://example.com", Ecc::M).unwrap();
        let style = Style {
            dark: Rgb::WHITE,
            light: Rgb::BLACK,
            ..Default::default()
        };
        let v = check(&qr, &style, None, "https://example.com");
        assert_eq!(v.level, Level::Risky, "notes were {:?}", v.notes);
        assert!(v.notes.iter().any(|n| n.contains("inverted")));
    }

    /// An opaque square of the given size, as PNG bytes.
    fn solid_logo(px: u32) -> Logo {
        let img = image::RgbaImage::from_pixel(px, px, image::Rgba([20, 20, 20, 255]));
        let mut bytes = std::io::Cursor::new(Vec::new());
        img.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        Logo::load(&bytes.into_inner()).unwrap()
    }

    /// The reason the logo feature is safe to ship: error correction really
    /// does recover the payload from under a modest overlay, and this proves
    /// it against a decoder rather than against the spec sheet.
    #[test]
    fn a_modest_logo_still_reads_back_at_high_error_correction() {
        let url = "https://example.com/menu";
        let qr = Qr::encode(url, Ecc::H).unwrap();
        let style = Style {
            logo_fraction: 0.18,
            ..Default::default()
        };
        let v = check(&qr, &style, Some(&solid_logo(96)), url);
        assert_eq!(v.level, Level::Good, "notes were {:?}", v.notes);
        assert!(v.decoded);
    }

    /// And the reason it is safe to expose a size slider: when the logo grows
    /// past what the code can recover, the check says so instead of handing
    /// over a pretty, unreadable image.
    #[test]
    fn an_oversized_logo_is_caught_before_it_ships() {
        let url = "https://example.com/menu";
        let qr = Qr::encode(url, Ecc::H).unwrap();
        let style = Style {
            logo_fraction: 0.35,
            ..Default::default()
        };
        let v = check(&qr, &style, Some(&solid_logo(96)), url);
        assert_eq!(
            v.level,
            Level::Bad,
            "a logo this large must be rejected, notes were {:?}",
            v.notes
        );
        assert!(!v.decoded);
    }

    /// A mid-tone brand colour on white is the common real-world case and
    /// must not be nagged about.
    #[test]
    fn a_brand_colour_on_white_is_accepted() {
        let qr = Qr::encode("https://example.com", Ecc::M).unwrap();
        let style = Style {
            dark: Rgb(0x22, 0x88, 0xbf),
            ..Default::default()
        };
        let v = check(&qr, &style, None, "https://example.com");
        assert_eq!(v.level, Level::Good, "notes were {:?}", v.notes);
    }
}
