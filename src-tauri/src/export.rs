//! Getting bytes onto disk without losing anything, and picking a sane name.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Longest filename stem we will generate.
///
/// Windows caps a full path at 260 characters by default, and a batch export
/// puts these inside a folder the user chose, so leaving headroom matters.
const MAX_STEM: usize = 48;

/// Device names Windows still reserves, with or without an extension.
///
/// A batch export driven from a spreadsheet column will eventually contain a
/// row saying `CON`, and `CON.svg` cannot be created on Windows at all.
const RESERVED: [&str; 22] = [
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// Write via a temporary sibling, then rename over the target.
///
/// A half-written export is worse than no export: the file looks present and
/// opens as a corrupt image. The rename is atomic on both NTFS and APFS, so
/// the target is either the old contents or the complete new ones.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension(format!(
        "{}.tmp",
        path.extension().and_then(|e| e.to_str()).unwrap_or("out")
    ));
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        // Flush to the device before the rename, or a crash immediately after
        // can leave the renamed file present but empty.
        f.sync_all()?;
    }
    fs::rename(&tmp, path)
}

/// Make an arbitrary string safe to use as a filename stem on Windows.
pub fn sanitize_stem(raw: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for c in raw.trim().chars() {
        let ok = c.is_ascii_alphanumeric() || c == '-' || c == '_';
        if ok {
            out.push(c.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash && !out.is_empty() {
            // Collapse every run of unsafe characters into a single dash so
            // "a // b" becomes "a-b" rather than "a----b".
            out.push('-');
            last_dash = true;
        }
        if out.len() >= MAX_STEM {
            break;
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        return "qr-code".into();
    }
    if RESERVED.contains(&trimmed.as_str()) {
        return format!("{trimmed}-qr");
    }
    trimmed
}

/// A filename stem derived from what the code actually contains.
///
/// Saving is the last step of every session, so the dialog arriving with a
/// meaningful name already filled in removes the one bit of typing left.
pub fn suggested_stem(encoded: &str) -> String {
    let s = encoded.trim();
    if s.is_empty() {
        return "qr-code".into();
    }

    // For a URL, the host is what a person would call the file.
    if let Some(rest) = s.split_once("://").map(|(_, r)| r) {
        let host = rest
            .split(['/', '?', '#'])
            .next()
            .unwrap_or("")
            .trim_start_matches("www.")
            .split(':')
            .next()
            .unwrap_or("");
        if !host.is_empty() {
            return sanitize_stem(host);
        }
    }
    sanitize_stem(s)
}

/// Add `-2`, `-3` and so on until the path is free.
///
/// Overwriting silently is the wrong default when a batch is exporting a
/// series of codes into one folder and two of them happen to share a host.
pub fn unique_path(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let first = dir.join(format!("{stem}.{ext}"));
    if !first.exists() {
        return first;
    }
    for n in 2..100_000 {
        let candidate = dir.join(format!("{stem}-{n}.{ext}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    first
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stems_are_stripped_to_safe_characters() {
        assert_eq!(sanitize_stem("Hello, World!"), "hello-world");
        assert_eq!(sanitize_stem("a // b"), "a-b");
        assert_eq!(sanitize_stem("  spaced  out  "), "spaced-out");
        assert_eq!(sanitize_stem(r#"bad<>:"/\|?*chars"#), "bad-chars");
        assert_eq!(
            sanitize_stem("keep-dash_and_underscore"),
            "keep-dash_and_underscore"
        );
    }

    #[test]
    fn empty_and_symbol_only_stems_fall_back() {
        for raw in ["", "   ", "///", "!!!", "..."] {
            assert_eq!(sanitize_stem(raw), "qr-code", "{raw:?} should fall back");
        }
    }

    #[test]
    fn windows_device_names_are_defused() {
        assert_eq!(sanitize_stem("CON"), "con-qr");
        assert_eq!(sanitize_stem("nul"), "nul-qr");
        assert_eq!(sanitize_stem("Com1"), "com1-qr");
        // Only exact matches are reserved; a longer name is fine.
        assert_eq!(sanitize_stem("console"), "console");
    }

    #[test]
    fn stems_are_length_capped() {
        let long = "x".repeat(200);
        assert!(sanitize_stem(&long).len() <= MAX_STEM);
    }

    #[test]
    fn suggested_stem_uses_the_host_of_a_url() {
        assert_eq!(suggested_stem("https://example.com"), "example-com");
        assert_eq!(
            suggested_stem("https://www.example.com/a/b?c=d"),
            "example-com"
        );
        assert_eq!(
            suggested_stem("http://sub.example.co.uk:8080/x"),
            "sub-example-co-uk"
        );
    }

    #[test]
    fn suggested_stem_falls_back_to_the_text() {
        assert_eq!(suggested_stem("Table 12"), "table-12");
        assert_eq!(suggested_stem(""), "qr-code");
    }

    #[test]
    fn atomic_write_leaves_no_temp_file_behind() {
        let dir = std::env::temp_dir().join("qrgen-export-test");
        let _ = fs::create_dir_all(&dir);
        let target = dir.join("out.svg");
        write_atomic(&target, b"<svg/>").unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"<svg/>");

        let leftovers: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "temp files left: {leftovers:?}");

        // Overwriting an existing file must also succeed.
        write_atomic(&target, b"<svg>2</svg>").unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"<svg>2</svg>");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn unique_path_avoids_collisions() {
        let dir = std::env::temp_dir().join("qrgen-unique-test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let a = unique_path(&dir, "code", "png");
        assert!(a.ends_with("code.png"));
        fs::write(&a, b"x").unwrap();

        let b = unique_path(&dir, "code", "png");
        assert!(b.ends_with("code-2.png"), "got {b:?}");
        fs::write(&b, b"x").unwrap();

        let c = unique_path(&dir, "code", "png");
        assert!(c.ends_with("code-3.png"), "got {c:?}");
        let _ = fs::remove_dir_all(&dir);
    }
}
