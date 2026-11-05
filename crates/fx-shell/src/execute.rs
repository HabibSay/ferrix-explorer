//! ShellExecuteW wrapper for double-click file opening.
//!
//! All unsafe code is confined to this module; each block carries a SAFETY comment.

use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::PCWSTR;

/// Open `path` with the default verb using `ShellExecuteW`.
///
/// Returns `Ok(())` when the shell reports success (return value > 32).
/// Returns `Err` with a Win32 error code otherwise.
pub fn shell_open(hwnd: HWND, path: &Path) -> windows::core::Result<()> {
    let path_wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let verb_wide: Vec<u16> = "open\0".encode_utf16().collect();

    // SAFETY: `hwnd` is a valid Win32 window handle on the UI thread (or NULL, which is also
    // acceptable via `Some(HWND(null_mut()))`). `verb_wide` and `path_wide` are null-terminated
    // UTF-16 strings that live for the duration of this call. `None` for parameters and directory
    // is explicitly documented as acceptable by the Win32 API.
    let result = unsafe {
        ShellExecuteW(
            Some(hwnd),
            PCWSTR(verb_wide.as_ptr()),
            PCWSTR(path_wide.as_ptr()),
            None,
            None,
            SW_SHOWNORMAL,
        )
    };

    // ShellExecuteW returns a pseudo-HINSTANCE; values > 32 indicate success.
    if result.0 as isize > 32 {
        Ok(())
    } else {
        // from_thread reads GetLastError() on the calling thread, valid after a failed Win32 call.
        Err(windows::core::Error::from_thread())
    }
}
