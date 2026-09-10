//! Reading a QR code back out of an image.
//!
//! The inverse of the rest of the app, and the same `rqrr` that powers the
//! scannability check. Useful for finding out what a code someone sent you
//! actually contains before you trust it, which is the one thing a printed
//! code never tells you.

use crate::render::Luma;

#[derive(Debug)]
pub enum DecodeError {
    NotAnImage(String),
    NothingFound,
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            DecodeError::NotAnImage(e) => write!(f, "Could not read that image: {e}"),
            DecodeError::NothingFound => write!(
                f,
                "No QR code found in that image. Try a sharper or less cropped picture."
            ),
        }
    }
}

impl std::error::Error for DecodeError {}

/// Every code found in the image, in the order the detector reports them.
///
/// Returns all of them rather than the first, because a photograph of a page
/// of stickers legitimately contains many and picking one arbitrarily would
/// be worse than showing the lot.
pub fn decode_bytes(bytes: &[u8]) -> Result<Vec<String>, DecodeError> {
    let image = image::load_from_memory(bytes)
        .map_err(|e| DecodeError::NotAnImage(e.to_string()))?
        .to_luma8();
    let (w, h) = (image.width() as usize, image.height() as usize);

    let luma = Luma {
        width: w,
        height: h,
        pixels: image.into_raw(),
    };
    let found = decode_all(&luma);
    if found.is_empty() {
        return Err(DecodeError::NothingFound);
    }
    Ok(found)
}

/// Detect and decode every grid in a greyscale raster.
fn decode_all(luma: &Luma) -> Vec<String> {
    let (w, h) = (luma.width, luma.height);
    let mut prepared =
        rqrr::PreparedImage::prepare_from_greyscale(w, h, |x, y| luma.pixels[y * w + x]);
    prepared
        .detect_grids()
        .iter()
        .filter_map(|g| g.decode().ok().map(|(_meta, content)| content))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::{Ecc, Qr, Style};

    /// A PNG of a code containing `payload`, produced by the app's own
    /// renderer, so this is a genuine encode-then-decode round trip through
    /// the real file format rather than an in-memory shortcut.
    fn png_of(payload: &str) -> Vec<u8> {
        let qr = Qr::encode(payload, Ecc::M).unwrap();
        crate::render::to_png(&qr, &Style::default(), None).unwrap()
    }

    #[test]
    fn reads_back_a_code_the_app_produced() {
        let payload = "https://example.com/menu?table=12";
        assert_eq!(decode_bytes(&png_of(payload)).unwrap(), vec![payload]);
    }

    #[test]
    fn reads_back_a_wifi_payload_with_escapes_intact() {
        let payload = r#"WIFI:T:WPA;S:My\;Net;P:pa\\ss;;"#;
        assert_eq!(decode_bytes(&png_of(payload)).unwrap(), vec![payload]);
    }

    #[test]
    fn reads_back_non_ascii() {
        let payload = "Grüße aus Amsterdam";
        assert_eq!(decode_bytes(&png_of(payload)).unwrap(), vec![payload]);
    }

    #[test]
    fn an_image_with_no_code_says_so() {
        let blank = image::RgbImage::from_pixel(200, 200, image::Rgb([255, 255, 255]));
        let mut bytes = std::io::Cursor::new(Vec::new());
        blank.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        assert!(matches!(
            decode_bytes(&bytes.into_inner()),
            Err(DecodeError::NothingFound)
        ));
    }

    #[test]
    fn a_file_that_is_not_an_image_is_reported_clearly() {
        assert!(matches!(
            decode_bytes(b"not an image at all"),
            Err(DecodeError::NotAnImage(_))
        ));
    }
}
