//! Entry filtering and visibility rules.

use crate::entry::FileEntry;

// ── FilterQuery ───────────────────────────────────────────────────────────────

/// A simple text filter applied against entry display names.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct FilterQuery {
    /// The search text.  An empty string matches everything.
    pub text: String,
}

impl FilterQuery {
    /// Returns `true` if `entry` matches this query.
    ///
    /// Matching is case-insensitive substring comparison against `display_name`.
    /// An empty query always returns `true`.
    pub fn matches(&self, entry: &FileEntry) -> bool {
        if self.text.is_empty() {
            return true;
        }
        entry
            .display_name
            .to_ascii_lowercase()
            .contains(&self.text.to_ascii_lowercase())
    }
}

// ── VisibilityRules ───────────────────────────────────────────────────────────

/// Controls which entries the panel shows based on their attributes.
#[derive(Clone, Debug, PartialEq)]
pub struct VisibilityRules {
    /// Show entries whose `hidden` attribute is set.
    pub show_hidden: bool,
    /// Show entries whose `system` attribute is set.
    pub show_system: bool,
    /// Whether the UI should display file extensions in the name column.
    /// This flag does NOT filter entries; it is purely a display hint for the UI layer.
    pub show_extensions: bool,
}

impl Default for VisibilityRules {
    fn default() -> Self {
        VisibilityRules {
            show_hidden: false,
            show_system: false,
            show_extensions: true,
        }
    }
}

impl VisibilityRules {
    /// Returns `true` if `entry` should be shown given these rules.
    pub fn is_visible(&self, entry: &FileEntry) -> bool {
        if entry.attributes.hidden && !self.show_hidden {
            return false;
        }
        if entry.attributes.system && !self.show_system {
            return false;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry::{EntryId, EntryKind, FileAttributes, FileEntry};
    use std::path::PathBuf;

    fn make_entry(name: &str, attrs: FileAttributes) -> FileEntry {
        FileEntry {
            id: EntryId(0),
            path: PathBuf::from(format!("C:\\test\\{name}")),
            display_name: name.to_owned(),
            kind: EntryKind::File,
            size: None,
            modified: None,
            attributes: attrs,
            icon_key: 0,
        }
    }

    #[test]
    fn empty_query_matches_all() {
        let q = FilterQuery::default();
        let e = make_entry("anything.txt", FileAttributes::default());
        assert!(q.matches(&e));
    }

    #[test]
    fn substring_match_case_insensitive() {
        let q = FilterQuery {
            text: "HELLO".to_owned(),
        };
        let yes = make_entry("hello_world.txt", FileAttributes::default());
        let no = make_entry("goodbye.txt", FileAttributes::default());
        assert!(q.matches(&yes));
        assert!(!q.matches(&no));
    }

    #[test]
    fn query_lower_case() {
        let q = FilterQuery {
            text: "rust".to_owned(),
        };
        let e = make_entry("RUST_FILE.rs", FileAttributes::default());
        assert!(q.matches(&e));
    }

    #[test]
    fn visibility_hidden_not_shown_by_default() {
        let rules = VisibilityRules::default();
        let hidden = make_entry(
            "secret",
            FileAttributes {
                hidden: true,
                ..Default::default()
            },
        );
        assert!(!rules.is_visible(&hidden));
    }

    #[test]
    fn visibility_hidden_shown_when_flag_set() {
        let rules = VisibilityRules {
            show_hidden: true,
            ..Default::default()
        };
        let hidden = make_entry(
            "secret",
            FileAttributes {
                hidden: true,
                ..Default::default()
            },
        );
        assert!(rules.is_visible(&hidden));
    }

    #[test]
    fn visibility_system_not_shown_by_default() {
        let rules = VisibilityRules::default();
        let sys = make_entry(
            "sys",
            FileAttributes {
                system: true,
                ..Default::default()
            },
        );
        assert!(!rules.is_visible(&sys));
    }

    #[test]
    fn visibility_system_shown_when_flag_set() {
        let rules = VisibilityRules {
            show_system: true,
            ..Default::default()
        };
        let sys = make_entry(
            "sys",
            FileAttributes {
                system: true,
                ..Default::default()
            },
        );
        assert!(rules.is_visible(&sys));
    }

    #[test]
    fn visibility_normal_file_always_visible() {
        let rules = VisibilityRules::default();
        let e = make_entry("normal.txt", FileAttributes::default());
        assert!(rules.is_visible(&e));
    }
}
