//! Reading and writing the app's own state without losing any of it.
//!
//! Everything here exists because of one rule: a file this app cannot parse is
//! never overwritten. A ticket ledger records who was issued what and who has
//! already walked through the door. Truncating it because of a parse bug, or a
//! half-written save during a power cut, would destroy the only record of a
//! real event. So a file that fails to load is renamed out of the way, the
//! caller carries on with a default, and the operator is told once.

use serde::{de::DeserializeOwned, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

use crate::export::write_atomic;

/// What came back from a load attempt.
pub enum Loaded<T> {
    Parsed(T),
    /// No file yet, which is the normal state on first run.
    Missing,
    /// Unreadable. The original was moved to `backup` and left untouched.
    Corrupt {
        backup: PathBuf,
        error: String,
    },
}

impl<T> Loaded<T> {
    /// The value, or a default, plus a warning worth showing if there was one.
    pub fn or_default(self) -> (T, Option<String>)
    where
        T: Default,
    {
        match self {
            Loaded::Parsed(v) => (v, None),
            Loaded::Missing => (T::default(), None),
            Loaded::Corrupt { backup, error } => (
                T::default(),
                Some(format!(
                    "{} could not be read ({error}). It was kept as {} and a fresh one started.",
                    backup
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    backup.display()
                )),
            ),
        }
    }
}

/// Read and parse a JSON file, quarantining it if it will not parse.
pub fn load_json<T: DeserializeOwned>(path: &Path) -> Loaded<T> {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Loaded::Missing,
        Err(e) => {
            // Unreadable for a reason other than absence, a permission problem
            // say. Nothing to quarantine, because nothing was understood.
            return Loaded::Corrupt {
                backup: path.to_path_buf(),
                error: e.to_string(),
            };
        }
    };

    match serde_json::from_str(&text) {
        Ok(value) => Loaded::Parsed(value),
        Err(e) => {
            let backup = quarantine(path);
            Loaded::Corrupt {
                backup,
                error: e.to_string(),
            }
        }
    }
}

/// Move a file aside under a timestamped name, so it is recoverable by hand.
fn quarantine(path: &Path) -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "state".into());
    let backup = path.with_file_name(format!("{name}.corrupt-{stamp}"));
    // If even the rename fails there is nothing further to try, and the
    // original is still on disk, which is the outcome that matters.
    let _ = fs::rename(path, &backup);
    backup
}

/// Serialise and write, creating the parent directory if needed.
pub fn save_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("Could not create {parent:?}: {e}"))?;
    }
    // Pretty printed on purpose. These files are small, and being able to read
    // and repair one in a text editor has saved more time than the bytes cost.
    let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    write_atomic(path, text.as_bytes()).map_err(|e| format!("Could not write {path:?}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
    struct Thing {
        name: String,
        count: u32,
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("qrgen-storage-{tag}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_missing_file_is_not_an_error() {
        let path = temp_dir("missing").join("nope.json");
        assert!(matches!(load_json::<Thing>(&path), Loaded::Missing));
        let (value, warning) = load_json::<Thing>(&path).or_default();
        assert_eq!(value, Thing::default());
        assert!(warning.is_none());
    }

    #[test]
    fn values_round_trip() {
        let path = temp_dir("round").join("thing.json");
        let thing = Thing {
            name: "Summer Fest".into(),
            count: 137,
        };
        save_json(&path, &thing).unwrap();
        match load_json::<Thing>(&path) {
            Loaded::Parsed(back) => assert_eq!(back, thing),
            _ => panic!("should have parsed"),
        }
    }

    /// The rule the whole module exists for.
    #[test]
    fn a_corrupt_file_is_kept_rather_than_overwritten() {
        let dir = temp_dir("corrupt");
        let path = dir.join("tickets.json");
        fs::write(&path, "{ this is not json").unwrap();

        let (value, warning) = load_json::<Thing>(&path).or_default();
        assert_eq!(value, Thing::default(), "callers carry on with a default");
        assert!(warning.is_some(), "and the operator is told");

        assert!(!path.exists(), "the bad file is moved out of the way");
        let kept: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".corrupt-"))
            .collect();
        assert_eq!(kept.len(), 1, "exactly one quarantined copy");
        assert_eq!(
            fs::read_to_string(kept[0].path()).unwrap(),
            "{ this is not json",
            "and its contents are untouched, so it can be repaired by hand"
        );
    }

    #[test]
    fn saving_creates_the_directory_it_needs() {
        let path = temp_dir("nested")
            .join("events")
            .join("summer-fest")
            .join("event.json");
        save_json(&path, &Thing::default()).unwrap();
        assert!(path.exists());
    }

    /// A save that lands on top of a good file must not leave a temp behind.
    #[test]
    fn overwriting_leaves_no_debris() {
        let dir = temp_dir("debris");
        let path = dir.join("thing.json");
        save_json(&path, &Thing::default()).unwrap();
        save_json(
            &path,
            &Thing {
                name: "second".into(),
                count: 2,
            },
        )
        .unwrap();

        let files: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(files, vec!["thing.json"], "got {files:?}");
    }
}
