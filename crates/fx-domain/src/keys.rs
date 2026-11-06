//! Keyboard shortcut definitions for Tranche 1.
//!
//! These types are intentionally decoupled from any UI framework (Slint, winit, etc.).
//! The UI layer maps its native key events to these enums before dispatching.

// ── Key ───────────────────────────────────────────────────────────────────────

/// Physical keys that can participate in a shortcut.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    // Arrow keys
    Left,
    Right,
    Up,
    Down,
    // Function row
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
    // Alphabet
    A,
    B,
    C,
    D,
    E,
    F,
    G,
    H,
    I,
    J,
    K,
    L,
    M,
    N,
    O,
    P,
    Q,
    R,
    S,
    T,
    U,
    V,
    W,
    X,
    Y,
    Z,
    // Special
    Enter,
    Escape,
    Delete,
    BackSpace,
}

// ── Modifiers ─────────────────────────────────────────────────────────────────

/// Modifier-key state at the time a key is pressed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Modifiers {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
}

// ── Shortcut ─────────────────────────────────────────────────────────────────

/// A fully-qualified keyboard shortcut (modifiers + key).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Shortcut {
    pub modifiers: Modifiers,
    pub key: Key,
}

impl Shortcut {
    const fn ctrl(key: Key) -> Self {
        Shortcut {
            modifiers: Modifiers {
                ctrl: true,
                alt: false,
                shift: false,
                win: false,
            },
            key,
        }
    }

    const fn alt(key: Key) -> Self {
        Shortcut {
            modifiers: Modifiers {
                ctrl: false,
                alt: true,
                shift: false,
                win: false,
            },
            key,
        }
    }

    const fn shift(key: Key) -> Self {
        Shortcut {
            modifiers: Modifiers {
                ctrl: false,
                alt: false,
                shift: true,
                win: false,
            },
            key,
        }
    }

    const fn bare(key: Key) -> Self {
        Shortcut {
            modifiers: Modifiers {
                ctrl: false,
                alt: false,
                shift: false,
                win: false,
            },
            key,
        }
    }

    const fn ctrl_shift(key: Key) -> Self {
        Shortcut {
            modifiers: Modifiers {
                ctrl: true,
                alt: false,
                shift: true,
                win: false,
            },
            key,
        }
    }
}

// ── ShortcutAction ────────────────────────────────────────────────────────────

/// High-level actions that shortcuts can invoke.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShortcutAction {
    NavigateBack,
    NavigateForward,
    NavigateUp,
    Refresh,
    FocusAddressBar,
    SelectAll,
    ContextMenu,
    Rename,
    Delete,
    Cut,
    Copy,
    Paste,
    Properties,
    // Tranche 2 additions
    Undo,
    Redo,
    CreateFolder,
    PermanentDelete,
}

// ── Default shortcut map ──────────────────────────────────────────────────────

/// Returns the default shortcut-to-action mapping for Tranche 1 + Tranche 2.
pub fn default_shortcuts() -> Vec<(Shortcut, ShortcutAction)> {
    vec![
        (Shortcut::alt(Key::Left), ShortcutAction::NavigateBack),
        (Shortcut::alt(Key::Right), ShortcutAction::NavigateForward),
        (Shortcut::alt(Key::Up), ShortcutAction::NavigateUp),
        (Shortcut::bare(Key::F5), ShortcutAction::Refresh),
        (Shortcut::ctrl(Key::L), ShortcutAction::FocusAddressBar),
        (Shortcut::alt(Key::D), ShortcutAction::FocusAddressBar),
        (Shortcut::ctrl(Key::A), ShortcutAction::SelectAll),
        (Shortcut::shift(Key::F10), ShortcutAction::ContextMenu),
        // Tranche 2 shortcuts
        (Shortcut::ctrl(Key::Z), ShortcutAction::Undo),
        (Shortcut::ctrl(Key::Y), ShortcutAction::Redo),
        (Shortcut::ctrl_shift(Key::N), ShortcutAction::CreateFolder),
        (
            Shortcut::shift(Key::Delete),
            ShortcutAction::PermanentDelete,
        ),
        (Shortcut::bare(Key::Delete), ShortcutAction::Delete),
        (Shortcut::bare(Key::F2), ShortcutAction::Rename),
        (Shortcut::ctrl(Key::C), ShortcutAction::Copy),
        (Shortcut::ctrl(Key::X), ShortcutAction::Cut),
        (Shortcut::ctrl(Key::V), ShortcutAction::Paste),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_shortcuts_contains_navigate_back() {
        let map = default_shortcuts();
        let found = map.iter().any(|(s, a)| {
            *a == ShortcutAction::NavigateBack && s.modifiers.alt && s.key == Key::Left
        });
        assert!(found, "Alt+Left → NavigateBack not found");
    }

    #[test]
    fn default_shortcuts_contains_refresh() {
        let map = default_shortcuts();
        let found = map.iter().any(|(s, a)| {
            *a == ShortcutAction::Refresh
                && !s.modifiers.alt
                && !s.modifiers.ctrl
                && s.key == Key::F5
        });
        assert!(found, "F5 → Refresh not found");
    }

    #[test]
    fn default_shortcuts_contains_focus_address_bar_ctrl_l() {
        let map = default_shortcuts();
        let found = map.iter().any(|(s, a)| {
            *a == ShortcutAction::FocusAddressBar && s.modifiers.ctrl && s.key == Key::L
        });
        assert!(found, "Ctrl+L → FocusAddressBar not found");
    }

    #[test]
    fn default_shortcuts_contains_focus_address_bar_alt_d() {
        let map = default_shortcuts();
        let found = map.iter().any(|(s, a)| {
            *a == ShortcutAction::FocusAddressBar && s.modifiers.alt && s.key == Key::D
        });
        assert!(found, "Alt+D → FocusAddressBar not found");
    }

    #[test]
    fn default_shortcuts_contains_context_menu() {
        let map = default_shortcuts();
        let found = map.iter().any(|(s, a)| {
            *a == ShortcutAction::ContextMenu && s.modifiers.shift && s.key == Key::F10
        });
        assert!(found, "Shift+F10 → ContextMenu not found");
    }

    #[test]
    fn no_duplicate_shortcuts() {
        let map = default_shortcuts();
        let mut seen = std::collections::HashSet::new();
        for (s, _) in &map {
            assert!(seen.insert(s), "Duplicate shortcut: {s:?}");
        }
    }

    #[test]
    fn ctrl_z_maps_to_undo() {
        let map = default_shortcuts();
        let found = map.iter().any(|(s, a)| {
            *a == ShortcutAction::Undo && s.modifiers.ctrl && !s.modifiers.shift && s.key == Key::Z
        });
        assert!(found, "Ctrl+Z → Undo not found");
    }

    #[test]
    fn ctrl_y_maps_to_redo() {
        let map = default_shortcuts();
        let found = map
            .iter()
            .any(|(s, a)| *a == ShortcutAction::Redo && s.modifiers.ctrl && s.key == Key::Y);
        assert!(found, "Ctrl+Y → Redo not found");
    }

    #[test]
    fn ctrl_shift_n_maps_to_create_folder() {
        let map = default_shortcuts();
        let found = map.iter().any(|(s, a)| {
            *a == ShortcutAction::CreateFolder
                && s.modifiers.ctrl
                && s.modifiers.shift
                && s.key == Key::N
        });
        assert!(found, "Ctrl+Shift+N → CreateFolder not found");
    }

    #[test]
    fn shift_delete_maps_to_permanent_delete() {
        let map = default_shortcuts();
        let found = map.iter().any(|(s, a)| {
            *a == ShortcutAction::PermanentDelete && s.modifiers.shift && s.key == Key::Delete
        });
        assert!(found, "Shift+Del → PermanentDelete not found");
    }

    #[test]
    fn delete_maps_to_delete() {
        let map = default_shortcuts();
        let found = map.iter().any(|(s, a)| {
            *a == ShortcutAction::Delete && !s.modifiers.shift && s.key == Key::Delete
        });
        assert!(found, "Del → Delete not found");
    }

    #[test]
    fn f2_maps_to_rename() {
        let map = default_shortcuts();
        let found = map.iter().any(|(s, a)| {
            *a == ShortcutAction::Rename
                && !s.modifiers.ctrl
                && !s.modifiers.shift
                && s.key == Key::F2
        });
        assert!(found, "F2 → Rename not found");
    }

    #[test]
    fn ctrl_c_maps_to_copy() {
        let map = default_shortcuts();
        let found = map
            .iter()
            .any(|(s, a)| *a == ShortcutAction::Copy && s.modifiers.ctrl && s.key == Key::C);
        assert!(found, "Ctrl+C → Copy not found");
    }

    #[test]
    fn ctrl_x_maps_to_cut() {
        let map = default_shortcuts();
        let found = map
            .iter()
            .any(|(s, a)| *a == ShortcutAction::Cut && s.modifiers.ctrl && s.key == Key::X);
        assert!(found, "Ctrl+X → Cut not found");
    }

    #[test]
    fn ctrl_v_maps_to_paste() {
        let map = default_shortcuts();
        let found = map
            .iter()
            .any(|(s, a)| *a == ShortcutAction::Paste && s.modifiers.ctrl && s.key == Key::V);
        assert!(found, "Ctrl+V → Paste not found");
    }
}
