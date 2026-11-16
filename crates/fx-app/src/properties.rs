//! Ferrix's side of the native properties sheet and "Open with" (Tranche 3 slice 3).
//!
//! The Shell calls live in `fx_shell::properties`; this module resolves the owner window,
//! keeps user-visible text in `resources/en-US.rs`, and reports a failure in the status
//! line instead of dropping a `HRESULT` on the floor.
//!
//! Both calls are Shell UI calls that need the owning window's thread and pump their own
//! messages, so they run here on the UI thread — the same place slice 1 shows the native
//! menu — and never inside the orchestrator's lock.

use std::path::{Path, PathBuf};

use fx_ui::ApplicationWindow;

use crate::strings;

/// Shows the properties sheet for `paths`, parented to the application's window.
///
/// Returns `true` when the Shell accepted the request. The sheet itself is modeless, so
/// `true` means "shown", not "closed".
pub fn show_properties(window: &ApplicationWindow, paths: &[PathBuf]) -> bool {
    let owner = match crate::get_hwnd(window) {
        Ok(hwnd) => hwnd,
        Err(error) => {
            tracing::warn!(target: "properties", %error, "no window handle for the sheet");
            window.set_status_message(slint::SharedString::from(strings::STATUS_OPERATION_FAILED));
            return false;
        }
    };
    match fx_shell::show_properties(owner, paths) {
        Ok(()) => {
            tracing::info!(target: "properties", count = paths.len(), "properties sheet shown");
            true
        }
        Err(error) => {
            tracing::warn!(target: "properties", %error, "properties sheet refused");
            window.set_status_message(slint::SharedString::from(strings::STATUS_OPERATION_FAILED));
            false
        }
    }
}

/// Shows the Shell's "Open with" dialog for `path`, parented to the application's window.
///
/// The call is modal, so this returns once the user has dismissed the dialog — never from
/// inside a lock or a callback that the UI needs to service.
pub fn open_with(window: &ApplicationWindow, path: &Path) -> bool {
    let owner = match crate::get_hwnd(window) {
        Ok(hwnd) => hwnd,
        Err(error) => {
            tracing::warn!(target: "properties", %error, "no window handle for Open with");
            window.set_status_message(slint::SharedString::from(strings::STATUS_OPERATION_FAILED));
            return false;
        }
    };
    match fx_shell::open_with(owner, path) {
        Ok(()) => {
            tracing::info!(target: "properties", path = %path.display(), "open with dismissed");
            true
        }
        Err(error) => {
            tracing::warn!(target: "properties", %error, path = %path.display(), "open with refused");
            window.set_status_message(slint::SharedString::from(strings::STATUS_OPERATION_FAILED));
            false
        }
    }
}
