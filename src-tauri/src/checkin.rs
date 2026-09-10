//! Checking a ticket at the door.
//!
//! Two questions get answered in order, and the order matters. First, is this
//! ticket genuine? That is the signature, and it needs nothing but the public
//! key. Second, has it already been used? That needs a record, and the record
//! is local.
//!
//! A validly signed ticket that is not in the local ledger is still admitted.
//! The signature is the proof of issue; the ledger is only a convenience for
//! showing a name. A ticket issued from another machine on the same key is
//! genuine, and refusing it because this laptop has not seen it before would
//! turn a working ticket into an argument at the door.
//!
//! The honest limit is one device. Two doors checking at once cannot see each
//! other's log and can both admit the same ticket. Nothing here pretends
//! otherwise; the log exports so two doors can reconcile afterwards.

use serde::Serialize;
use std::collections::BTreeMap;

use crate::events::{now, Redemption, Ticket};
use crate::tickets::{self, TicketError};

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "outcome", rename_all = "camelCase")]
pub enum Outcome {
    /// Genuine, unused, and now recorded.
    Admitted { serial: u32, name: String },
    /// Genuine, but someone already came in on it.
    AlreadyUsed {
        serial: u32,
        name: String,
        first_at: String,
    },
    /// Validly signed, by a different event's key.
    WrongEvent { event: String },
    /// Looks like a ticket, but the signature does not hold.
    NotGenuine,
    /// Not a ticket from this app at all.
    Unreadable { reason: String },
}

/// Another event this machine knows about, so a ticket for the wrong night can
/// be named rather than called a forgery.
pub struct OtherEvent {
    pub name: String,
    pub public_key: String,
}

/// Check one scanned payload, recording the admission if it is allowed.
///
/// The log is mutated rather than returned so the caller can persist it
/// immediately. Admitting someone and then failing to write the record would
/// let the same ticket through twice, which is the one outcome this exists to
/// prevent.
pub fn check(
    public_key: &str,
    others: &[OtherEvent],
    log: &mut BTreeMap<String, Redemption>,
    tickets: &[Ticket],
    scanned: &str,
) -> Outcome {
    let payload = scanned.trim();
    if payload.is_empty() {
        return Outcome::Unreadable {
            reason: "Nothing was scanned.".into(),
        };
    }

    let claim = match tickets::verify(public_key, payload) {
        Ok(c) => c,
        Err(TicketError::BadSignature) => {
            // Structurally a ticket, just not one of ours. Naming the event it
            // does belong to turns "not genuine" into "wrong night", which is
            // the difference between accusing someone and helping them.
            for other in others {
                if tickets::verify(&other.public_key, payload).is_ok() {
                    return Outcome::WrongEvent {
                        event: other.name.clone(),
                    };
                }
            }
            return Outcome::NotGenuine;
        }
        Err(e) => {
            return Outcome::Unreadable {
                reason: e.to_string(),
            }
        }
    };

    let name = tickets
        .iter()
        .find(|t| t.payload == payload)
        .map(|t| t.display_name())
        .unwrap_or_default();

    // Keyed by the payload rather than the serial. Two events can both have a
    // ticket number 7, and the payload is unique by construction.
    if let Some(previous) = log.get(payload) {
        return Outcome::AlreadyUsed {
            serial: previous.serial,
            // Prefer the name recorded at the time, which survives even if the
            // ledger is later replaced.
            name: if previous.name.is_empty() {
                name
            } else {
                previous.name.clone()
            },
            first_at: previous.at.clone(),
        };
    }

    log.insert(
        payload.to_string(),
        Redemption {
            serial: claim.serial,
            at: now(),
            name: name.clone(),
        },
    );

    Outcome::Admitted {
        serial: claim.serial,
        name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tickets::EventKey;

    fn key() -> EventKey {
        EventKey::from_seed(&[7u8; 32])
    }

    fn ledger(payload: &str, name: &str) -> Vec<Ticket> {
        let mut fields = std::collections::BTreeMap::new();
        fields.insert("name".to_string(), name.to_string());
        vec![Ticket::new(1, payload.into(), fields, String::new())]
    }

    #[test]
    fn a_genuine_unused_ticket_is_admitted_and_recorded() {
        let k = key();
        let payload = k.issue("Summer Fest", 1);
        let mut log = BTreeMap::new();

        let out = check(
            &k.public_key(),
            &[],
            &mut log,
            &ledger(&payload, "Marieke"),
            &payload,
        );
        assert_eq!(
            out,
            Outcome::Admitted {
                serial: 1,
                name: "Marieke".into()
            }
        );
        assert_eq!(log.len(), 1, "the admission is written down");
    }

    /// The whole point: the second scan is refused.
    #[test]
    fn the_second_scan_is_refused_and_reports_the_first() {
        let k = key();
        let payload = k.issue("Summer Fest", 1);
        let tickets = ledger(&payload, "Marieke");
        let mut log = BTreeMap::new();

        let first = check(&k.public_key(), &[], &mut log, &tickets, &payload);
        assert!(matches!(first, Outcome::Admitted { .. }));
        let recorded = log[&payload].at.clone();

        let second = check(&k.public_key(), &[], &mut log, &tickets, &payload);
        match second {
            Outcome::AlreadyUsed {
                serial,
                name,
                first_at,
            } => {
                assert_eq!(serial, 1);
                assert_eq!(name, "Marieke");
                assert_eq!(
                    first_at, recorded,
                    "it reports the first scan, not this one"
                );
            }
            other => panic!("expected AlreadyUsed, got {other:?}"),
        }
        assert_eq!(log.len(), 1, "a refused scan does not add a second record");
    }

    /// Checking twice must not move the timestamp, or a repeat scan would keep
    /// looking like it had just happened.
    #[test]
    fn refusal_is_idempotent() {
        let k = key();
        let payload = k.issue("Fest", 1);
        let mut log = BTreeMap::new();
        check(&k.public_key(), &[], &mut log, &[], &payload);
        let at = log[&payload].at.clone();

        for _ in 0..3 {
            check(&k.public_key(), &[], &mut log, &[], &payload);
        }
        assert_eq!(log[&payload].at, at);
    }

    /// A ticket issued elsewhere on the same key is genuine. Refusing it
    /// because this machine has not seen it would break a real ticket.
    #[test]
    fn a_ticket_missing_from_the_local_ledger_is_still_admitted() {
        let k = key();
        let payload = k.issue("Fest", 99);
        let mut log = BTreeMap::new();

        let out = check(&k.public_key(), &[], &mut log, &[], &payload);
        assert_eq!(
            out,
            Outcome::Admitted {
                serial: 99,
                name: String::new()
            },
            "the signature is the proof, not the list"
        );
    }

    #[test]
    fn a_ticket_for_another_event_names_that_event() {
        let mine = key();
        let theirs = EventKey::from_seed(&[9u8; 32]);
        let payload = theirs.issue("Winter Ball", 1);
        let mut log = BTreeMap::new();

        let out = check(
            &mine.public_key(),
            &[OtherEvent {
                name: "Winter Ball".into(),
                public_key: theirs.public_key(),
            }],
            &mut log,
            &[],
            &payload,
        );
        assert_eq!(
            out,
            Outcome::WrongEvent {
                event: "Winter Ball".into()
            }
        );
        assert!(log.is_empty(), "the wrong night is not an admission");
    }

    #[test]
    fn an_unknown_key_is_reported_as_not_genuine() {
        let mine = key();
        let forger = EventKey::from_seed(&[11u8; 32]);
        let payload = forger.issue("Fest", 1);
        let mut log = BTreeMap::new();

        assert_eq!(
            check(&mine.public_key(), &[], &mut log, &[], &payload),
            Outcome::NotGenuine
        );
        assert!(log.is_empty());
    }

    /// An edited serial with the original signature attached.
    #[test]
    fn a_tampered_ticket_is_not_genuine() {
        let k = key();
        let real = k.issue("Fest", 1);
        let sig = real.rsplit('.').next().unwrap();
        let forged = format!("TKT1.{}.{sig}", crate::b64::encode_url(b"Fest|999"));

        let mut log = BTreeMap::new();
        assert_eq!(
            check(&k.public_key(), &[], &mut log, &[], &forged),
            Outcome::NotGenuine
        );
    }

    #[test]
    fn something_that_is_not_a_ticket_says_so_rather_than_accusing_anyone() {
        let k = key();
        let mut log = BTreeMap::new();
        for junk in ["", "   ", "https://example.com", "TKT1.only-two"] {
            match check(&k.public_key(), &[], &mut log, &[], junk) {
                Outcome::Unreadable { .. } => {}
                other => panic!("{junk:?} should be unreadable, got {other:?}"),
            }
        }
        assert!(log.is_empty());
    }

    /// Scanners append a newline, and some prepend whitespace.
    #[test]
    fn surrounding_whitespace_from_a_scanner_is_ignored() {
        let k = key();
        let payload = k.issue("Fest", 1);
        let mut log = BTreeMap::new();
        let out = check(
            &k.public_key(),
            &[],
            &mut log,
            &[],
            &format!("  {payload}\r\n"),
        );
        assert!(matches!(out, Outcome::Admitted { serial: 1, .. }));
    }
}
