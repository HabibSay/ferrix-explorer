//! Name rules, unique sibling naming, and renaming.
//!
//! The rules mirror Explorer's: invalid characters and reserved device names are refused,
//! a trailing dot or space is refused, collisions are compared case-insensitively, and a
//! case-only rename falls back to a two-step rename when Windows refuses the direct call.

use std::{
    fs, io,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{error::OperationError, plan::same_path};

const MAX_NAME_LEN: usize = 255;

/// Device names Windows reserves in every directory, with or without an extension.
const RESERVED_DEVICES: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Validates one path component. `Err` carries a machine-readable reason, not user text.
pub fn validate_name(name: &str) -> Result<(), OperationError> {
    if name.is_empty() {
        return Err(OperationError::InvalidName("empty"));
    }
    if name == "." || name == ".." {
        return Err(OperationError::InvalidName("relative"));
    }
    // NTFS limits a component to 255 UTF-16 code units, not 255 bytes: a name of accented
    // or CJK characters is longer in UTF-8 than the real limit allows.
    if name.encode_utf16().count() > MAX_NAME_LEN {
        return Err(OperationError::PathTooLong);
    }
    if name.chars().any(|character| {
        matches!(
            character,
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
        )
    }) {
        return Err(OperationError::InvalidName("invalid-character"));
    }
    if name.chars().any(|character| (character as u32) < 32) {
        return Err(OperationError::InvalidName("control-character"));
    }
    if name.ends_with(' ') || name.ends_with('.') {
        return Err(OperationError::InvalidName("trailing-dot-or-space"));
    }
    let stem = name.split('.').next().unwrap_or(name).to_uppercase();
    if RESERVED_DEVICES.contains(&stem.as_str()) {
        return Err(OperationError::InvalidName("reserved-device"));
    }
    Ok(())
}

/// Splits a name into stem and extension. A leading-dot name (`.gitignore`) has no extension.
fn split_name(name: &str) -> (&str, Option<&str>) {
    match name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() && !extension.is_empty() => {
            (stem, Some(extension))
        }
        _ => (name, None),
    }
}

/// `report.txt` + 2 → `report (2).txt`; `.gitignore` + 2 → `.gitignore (2)`.
pub fn numbered_name(name: &str, number: u32) -> String {
    let (stem, extension) = split_name(name);
    match extension {
        Some(extension) => format!("{stem} ({number}).{extension}"),
        None => format!("{stem} ({number})"),
    }
}

/// First free `stem (n).ext` next to `path`, starting at 2 like Explorer.
pub fn unique_sibling(path: &Path) -> PathBuf {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    for number in 2..10_000 {
        let candidate = parent.join(numbered_name(&name, number));
        if fs::symlink_metadata(&candidate).is_err() {
            return candidate;
        }
    }
    parent.join(numbered_name(&name, 10_000))
}

/// Renames `path` to `new_name` inside its own directory.
pub fn rename_path(path: &Path, new_name: &str) -> Result<PathBuf, OperationError> {
    validate_name(new_name)?;
    let parent = path
        .parent()
        .ok_or(OperationError::InvalidName("no parent directory"))?;
    let target = parent.join(new_name);

    if path.file_name().is_some_and(|name| name == new_name) {
        return Ok(target);
    }

    let case_only = path
        .file_name()
        .map(|name| name.to_string_lossy().to_lowercase() == new_name.to_lowercase())
        .unwrap_or(false);

    if !case_only && conflicting_sibling(parent, new_name, path)? {
        return Err(OperationError::AlreadyExists);
    }

    match fs::rename(path, &target) {
        Ok(()) => Ok(target),
        Err(error) if case_only => two_step_rename(path, &target, parent)
            .map(|()| target)
            .map_err(|error| OperationError::from_io(&error)),
        Err(error) => Err(OperationError::from_io(&error)),
    }
}

fn conflicting_sibling(parent: &Path, name: &str, current: &Path) -> io::Result<bool> {
    let wanted = name.to_lowercase();
    for entry in fs::read_dir(parent)? {
        let entry = entry?;
        let existing = entry.file_name().to_string_lossy().to_lowercase();
        if existing == wanted && !same_path(&entry.path(), current) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Renames through a temporary name, for case-only renames NTFS may refuse directly.
fn two_step_rename(from: &Path, to: &Path, parent: &Path) -> io::Result<()> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let temporary = parent.join(format!(".fxrename-{}-{stamp}", std::process::id()));
    fs::rename(from, &temporary)?;
    match fs::rename(&temporary, to) {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = fs::rename(&temporary, from);
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_ordinary_and_unicode_names() {
        for name in ["report.txt", "résumé – 東京.txt", "archive.tar.gz", "COM0"] {
            assert_eq!(validate_name(name), Ok(()), "{name} should be valid");
        }
    }

    #[test]
    fn rejects_invalid_characters() {
        for name in [
            "a<b", "a>b", "a:b", "a\"b", "a/b", "a\\b", "a|b", "a?b", "a*b",
        ] {
            assert_eq!(
                validate_name(name),
                Err(OperationError::InvalidName("invalid-character")),
                "{name} should be refused"
            );
        }
    }

    #[test]
    fn rejects_control_characters() {
        assert_eq!(
            validate_name("a\u{7}b"),
            Err(OperationError::InvalidName("control-character"))
        );
    }

    #[test]
    fn rejects_trailing_dot_or_space() {
        assert_eq!(
            validate_name("report."),
            Err(OperationError::InvalidName("trailing-dot-or-space"))
        );
        assert_eq!(
            validate_name("report "),
            Err(OperationError::InvalidName("trailing-dot-or-space"))
        );
    }

    #[test]
    fn rejects_reserved_devices_with_or_without_extension() {
        for name in ["CON", "con", "NUL.txt", "COM1.log", "LPT9"] {
            assert_eq!(
                validate_name(name),
                Err(OperationError::InvalidName("reserved-device")),
                "{name} should be refused"
            );
        }
    }

    #[test]
    fn rejects_empty_and_relative_names() {
        assert_eq!(validate_name(""), Err(OperationError::InvalidName("empty")));
        assert_eq!(
            validate_name("."),
            Err(OperationError::InvalidName("relative"))
        );
        assert_eq!(
            validate_name(".."),
            Err(OperationError::InvalidName("relative"))
        );
    }

    #[test]
    fn the_length_limit_counts_utf16_units_not_bytes() {
        // 200 accented characters: 400 UTF-8 bytes, but only 200 UTF-16 units.
        let accented = "é".repeat(200);
        assert!(validate_name(&accented).is_ok());
        assert_eq!(
            validate_name(&"a".repeat(256)),
            Err(OperationError::PathTooLong)
        );
    }

    #[test]
    fn numbers_before_the_last_extension() {
        assert_eq!(numbered_name("report.txt", 2), "report (2).txt");
        assert_eq!(numbered_name("archive.tar.gz", 2), "archive.tar (2).gz");
        assert_eq!(numbered_name(".gitignore", 2), ".gitignore (2)");
        assert_eq!(numbered_name("folder", 3), "folder (3)");
    }

    #[test]
    fn unique_sibling_skips_taken_numbers() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), b"1").unwrap();
        fs::write(dir.path().join("a (2).txt"), b"2").unwrap();
        assert_eq!(
            unique_sibling(&dir.path().join("a.txt")),
            dir.path().join("a (3).txt")
        );
    }

    #[test]
    fn renames_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("old.txt");
        fs::write(&source, b"x").unwrap();

        let target = rename_path(&source, "new.txt").unwrap();
        assert_eq!(target, dir.path().join("new.txt"));
        assert!(!source.exists());
        assert!(target.exists());
    }

    #[test]
    fn case_only_rename_keeps_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("note.txt");
        fs::write(&source, b"content").unwrap();

        let target = rename_path(&source, "Note.txt").unwrap();
        assert_eq!(target, dir.path().join("Note.txt"));
        assert_eq!(fs::read(&target).unwrap(), b"content");
    }

    #[test]
    fn refuses_a_name_already_taken_by_another_file() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("a.txt");
        fs::write(&source, b"a").unwrap();
        fs::write(dir.path().join("b.txt"), b"b").unwrap();

        assert_eq!(
            rename_path(&source, "b.txt"),
            Err(OperationError::AlreadyExists)
        );
        assert!(source.exists());
    }
}
