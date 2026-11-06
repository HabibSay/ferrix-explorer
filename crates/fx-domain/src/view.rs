//! View-state types: view mode, columns, sort order.
//!
//! Serialisation lives in fx-settings (Tranche 2); these types are serde-free.

use std::fmt;

// ── ViewMode ──────────────────────────────────────────────────────────────────

/// How the file panel renders its entries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ViewMode {
    List,
    #[default]
    Details,
}

// ── Column ────────────────────────────────────────────────────────────────────

/// A column that can appear in Details view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Column {
    Name,
    Size,
    Modified,
    Type,
    Attributes,
}

impl fmt::Display for Column {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Column::Name => "Name",
            Column::Size => "Size",
            Column::Modified => "Modified",
            Column::Type => "Type",
            Column::Attributes => "Attributes",
        };
        write!(f, "{s}")
    }
}

// ── SortDirection ─────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SortDirection {
    #[default]
    Ascending,
    Descending,
}

// ── SortKey ───────────────────────────────────────────────────────────────────

/// Primary (and optional secondary) sort specification.
#[derive(Clone, Debug, PartialEq)]
pub struct SortKey {
    pub primary: Column,
    pub direction: SortDirection,
    pub secondary: Option<(Column, SortDirection)>,
}

impl Default for SortKey {
    fn default() -> Self {
        SortKey {
            primary: Column::Name,
            direction: SortDirection::Ascending,
            secondary: None,
        }
    }
}

// ── ColumnLayout ──────────────────────────────────────────────────────────────

/// A single column descriptor inside a `ColumnLayout`.
#[derive(Clone, Debug, PartialEq)]
pub struct ColumnSpec {
    pub column: Column,
    pub width_px: u32,
    pub visible: bool,
}

/// The ordered set of columns shown in Details view.
#[derive(Clone, Debug, PartialEq)]
pub struct ColumnLayout {
    pub columns: Vec<ColumnSpec>,
}

impl Default for ColumnLayout {
    fn default() -> Self {
        ColumnLayout {
            columns: vec![
                ColumnSpec {
                    column: Column::Name,
                    width_px: 240,
                    visible: true,
                },
                ColumnSpec {
                    column: Column::Size,
                    width_px: 80,
                    visible: true,
                },
                ColumnSpec {
                    column: Column::Modified,
                    width_px: 160,
                    visible: true,
                },
                ColumnSpec {
                    column: Column::Type,
                    width_px: 120,
                    visible: true,
                },
            ],
        }
    }
}

// ── Sorting helper ────────────────────────────────────────────────────────────

/// Sort a slice of `FileEntry` values in place according to `key`.
///
/// Directories always sort before files (folders-first). Within each group,
/// names are compared case-insensitively using `.to_ascii_lowercase()`.
pub fn sort_entries(entries: &mut [crate::entry::FileEntry], key: &SortKey) {
    use crate::entry::EntryKind;

    entries.sort_by(|a, b| {
        // Folders first regardless of sort key.
        let a_is_dir = a.kind == EntryKind::Directory;
        let b_is_dir = b.kind == EntryKind::Directory;
        if a_is_dir != b_is_dir {
            return if a_is_dir {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Greater
            };
        }

        let cmp = compare_by_column(a, b, key.primary, key.direction);
        if cmp != std::cmp::Ordering::Equal {
            return cmp;
        }
        if let Some((sec_col, sec_dir)) = key.secondary {
            compare_by_column(a, b, sec_col, sec_dir)
        } else {
            std::cmp::Ordering::Equal
        }
    });
}

fn compare_by_column(
    a: &crate::entry::FileEntry,
    b: &crate::entry::FileEntry,
    column: Column,
    direction: SortDirection,
) -> std::cmp::Ordering {
    let base = match column {
        Column::Name => a
            .display_name
            .to_ascii_lowercase()
            .cmp(&b.display_name.to_ascii_lowercase()),
        Column::Size => a.size.cmp(&b.size),
        Column::Modified => a.modified.cmp(&b.modified),
        Column::Type => {
            let ext_a = a.path.extension().unwrap_or_default().to_ascii_lowercase();
            let ext_b = b.path.extension().unwrap_or_default().to_ascii_lowercase();
            ext_a.cmp(&ext_b)
        }
        Column::Attributes => std::cmp::Ordering::Equal,
    };

    match direction {
        SortDirection::Ascending => base,
        SortDirection::Descending => base.reverse(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry::{EntryId, EntryKind, FileAttributes, FileEntry};
    use std::path::PathBuf;

    fn make_entry(name: &str, kind: EntryKind, size: Option<u64>) -> FileEntry {
        FileEntry {
            id: EntryId(0),
            path: PathBuf::from(format!("C:\\test\\{name}")),
            display_name: name.to_owned(),
            kind,
            size,
            modified: None,
            attributes: FileAttributes::default(),
            icon_key: 0,
        }
    }

    #[test]
    fn column_display() {
        assert_eq!(Column::Name.to_string(), "Name");
        assert_eq!(Column::Size.to_string(), "Size");
        assert_eq!(Column::Modified.to_string(), "Modified");
        assert_eq!(Column::Type.to_string(), "Type");
        assert_eq!(Column::Attributes.to_string(), "Attributes");
    }

    #[test]
    fn default_column_layout() {
        let layout = ColumnLayout::default();
        assert_eq!(layout.columns.len(), 4);
        assert_eq!(layout.columns[0].column, Column::Name);
        assert_eq!(layout.columns[0].width_px, 240);
        assert!(layout.columns[0].visible);
    }

    #[test]
    fn sort_folders_first() {
        let mut entries = vec![
            make_entry("b.txt", EntryKind::File, Some(100)),
            make_entry("a_dir", EntryKind::Directory, None),
            make_entry("a.txt", EntryKind::File, Some(50)),
        ];
        sort_entries(&mut entries, &SortKey::default());
        assert_eq!(entries[0].kind, EntryKind::Directory);
        // Within files, ascending name order.
        assert_eq!(entries[1].display_name, "a.txt");
        assert_eq!(entries[2].display_name, "b.txt");
    }

    #[test]
    fn sort_case_insensitive() {
        let mut entries = vec![
            make_entry("Zebra.txt", EntryKind::File, None),
            make_entry("apple.txt", EntryKind::File, None),
            make_entry("Mango.txt", EntryKind::File, None),
        ];
        sort_entries(&mut entries, &SortKey::default());
        assert_eq!(entries[0].display_name, "apple.txt");
        assert_eq!(entries[1].display_name, "Mango.txt");
        assert_eq!(entries[2].display_name, "Zebra.txt");
    }

    #[test]
    fn sort_descending() {
        let mut entries = vec![
            make_entry("a.txt", EntryKind::File, None),
            make_entry("b.txt", EntryKind::File, None),
        ];
        let key = SortKey {
            primary: Column::Name,
            direction: SortDirection::Descending,
            secondary: None,
        };
        sort_entries(&mut entries, &key);
        assert_eq!(entries[0].display_name, "b.txt");
        assert_eq!(entries[1].display_name, "a.txt");
    }
}
