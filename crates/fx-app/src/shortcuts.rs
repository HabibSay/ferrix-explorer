//! Keyboard dispatch: maps Slint key events to domain shortcut actions.

use fx_domain::keys::{Key, Modifiers, Shortcut, ShortcutAction, default_shortcuts};
use slint::platform::Key as SlintKey;

/// Resolves a Slint key text plus modifier state through the default shortcut map.
/// Returns `None` for text that is not a single mapped key or an unconfigured combination.
pub fn action_for(text: &str, ctrl: bool, shift: bool, alt: bool) -> Option<ShortcutAction> {
    let key = domain_key(text)?;
    let shortcut = Shortcut {
        modifiers: Modifiers {
            ctrl,
            alt,
            shift,
            win: false,
        },
        key,
    };
    default_shortcuts()
        .into_iter()
        .find(|(s, _)| *s == shortcut)
        .map(|(_, action)| action)
}

fn domain_key(text: &str) -> Option<Key> {
    let mut chars = text.chars();
    let c = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    if c.is_ascii_alphabetic() {
        return Some(match c.to_ascii_lowercase() {
            'a' => Key::A,
            'b' => Key::B,
            'c' => Key::C,
            'd' => Key::D,
            'e' => Key::E,
            'f' => Key::F,
            'g' => Key::G,
            'h' => Key::H,
            'i' => Key::I,
            'j' => Key::J,
            'k' => Key::K,
            'l' => Key::L,
            'm' => Key::M,
            'n' => Key::N,
            'o' => Key::O,
            'p' => Key::P,
            'q' => Key::Q,
            'r' => Key::R,
            's' => Key::S,
            't' => Key::T,
            'u' => Key::U,
            'v' => Key::V,
            'w' => Key::W,
            'x' => Key::X,
            'y' => Key::Y,
            'z' => Key::Z,
            _ => return None,
        });
    }
    Some(match c {
        c if c == char::from(SlintKey::Delete) => Key::Delete,
        c if c == char::from(SlintKey::F1) => Key::F1,
        c if c == char::from(SlintKey::F2) => Key::F2,
        c if c == char::from(SlintKey::F3) => Key::F3,
        c if c == char::from(SlintKey::F4) => Key::F4,
        c if c == char::from(SlintKey::F5) => Key::F5,
        c if c == char::from(SlintKey::F6) => Key::F6,
        c if c == char::from(SlintKey::F7) => Key::F7,
        c if c == char::from(SlintKey::F8) => Key::F8,
        c if c == char::from(SlintKey::F9) => Key::F9,
        c if c == char::from(SlintKey::F10) => Key::F10,
        c if c == char::from(SlintKey::F11) => Key::F11,
        c if c == char::from(SlintKey::F12) => Key::F12,
        c if c == char::from(SlintKey::LeftArrow) => Key::Left,
        c if c == char::from(SlintKey::RightArrow) => Key::Right,
        c if c == char::from(SlintKey::UpArrow) => Key::Up,
        c if c == char::from(SlintKey::DownArrow) => Key::Down,
        c if c == char::from(SlintKey::Escape) => Key::Escape,
        c if c == char::from(SlintKey::Return) => Key::Enter,
        c if c == char::from(SlintKey::Backspace) => Key::BackSpace,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::action_for;
    use fx_domain::keys::ShortcutAction;
    use slint::platform::Key;

    fn key(k: Key) -> String {
        String::from(char::from(k))
    }

    #[test]
    fn ctrl_c_is_copy() {
        assert_eq!(
            action_for("c", true, false, false),
            Some(ShortcutAction::Copy)
        );
    }

    #[test]
    fn ctrl_x_is_cut() {
        assert_eq!(
            action_for("x", true, false, false),
            Some(ShortcutAction::Cut)
        );
    }

    #[test]
    fn ctrl_v_is_paste() {
        assert_eq!(
            action_for("v", true, false, false),
            Some(ShortcutAction::Paste)
        );
    }

    #[test]
    fn ctrl_z_is_undo() {
        assert_eq!(
            action_for("z", true, false, false),
            Some(ShortcutAction::Undo)
        );
    }

    #[test]
    fn ctrl_y_is_redo() {
        assert_eq!(
            action_for("y", true, false, false),
            Some(ShortcutAction::Redo)
        );
    }

    #[test]
    fn f2_is_rename() {
        assert_eq!(
            action_for(&key(Key::F2), false, false, false),
            Some(ShortcutAction::Rename)
        );
    }

    #[test]
    fn delete_is_recycle() {
        assert_eq!(
            action_for(&key(Key::Delete), false, false, false),
            Some(ShortcutAction::Delete)
        );
    }

    #[test]
    fn shift_delete_is_permanent_delete() {
        assert_eq!(
            action_for(&key(Key::Delete), false, true, false),
            Some(ShortcutAction::PermanentDelete)
        );
    }

    #[test]
    fn ctrl_shift_n_creates_a_folder() {
        assert_eq!(
            action_for("n", true, true, false),
            Some(ShortcutAction::CreateFolder)
        );
        assert_eq!(
            action_for("N", true, true, false),
            Some(ShortcutAction::CreateFolder)
        );
    }

    #[test]
    fn f5_is_refresh_and_distinct_from_f2() {
        assert_eq!(
            action_for(&key(Key::F5), false, false, false),
            Some(ShortcutAction::Refresh)
        );
        assert_ne!(key(Key::F5), key(Key::F2));
    }

    #[test]
    fn bare_letter_is_not_a_shortcut() {
        assert_eq!(action_for("c", false, false, false), None);
    }

    #[test]
    fn unmapped_combination_is_none() {
        assert_eq!(action_for("c", true, true, false), None);
    }

    #[test]
    fn non_key_text_is_none() {
        assert_eq!(action_for("ab", true, false, false), None);
        assert_eq!(action_for("", true, false, false), None);
    }
}
