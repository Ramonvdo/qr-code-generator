//! Base64, in both the alphabets this app needs.
//!
//! Standard with padding for the SVG data URI, and the URL-safe alphabet
//! without padding for ticket payloads, where `+`, `/` and `=` would all be
//! mangled by anything that treats the string as a URL.
//!
//! Two dozen lines against a dependency that would be pulled in for three
//! calls, and one whose exact API has churned across major versions more than
//! once. The vectors in the tests come from RFC 4648 itself.

const STANDARD: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const URL_SAFE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

fn encode_with(bytes: &[u8], alphabet: &[u8; 64], pad: bool) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(alphabet[(n >> 18) as usize & 63] as char);
        out.push(alphabet[(n >> 12) as usize & 63] as char);
        match chunk.len() {
            1 => {
                if pad {
                    out.push_str("==");
                }
            }
            2 => {
                out.push(alphabet[(n >> 6) as usize & 63] as char);
                if pad {
                    out.push('=');
                }
            }
            _ => {
                out.push(alphabet[(n >> 6) as usize & 63] as char);
                out.push(alphabet[n as usize & 63] as char);
            }
        }
    }
    out
}

/// Standard alphabet, padded. What a `data:` URI expects.
pub fn encode(bytes: &[u8]) -> String {
    encode_with(bytes, STANDARD, true)
}

/// URL-safe alphabet, unpadded. What goes inside a ticket payload.
pub fn encode_url(bytes: &[u8]) -> String {
    encode_with(bytes, URL_SAFE, false)
}

/// Decode the URL-safe, unpadded form.
///
/// Present so the test suite can verify a ticket the same way an independent
/// scanner would, rather than trusting the encoder to agree with itself.
pub fn decode_url(s: &str) -> Option<Vec<u8>> {
    let mut acc: u32 = 0;
    let mut bits = 0u32;
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    for c in s.bytes() {
        let v = URL_SAFE.iter().position(|&a| a == c)? as u32;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    // Leftover bits must be zero padding, never dropped data.
    if bits >= 6 || (acc & ((1 << bits) - 1)) != 0 {
        return None;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 4648 section 10, an independent source of truth rather than this
    /// implementation checked against itself.
    #[test]
    fn standard_matches_the_rfc_vectors() {
        for (input, expected) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(encode(input.as_bytes()), expected, "for {input:?}");
        }
    }

    #[test]
    fn url_safe_drops_padding_and_swaps_the_two_symbols() {
        for (input, expected) in [
            ("", ""),
            ("f", "Zg"),
            ("fo", "Zm8"),
            ("foo", "Zm9v"),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(encode_url(input.as_bytes()), expected, "for {input:?}");
        }
        // 0xFB 0xFF encodes to "+/" in the standard alphabet and "-_" here.
        assert_eq!(encode(&[0xfb, 0xff]), "+/8=");
        assert_eq!(encode_url(&[0xfb, 0xff]), "-_8");
    }

    #[test]
    fn url_safe_round_trips_arbitrary_bytes() {
        for len in [0usize, 1, 2, 3, 31, 32, 64, 65] {
            let bytes: Vec<u8> = (0..len).map(|i| (i * 7 + 13) as u8).collect();
            let encoded = encode_url(&bytes);
            assert!(!encoded.contains('='), "no padding in {encoded}");
            assert_eq!(decode_url(&encoded).as_deref(), Some(&bytes[..]));
        }
    }

    #[test]
    fn decoding_rejects_characters_outside_the_alphabet() {
        assert!(decode_url("abc!").is_none());
        assert!(decode_url("ab=c").is_none(), "padding is not accepted");
        assert!(decode_url("ab+c").is_none(), "standard symbols are not");
    }
}
