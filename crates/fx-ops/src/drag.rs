//! Drag-and-drop semantics shared by every drop path.
//!
//! Tranche 3, slice 2 (spec decision 8): the Tranche 2 internal-drag semantics are
//! reused, not rewritten, and the OLE transport (§7) carries them. The rules are
//! Explorer's:
//!
//! * `Ctrl` copies.
//! * `Shift` moves.
//! * `Ctrl+Shift` creates a shortcut (13.9).
//! * No modifier follows the source's preference — but a move is downgraded to a copy
//!   when the source and the destination are on different volumes, because a move
//!   across volumes is not a single operation.
//!
//! The policy lives here, in the crate that has no UI and no COM, so the rules are
//! covered by the ordinary test suite instead of by a manual session.

use std::path::Path;

/// The modifier keys held while a drag is in progress.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DropModifiers {
    /// The control key is held.
    pub control: bool,
    /// The shift key is held.
    pub shift: bool,
}

/// What a drop does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropAction {
    /// The files are copied into the destination.
    Copy,
    /// The files are moved into the destination.
    Move,
    /// A shortcut to the files is created in the destination.
    Shortcut,
}

/// Resolves what a drop does.
///
/// `preferred` is the effect the drag source advertised — for an Explorer drag it is
/// Explorer's `Preferred DropEffect`, for a Ferrix drag it is the effect Ferrix
/// advertised. `same_volume` says whether the source and the destination live on the
/// same volume.
pub fn resolve_action(
    modifiers: DropModifiers,
    preferred: DropAction,
    same_volume: bool,
) -> DropAction {
    match (modifiers.control, modifiers.shift) {
        (true, true) => DropAction::Shortcut,
        (true, false) => DropAction::Copy,
        (false, true) => DropAction::Move,
        (false, false) => match preferred {
            DropAction::Copy => DropAction::Copy,
            // A link works across volumes, so it stays what the source asked for.
            DropAction::Shortcut => DropAction::Shortcut,
            // A move only stays a move inside one volume.
            DropAction::Move => {
                if same_volume {
                    DropAction::Move
                } else {
                    DropAction::Copy
                }
            }
        },
    }
}

/// Whether a drop targets the folder the dragged files already live in.
///
/// Tranche 2 rule 15: dropping rows back onto their own folder does nothing, and the
/// user is told so instead of seeing a no-op transfer.
pub fn is_same_folder_drop(source_parent: Option<&Path>, destination: &Path) -> bool {
    match source_parent {
        Some(parent) => parent == destination,
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn modifiers(control: bool, shift: bool) -> DropModifiers {
        DropModifiers { control, shift }
    }

    #[test]
    fn control_copies_whatever_the_source_preferred() {
        assert_eq!(
            resolve_action(modifiers(true, false), DropAction::Move, true),
            DropAction::Copy
        );
        assert_eq!(
            resolve_action(modifiers(true, false), DropAction::Copy, true),
            DropAction::Copy
        );
    }

    #[test]
    fn shift_moves_whatever_the_source_preferred() {
        assert_eq!(
            resolve_action(modifiers(false, true), DropAction::Copy, true),
            DropAction::Move
        );
        assert_eq!(
            resolve_action(modifiers(false, true), DropAction::Move, true),
            DropAction::Move
        );
    }

    #[test]
    fn control_and_shift_creates_a_shortcut() {
        assert_eq!(
            resolve_action(modifiers(true, true), DropAction::Move, true),
            DropAction::Shortcut
        );
        assert_eq!(
            resolve_action(modifiers(true, true), DropAction::Copy, false),
            DropAction::Shortcut
        );
    }

    #[test]
    fn without_a_modifier_the_source_preference_holds_inside_one_volume() {
        assert_eq!(
            resolve_action(modifiers(false, false), DropAction::Move, true),
            DropAction::Move
        );
        assert_eq!(
            resolve_action(modifiers(false, false), DropAction::Copy, true),
            DropAction::Copy
        );
        // A link is not a move: it survives a change of volume, because the shortcut
        // keeps pointing at the original file wherever it lives.
        assert_eq!(
            resolve_action(modifiers(false, false), DropAction::Shortcut, false),
            DropAction::Shortcut
        );
    }

    #[test]
    fn a_move_across_volumes_becomes_a_copy() {
        assert_eq!(
            resolve_action(modifiers(false, false), DropAction::Move, false),
            DropAction::Copy
        );
    }

    #[test]
    fn a_same_folder_drop_is_detected() {
        let folder = PathBuf::from(r"C:\folder");
        assert!(is_same_folder_drop(Some(&folder), Path::new(r"C:\folder")));
        assert!(!is_same_folder_drop(Some(&folder), Path::new(r"C:\other")));
        assert!(!is_same_folder_drop(None, Path::new(r"C:\folder")));
    }
}
