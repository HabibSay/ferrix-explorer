//! Core file-system entry types for Ferrix Explorer.
//!
//! Deliberately excludes git_state, tags, thumbnail_key and cloud_state — those belong to
//! later tranches (see spec section 5).

use std::path::PathBuf;

// ── EntryId ─────────────────────────────────────────────────────────────────

/// Opaque, cheap identifier for a file-system entry within a single listing.
///
/// Values are assigned by the listing layer; they are not stable across reloads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EntryId(pub u64);

// ── EntryKind ────────────────────────────────────────────────────────────────

/// The kind of a file-system entry as reported by the OS.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Directory,
    Symlink,
    ReparsePoint,
}

// ── FileAttributes ────────────────────────────────────────────────────────────

/// Win32-level file attribute flags relevant to the UI.
///
/// Using a plain struct with bool fields avoids a dependency on the `bitflags` crate,
/// which is not in the workspace at this tranche.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct FileAttributes {
    pub hidden: bool,
    pub system: bool,
    pub readonly: bool,
    pub archive: bool,
    pub reparse_point: bool,
}

// ── FileEntry ─────────────────────────────────────────────────────────────────

/// A single entry returned by a directory listing.
#[derive(Clone, Debug, PartialEq)]
pub struct FileEntry {
    /// Stable identifier within the current listing generation.
    pub id: EntryId,
    /// Absolute path of the entry.
    pub path: PathBuf,
    /// Name shown in the UI (no verbatim prefix, extension always present).
    pub display_name: String,
    /// Whether the entry is a file, directory, symlink, or reparse point.
    pub kind: EntryKind,
    /// File size in bytes. `None` for directories.
    pub size: Option<u64>,
    /// Last-modified time. `None` when the OS does not expose it.
    pub modified: Option<std::time::SystemTime>,
    /// Attribute flags (hidden, system, read-only, archive, reparse point).
    pub attributes: FileAttributes,
    /// Icon key assigned by the shell layer. `0` until Slice 4 fills it in.
    pub icon_key: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_entry(name: &str, kind: EntryKind) -> FileEntry {
        FileEntry {
            id: EntryId(1),
            path: PathBuf::from(format!("C:\\test\\{name}")),
            display_name: name.to_owned(),
            kind,
            size: None,
            modified: None,
            attributes: FileAttributes::default(),
            icon_key: 0,
        }
    }

    #[test]
    fn entry_id_equality() {
        assert_eq!(EntryId(42), EntryId(42));
        assert_ne!(EntryId(1), EntryId(2));
    }

    #[test]
    fn entry_kind_clone_debug() {
        let k = EntryKind::Directory;
        assert_eq!(k.clone(), EntryKind::Directory);
        assert!(!format!("{k:?}").is_empty());
    }

    #[test]
    fn file_attributes_default_all_false() {
        let a = FileAttributes::default();
        assert!(!a.hidden);
        assert!(!a.system);
        assert!(!a.readonly);
        assert!(!a.archive);
        assert!(!a.reparse_point);
    }

    #[test]
    fn file_entry_clone_equality() {
        let e = sample_entry("foo.txt", EntryKind::File);
        assert_eq!(e.clone(), e);
    }
}
