//! Issuing tickets a door can verify without a network.
//!
//! Three separate problems hide behind the phrase "working tickets", and this
//! module solves exactly one of them:
//!
//! 1. **Issuance**: minting unique codes and putting them on something
//!    printable. That is this module, plus `pdf`.
//! 2. **Authenticity**: proving a code came from you. Also here. Each payload
//!    carries an Ed25519 signature, so a scanner holding only the public key
//!    can check it offline, with no guest list and no server. Forging one
//!    needs the private key.
//! 3. **Double-scan prevention**: stopping one ticket being used twice. This
//!    needs state shared between the scanners at the door, and no generator
//!    can provide it. The exported CSV exists so whatever runs the door can
//!    track redemption itself.
//!
//! The app must not imply it does the third, so nothing here pretends to.

use crate::b64;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::Serialize;

/// Version marker, so a scanner can reject a format it does not understand
/// rather than misreading it. Bump it if the body layout ever changes.
const PREFIX: &str = "TKT1";

/// The separator inside the signed body. Stripped from event names so it
/// cannot appear twice and shift the serial into the event field.
const SEP: char = '|';

#[derive(Debug)]
pub enum TicketError {
    Randomness(String),
    BadKey(String),
    BadPayload(&'static str),
    BadSignature,
}

impl std::fmt::Display for TicketError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            TicketError::Randomness(e) => write!(f, "Could not generate a key: {e}"),
            TicketError::BadKey(e) => write!(f, "That is not a valid key: {e}"),
            TicketError::BadPayload(w) => write!(f, "Malformed ticket: {w}"),
            TicketError::BadSignature => write!(f, "Signature does not match."),
        }
    }
}

impl std::error::Error for TicketError {}

/// What a scanner learns from a ticket it has verified.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Claim {
    pub event: String,
    pub serial: u32,
}

/// Strip anything that would confuse the signed body or a filename.
///
/// The separator has to go, or an event called `a|b` would produce a body with
/// three fields and a scanner would read the serial from the wrong place.
pub fn clean_event(raw: &str) -> String {
    let cleaned: String = raw
        .trim()
        .chars()
        .filter(|c| *c != SEP && !c.is_control())
        .collect();
    let cleaned = cleaned.trim().to_string();
    if cleaned.is_empty() {
        "event".into()
    } else {
        cleaned.chars().take(64).collect()
    }
}

pub struct EventKey {
    signing: SigningKey,
}

impl EventKey {
    /// A fresh key from the operating system's randomness.
    ///
    /// Filled directly rather than through `rand_core` so the app does not
    /// have to track which major version of it `ed25519-dalek` expects this
    /// month, which has changed more than once.
    pub fn generate() -> Result<EventKey, TicketError> {
        let mut seed = [0u8; 32];
        getrandom::fill(&mut seed).map_err(|e| TicketError::Randomness(e.to_string()))?;
        Ok(EventKey::from_seed(&seed))
    }

    pub fn from_seed(seed: &[u8; 32]) -> EventKey {
        EventKey {
            signing: SigningKey::from_bytes(seed),
        }
    }

    /// The private seed, for storing. Anyone holding this can issue tickets.
    pub fn seed(&self) -> [u8; 32] {
        self.signing.to_bytes()
    }

    /// The public key a scanner needs, and nothing more than that.
    pub fn public_key(&self) -> String {
        b64::encode_url(self.signing.verifying_key().as_bytes())
    }

    /// The exact string that goes into one ticket's QR code.
    ///
    /// Roughly 110 characters, which fits comfortably at medium error
    /// correction and prints small enough for a ticket stub.
    pub fn issue(&self, event: &str, serial: u32) -> String {
        let body = format!("{}{SEP}{serial}", clean_event(event));
        let sig = self.signing.sign(body.as_bytes());
        format!(
            "{PREFIX}.{}.{}",
            b64::encode_url(body.as_bytes()),
            b64::encode_url(&sig.to_bytes())
        )
    }
}

/// Check a ticket against a public key.
///
/// This is the whole of what a door scanner has to do, which is why it lives
/// here rather than only in the documentation: the README describes it, and
/// this function is the executable version of that description.
pub fn verify(public_key: &str, payload: &str) -> Result<Claim, TicketError> {
    let key_bytes = b64::decode_url(public_key.trim())
        .ok_or_else(|| TicketError::BadKey("not base64url".into()))?;
    let key_bytes: [u8; 32] = key_bytes
        .try_into()
        .map_err(|_| TicketError::BadKey("wrong length".into()))?;
    let verifying =
        VerifyingKey::from_bytes(&key_bytes).map_err(|e| TicketError::BadKey(e.to_string()))?;

    let mut parts = payload.trim().split('.');
    let (Some(prefix), Some(body_b64), Some(sig_b64), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(TicketError::BadPayload(
            "expected three dot-separated parts",
        ));
    };
    if prefix != PREFIX {
        return Err(TicketError::BadPayload("unknown format version"));
    }

    let body = b64::decode_url(body_b64).ok_or(TicketError::BadPayload("body is not base64url"))?;
    let sig_bytes =
        b64::decode_url(sig_b64).ok_or(TicketError::BadPayload("signature is not base64url"))?;
    let sig_bytes: [u8; 64] = sig_bytes
        .try_into()
        .map_err(|_| TicketError::BadPayload("signature is the wrong length"))?;

    verifying
        .verify(&body, &Signature::from_bytes(&sig_bytes))
        .map_err(|_| TicketError::BadSignature)?;

    // Only parsed after the signature checks out, so malformed input from an
    // unsigned source never reaches the parser.
    let body = String::from_utf8(body).map_err(|_| TicketError::BadPayload("body is not UTF-8"))?;
    let (event, serial) = body
        .rsplit_once(SEP)
        .ok_or(TicketError::BadPayload("body has no serial"))?;
    let serial = serial
        .parse()
        .map_err(|_| TicketError::BadPayload("serial is not a number"))?;

    Ok(Claim {
        event: event.to_string(),
        serial,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> EventKey {
        // A fixed seed keeps the tests deterministic. Never used for issuing.
        EventKey::from_seed(&[7u8; 32])
    }

    #[test]
    fn a_ticket_verifies_against_the_public_key_alone() {
        let k = key();
        let payload = k.issue("Summer Fest", 137);
        let claim = verify(&k.public_key(), &payload).expect("must verify");
        assert_eq!(
            claim,
            Claim {
                event: "Summer Fest".into(),
                serial: 137
            }
        );
    }

    #[test]
    fn every_serial_produces_a_different_code() {
        let k = key();
        let a = k.issue("Fest", 1);
        let b = k.issue("Fest", 2);
        assert_ne!(a, b);
        assert_eq!(verify(&k.public_key(), &a).unwrap().serial, 1);
        assert_eq!(verify(&k.public_key(), &b).unwrap().serial, 2);
    }

    /// The property the whole scheme rests on: changing the ticket without
    /// the private key invalidates it.
    #[test]
    fn editing_the_serial_breaks_the_signature() {
        let k = key();
        let real = k.issue("Fest", 1);
        let forged = {
            // Re-encode a body claiming serial 999, keeping the real signature.
            let sig = real.rsplit('.').next().unwrap();
            format!("{PREFIX}.{}.{sig}", b64::encode_url(b"Fest|999"))
        };
        assert!(matches!(
            verify(&k.public_key(), &forged),
            Err(TicketError::BadSignature)
        ));
    }

    #[test]
    fn a_ticket_from_another_event_key_is_rejected() {
        let mine = key();
        let theirs = EventKey::from_seed(&[9u8; 32]);
        let payload = theirs.issue("Fest", 1);
        assert!(matches!(
            verify(&mine.public_key(), &payload),
            Err(TicketError::BadSignature)
        ));
    }

    #[test]
    fn malformed_payloads_are_rejected_without_panicking() {
        let k = key();
        let pk = k.public_key();
        for bad in [
            "",
            "nonsense",
            "TKT1.only-two-parts",
            "TKT1.a.b.c.d",
            "TKT9.aaaa.bbbb",
            "TKT1.!!!.bbbb",
            "TKT1.RmVzdHwx.tooshort",
        ] {
            assert!(verify(&pk, bad).is_err(), "{bad:?} must be rejected");
        }
    }

    #[test]
    fn a_generated_key_round_trips_through_its_seed() {
        let k = EventKey::generate().expect("os randomness must work");
        let restored = EventKey::from_seed(&k.seed());
        let payload = k.issue("Fest", 42);
        assert_eq!(
            verify(&restored.public_key(), &payload).unwrap().serial,
            42,
            "a key reloaded from disk must issue and verify identically"
        );
    }

    #[test]
    fn two_generated_keys_differ() {
        let a = EventKey::generate().unwrap();
        let b = EventKey::generate().unwrap();
        assert_ne!(a.seed(), b.seed());
    }

    /// The payload has to fit a printable code. A ticket nobody can scan from
    /// a stub is not a ticket.
    #[test]
    fn the_payload_stays_short_enough_to_print() {
        let k = key();
        let payload = k.issue("A Reasonably Long Festival Name 2026", 99999);
        assert!(
            payload.len() < 160,
            "payload was {} chars: {payload}",
            payload.len()
        );
        assert!(
            payload
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_'),
            "must stay in the URL-safe set so no scanner mangles it"
        );
    }

    #[test]
    fn event_names_cannot_smuggle_in_a_separator() {
        let k = key();
        let payload = k.issue("Fake|999", 1);
        let claim = verify(&k.public_key(), &payload).unwrap();
        assert_eq!(claim.event, "Fake999");
        assert_eq!(claim.serial, 1, "the real serial must survive");
    }

    #[test]
    fn a_blank_event_name_falls_back_rather_than_producing_an_empty_field() {
        assert_eq!(clean_event("   "), "event");
        assert_eq!(clean_event(""), "event");
    }
}
