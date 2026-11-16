//! Native properties sheet and "Open with" — Tranche 3 slice 3 (OPS-14, OPS-15).
//!
//! Spec §7: the properties sheet is `ShellExecuteExW` with `SEE_MASK_INVOKEIDLIST` and
//! the `properties` verb, which is modeless and parents to the owner `HWND`; "Open with"
//! is `SHOpenWithDialog` with an `OPENASINFO`, which is modal and parents the same way.
//!
//! **Multi-select.** The single-path verb does not extend to arrays, and the slice-0
//! spike row that was meant to settle the question came back INVALID (it passed absolute
//! ids to `GetCommandString`), so spec §15 still records it as open. This module answers
//! it with `SHMultiFileProperties`, the documented call that shows *one combined* sheet
//! for a set of files — the behaviour Explorer's multi-selection has. Whether the sheet
//! that call shows matches Explorer's has not been observed on this host (rule 8), so the
//! manual row stays NOT RUN in `docs/tranche-3-validation.md`.
//!
//! Neither call runs on a pool thread: both are Shell UI calls that need the owner
//! window's thread, and both pump their own messages while they are up. They are called
//! from the UI thread the way slice 1 shows the native menu there, which is why they are
//! not offloaded like a transfer (rule 4 governs I/O and computation, not a modal Shell
//! dialog the user just asked for).

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use windows::{
    Win32::{
        Foundation::HWND,
        UI::{
            Shell::{
                OAIF_ALLOW_REGISTRATION, OAIF_EXEC, OPENASINFO, SEE_MASK_INVOKEIDLIST,
                SHELLEXECUTEINFOW, SHMultiFileProperties, SHOpenWithDialog, ShellExecuteExW,
            },
            WindowsAndMessaging::SW_SHOWNORMAL,
        },
    },
    core::{PCWSTR, w},
};

/// Why a properties or "Open with" request did not reach the Shell.
#[derive(Debug)]
pub enum PropertiesError {
    /// Nothing was selected, so there is no sheet and no file to open.
    EmptySelection,
    /// The Shell refused the request and returned a failing `HRESULT`.
    Shell(windows::core::Error),
}

impl std::fmt::Display for PropertiesError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptySelection => write!(formatter, "nothing is selected"),
            Self::Shell(error) => write!(formatter, "the Shell refused the request: {error}"),
        }
    }
}

impl std::error::Error for PropertiesError {}

/// Which Shell call a selection needs (spec §7).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PropertiesMode {
    /// One path: `ShellExecuteExW` with the `properties` verb.
    Single,
    /// More than one path: one combined sheet through `SHMultiFileProperties`.
    Combined,
}

/// The call a selection of `len` paths needs, or `None` when there is nothing to show.
///
/// Pure, so the routing rule is tested without COM: an empty selection is refused rather
/// than opening the sheet of an unrelated folder, and the boundary between the verb and
/// the combined sheet sits at exactly one path.
pub fn properties_mode(len: usize) -> Option<PropertiesMode> {
    match len {
        0 => None,
        1 => Some(PropertiesMode::Single),
        _ => Some(PropertiesMode::Combined),
    }
}

/// Shows the Shell's properties sheet for `paths`, parented to `owner`.
///
/// The sheet is modeless: `ShellExecuteExW` returns while the sheet stays open, kept
/// alive by the owner window's message loop (validated in the slice-0 spike:
/// `properties_see` returned in 102 ms with the sheet still up).
///
/// # Errors
///
/// [`PropertiesError::EmptySelection`] when `paths` is empty, otherwise whatever the
/// Shell returned.
pub fn show_properties(owner: HWND, paths: &[PathBuf]) -> Result<(), PropertiesError> {
    match properties_mode(paths.len()) {
        None => Err(PropertiesError::EmptySelection),
        Some(PropertiesMode::Single) => {
            let file = wide_null(&paths[0]);
            let mut info = SHELLEXECUTEINFOW {
                cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
                // Without this mask the `properties` verb is not looked up in the item's
                // context menu, and ShellExecute falls back to opening the file.
                fMask: SEE_MASK_INVOKEIDLIST,
                hwnd: owner,
                lpVerb: w!("properties"),
                lpFile: PCWSTR(file.as_ptr()),
                nShow: SW_SHOWNORMAL.0,
                ..Default::default()
            };
            // SAFETY: `file` outlives the call and `info` is a fully initialised
            // SHELLEXECUTEINFOW with `cbSize` set as the API requires; `owner` is the
            // window the sheet parents to.
            unsafe { ShellExecuteExW(&mut info) }.map_err(PropertiesError::Shell)
        }
        Some(PropertiesMode::Combined) => {
            // The combined sheet reads the paths from a data object, not an array; the
            // drag source already builds exactly the CF_HDROP object the Shell expects.
            let data = crate::drop_source::paths_data_object(paths);
            // SAFETY: `data` is a live COM object for the duration of the call and the
            // Shell only reads from it. Flags are 0: the documented "one merged sheet"
            // behaviour is the default.
            unsafe { SHMultiFileProperties(&data, 0) }.map_err(PropertiesError::Shell)
        }
    }
}

/// Shows the Shell's "Open with" dialog for `path`, parented to `owner`.
///
/// The dialog is modal: `SHOpenWithDialog` returns only once the user has dismissed it
/// (validated in the slice-0 spike: it blocked for 11 292 ms), so this call belongs on
/// the UI thread and never inside the orchestrator's lock.
///
/// The flags are "open with" semantics rather than a bare dialog: `OAIF_ALLOW_REGISTRATION`
/// lets the user make the choice permanent, and `OAIF_EXEC` opens the file once a program
/// is chosen, which is what the entry promises. Only the dialog's appearance was observed
/// in the spike; the execute-on-choice half is unobserved here (rule 8).
///
/// # Errors
///
/// [`PropertiesError::Shell`] with whatever the Shell returned.
pub fn open_with(owner: HWND, path: &Path) -> Result<(), PropertiesError> {
    let file = wide_null(path);
    let info = OPENASINFO {
        pcszFile: PCWSTR(file.as_ptr()),
        // No ProgID class: the file's own type decides which applications are offered.
        pcszClass: PCWSTR::null(),
        oaifInFlags: OAIF_ALLOW_REGISTRATION | OAIF_EXEC,
    };
    // SAFETY: `file` outlives the call, `info` points at it and at no other borrowed
    // memory, and `owner` is the window the dialog parents to.
    unsafe { SHOpenWithDialog(Some(owner), &info) }.map_err(PropertiesError::Shell)
}

/// `path` as a NUL-terminated wide string, the form every Shell call above takes.
fn wide_null(path: &Path) -> Vec<u16> {
    OsStr::new(path.as_os_str())
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The routing rule the module documents: nothing for an empty selection, the verb for
    /// one path, and the combined sheet from two paths on.
    #[test]
    fn the_mode_follows_the_selection_size() {
        assert_eq!(properties_mode(0), None);
        assert_eq!(properties_mode(1), Some(PropertiesMode::Single));
        assert_eq!(properties_mode(2), Some(PropertiesMode::Combined));
        assert_eq!(properties_mode(9), Some(PropertiesMode::Combined));
    }

    /// An empty selection is a refusal, not a sheet for the current folder: the caller
    /// decides what an empty selection means, and guessing here would show the wrong item.
    #[test]
    fn an_empty_selection_is_refused_before_any_shell_call() {
        let error = show_properties(HWND::default(), &[]).expect_err("an empty selection");
        assert!(matches!(error, PropertiesError::EmptySelection));
        assert_eq!(error.to_string(), "nothing is selected");
    }
}
