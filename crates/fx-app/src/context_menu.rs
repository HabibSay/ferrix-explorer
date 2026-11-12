//! The Ferrix context-menu registry and the native/fallback policy (spec §4, §8).
//!
//! This module owns the mapping between a Ferrix command and the label it shows, the
//! timeout budget, and the fallback: whenever the native menu cannot be shown, the
//! minimal Ferrix menu is shown instead, carrying the same command registry so the user
//! is never left without a menu (spec §8.7).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use fx_settings::view_state::MenuExtensionState;
use fx_shell::context_menu::{FerrixCommand, FerrixMenuItem, MenuHost, MenuOutcome, MenuRequest};
use windows::Win32::Foundation::HWND;

use crate::strings;

/// Budget for the native menu to appear (spec §8.2).
///
/// One constant, tunable when the slice-0 measurements speak. The budget covers the
/// *query* phase only; an open menu is unbounded by design (spec §6).
pub const MENU_TIMEOUT: Duration = Duration::from_secs(2);

/// Which thread runs the Shell interaction (spec decision 4).
///
/// `CallerThread` keeps the popup on the UI thread, the same thread that shows the
/// app-local menu, so activation and keyboard navigation behave the way the user
/// expects. `HelperThread` moves the whole interaction to a dedicated STA thread and
/// lets the UI thread watch the budget, at the cost of a cross-thread popup whose
/// activation the slice-0 spike was meant to settle. See
/// `docs/tranche-3-validation.md`.
pub const MENU_HOST: MenuHost = MenuHost::CallerThread;

/// The Ferrix section of the menu: the labels the user sees, in display order.
///
/// The labels live in `resources/en-US.rs`; the pairing of label and command lives
/// here, so the native menu and the minimal fallback menu always offer the same
/// commands in the same order.
///
/// `has_selection` decides whether the checksums entry appears: checksumming nothing
/// makes no sense, and the app-local menu has always hidden it in that case.
pub fn ferrix_items(has_selection: bool) -> Vec<FerrixMenuItem> {
    let mut items = Vec::with_capacity(7);
    if has_selection {
        items.push(FerrixMenuItem {
            command: FerrixCommand::Checksums,
            label: strings::ACTIVITY_CHECKSUMS.to_string(),
        });
    }
    items.push(FerrixMenuItem {
        command: FerrixCommand::SelectByPattern,
        label: strings::MENU_SELECT_BY_PATTERN.to_string(),
    });
    items.push(FerrixMenuItem {
        command: FerrixCommand::InvertSelection,
        label: strings::MENU_INVERT_SELECTION.to_string(),
    });
    items.push(FerrixMenuItem {
        command: FerrixCommand::NewFile,
        label: strings::MENU_NEW_FILE.to_string(),
    });
    items.push(FerrixMenuItem {
        command: FerrixCommand::CleanEmptyFolders,
        label: strings::ACTIVITY_CLEAN.to_string(),
    });
    items.push(FerrixMenuItem {
        command: FerrixCommand::CopyPath,
        label: strings::MENU_COPY_PATH.to_string(),
    });
    items.push(FerrixMenuItem {
        command: FerrixCommand::CopyName,
        label: strings::MENU_COPY_NAME.to_string(),
    });
    items
}

/// What the user chose in the context menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuChoice {
    /// A Ferrix command was chosen and must be run.
    Ferrix(FerrixCommand),
    /// A Shell verb was chosen and the Shell already ran it.
    Shell { verb: String },
    /// The chosen extension is turned off in the local list and was not run.
    Blocked { verb: String },
    /// The menu was dismissed without a choice.
    Dismissed,
}

/// The result of showing the menu.
#[derive(Clone, Debug)]
pub struct MenuResult {
    /// The command the user chose, if any.
    pub choice: MenuChoice,
    /// True when the native menu was unavailable and the minimal Ferrix menu was shown.
    pub used_fallback: bool,
}

/// Shows the context menu for a selection, or for the folder background.
///
/// `selection` empty means the background of `background` was clicked. When the native
/// menu cannot be shown for any reason, the minimal Ferrix menu is shown instead, with
/// the same command registry (spec §8.7).
pub fn show(
    owner: HWND,
    cursor: (i32, i32),
    selection: Vec<PathBuf>,
    background: Option<PathBuf>,
    extensions: &MenuExtensionState,
) -> MenuResult {
    let items = ferrix_items(!selection.is_empty());

    if extensions.native_menu_enabled {
        let request = MenuRequest {
            owner,
            cursor,
            selection,
            background,
            ferrix_items: items.clone(),
            blocked_verbs: extensions.blocked_extensions.clone(),
        };
        match fx_shell::context_menu::show_native_menu(request, MENU_HOST, MENU_TIMEOUT) {
            Ok((outcome, timings)) => {
                tracing::info!(
                    target: "menu",
                    query_ms = timings.query.as_millis() as u64,
                    visible_ms = timings.visible.as_millis() as u64,
                    items = timings.items,
                    source = "shell",
                );
                return MenuResult {
                    choice: match outcome {
                        MenuOutcome::Ferrix { command } => MenuChoice::Ferrix(command),
                        MenuOutcome::Shell { verb } => MenuChoice::Shell { verb },
                        MenuOutcome::Cancelled => MenuChoice::Dismissed,
                        MenuOutcome::Blocked { verb } => MenuChoice::Blocked { verb },
                    },
                    used_fallback: false,
                };
            }
            Err(error) => {
                tracing::warn!(target: "menu", ?error, "native menu unavailable; falling back");
            }
        }
    } else {
        tracing::info!(target: "menu", source = "disabled", "native menu disabled in the settings");
    }

    // The minimal Ferrix menu: the same commands, no Shell verbs (spec §8.7).
    let labels: Vec<&str> = items.iter().map(|item| item.label.as_str()).collect();
    let started = Instant::now();
    let chosen = fx_shell::show_popup_menu(owner, cursor.0, cursor.1, &labels);
    let visible_ms = started.elapsed().as_millis() as u64;
    tracing::info!(
        target: "menu",
        query_ms = 0,
        visible_ms,
        items = items.len(),
        source = "ferrix_fallback",
    );
    MenuResult {
        choice: match chosen {
            Ok(Some(index)) => match items.get(index) {
                Some(item) => MenuChoice::Ferrix(item.command),
                None => MenuChoice::Dismissed,
            },
            _ => MenuChoice::Dismissed,
        },
        used_fallback: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fx_shell::context_menu::FerrixCommand;

    /// The registry maps every command to exactly one label, and no two commands share
    /// one: the fallback menu and the native section must offer the same commands.
    #[test]
    fn the_registry_covers_every_command_once() {
        let items = ferrix_items(true);
        let mut seen: Vec<FerrixCommand> = Vec::new();
        for item in &items {
            assert!(!item.label.is_empty(), "a Ferrix entry has no label");
            assert!(
                !seen.contains(&item.command),
                "the command {:?} appears twice",
                item.command
            );
            seen.push(item.command);
        }
        assert_eq!(seen.len(), 7);
    }

    /// Without a selection there is nothing to checksum, so that entry is hidden — the
    /// behaviour the app-local menu has always had.
    #[test]
    fn the_checksums_entry_appears_only_with_a_selection() {
        let with = ferrix_items(true);
        let without = ferrix_items(false);
        assert!(
            with.iter()
                .any(|item| item.command == FerrixCommand::Checksums)
        );
        assert!(
            !without
                .iter()
                .any(|item| item.command == FerrixCommand::Checksums)
        );
        assert_eq!(with.len() - 1, without.len());
    }

    /// The labels come from `resources/en-US.rs`, so a translation cannot change what a
    /// menu click does.
    #[test]
    fn the_labels_come_from_the_resource_file() {
        let items = ferrix_items(true);
        let labels: Vec<&str> = items.iter().map(|item| item.label.as_str()).collect();
        assert_eq!(labels[0], strings::ACTIVITY_CHECKSUMS);
        assert_eq!(labels[1], strings::MENU_SELECT_BY_PATTERN);
        assert_eq!(labels[2], strings::MENU_INVERT_SELECTION);
        assert_eq!(labels[3], strings::MENU_NEW_FILE);
        assert_eq!(labels[4], strings::ACTIVITY_CLEAN);
        assert_eq!(labels[5], strings::MENU_COPY_PATH);
        assert_eq!(labels[6], strings::MENU_COPY_NAME);
    }
}
