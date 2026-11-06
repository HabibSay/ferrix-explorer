//! Windows clipboard interop: CF_HDROP + Preferred DropEffect read/write.
//!
//! Write: opens the clipboard with retries, sets CF_HDROP (DROPFILES + UTF-16 paths),
//! and sets the registered "Preferred DropEffect" format. The OS takes ownership of
//! the allocated memory; we never free it.
//!
//! Read: DragQueryFileW extracts paths, the effect format is read if present.

use std::{ffi::OsString, os::windows::ffi::OsStringExt, path::PathBuf, thread, time::Duration};

use windows::{
    Win32::{
        Foundation::{GlobalFree, HANDLE, HGLOBAL},
        System::{
            DataExchange::{
                CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard,
                RegisterClipboardFormatW, SetClipboardData,
            },
            Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock},
            Ole::CF_HDROP,
        },
        UI::Shell::{DragQueryFileW, HDROP},
    },
    core::PCWSTR,
};

/// Maps to `DROPEFFECT_COPY` (1) or `DROPEFFECT_MOVE` (2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipboardEffect {
    Copy,
    Move,
}

impl ClipboardEffect {
    fn dropeffect_value(self) -> u32 {
        match self {
            Self::Copy => 1, // DROPEFFECT_COPY
            Self::Move => 2, // DROPEFFECT_MOVE
        }
    }

    fn from_dropeffect(value: u32) -> Self {
        if value == 2 { Self::Move } else { Self::Copy }
    }
}

/// DROPFILES structure layout (matching Windows DROPFILES).
#[repr(C)]
struct DropFiles {
    /// Offset of file list from start of this structure.
    p_files: u32,
    pt_x: i32,
    pt_y: i32,
    /// Non-client mouse position flag (unused; set to 0).
    f_nc: u32,
    /// TRUE if the file list is Unicode (UTF-16).
    f_wide: u32,
}

/// Writes `paths` to the clipboard as CF_HDROP + Preferred DropEffect.
///
/// Opens the clipboard on `NULL` (no window owner) with up to 5 retries over ~100 ms,
/// which is the standard pattern for a contested clipboard. The memory handed to
/// `SetClipboardData` is owned by Windows after the call; we never free it.
pub fn write_clipboard(paths: &[PathBuf], effect: ClipboardEffect) -> windows::core::Result<()> {
    // ── Build the UTF-16 path block ──────────────────────────────────────────
    // Layout: for every path, encode UTF-16, append a NUL, then append a final NUL.
    let mut wide_paths: Vec<u16> = Vec::new();
    for path in paths {
        for unit in path.to_string_lossy().encode_utf16() {
            wide_paths.push(unit);
        }
        wide_paths.push(0); // path-terminating NUL
    }
    wide_paths.push(0); // double-NUL terminator

    let header_size = std::mem::size_of::<DropFiles>();
    let path_bytes = wide_paths.len() * std::mem::size_of::<u16>();
    let total_size = header_size + path_bytes;

    // ── Allocate moveable global memory ─────────────────────────────────────
    // SAFETY: GlobalAlloc with GMEM_MOVEABLE and a positive size is always valid.
    let h_global: HGLOBAL = unsafe { GlobalAlloc(GMEM_MOVEABLE, total_size)? };

    // SAFETY: We just allocated h_global; GlobalLock returns a writeable pointer.
    let ptr = unsafe { GlobalLock(h_global) };
    if ptr.is_null() {
        // SAFETY: h_global is a valid HGLOBAL that we own; free it to avoid a leak.
        unsafe {
            let _ = GlobalFree(Some(h_global));
        }
        return Err(windows::core::Error::from_thread());
    }

    // Write the DROPFILES header.
    // SAFETY: ptr points to `total_size` bytes of writeable memory; we write within bounds.
    unsafe {
        let header = ptr as *mut DropFiles;
        (*header).p_files = header_size as u32;
        (*header).pt_x = 0;
        (*header).pt_y = 0;
        (*header).f_nc = 0;
        (*header).f_wide = 1; // TRUE — Unicode paths
    }

    // Write the UTF-16 path data after the header.
    // SAFETY: `ptr` is valid and the offset stays within `total_size` bytes.
    unsafe {
        let dest = (ptr as *mut u8).add(header_size) as *mut u16;
        std::ptr::copy_nonoverlapping(wide_paths.as_ptr(), dest, wide_paths.len());
    }

    // SAFETY: h_global was successfully locked; unlocking it is always valid after a lock.
    let _ = unsafe { GlobalUnlock(h_global) };

    // ── Open clipboard with retries ─────────────────────────────────────────
    let mut opened = false;
    for _ in 0..5 {
        // SAFETY: None HWND is valid for a clipboard open with no window owner.
        if unsafe { OpenClipboard(None).is_ok() } {
            opened = true;
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }
    if !opened {
        // SAFETY: h_global is a valid HGLOBAL that we still own (clipboard was not opened).
        unsafe {
            let _ = GlobalFree(Some(h_global));
        }
        return Err(windows::core::Error::from_thread());
    }

    // SAFETY: Clipboard is open on this thread.
    let empty_result = unsafe { EmptyClipboard() };
    if empty_result.is_err() {
        // SAFETY: Clipboard is open; close it on error.
        unsafe {
            let _ = CloseClipboard();
        }
        // SAFETY: h_global still owned by us (EmptyClipboard failed before SetClipboardData).
        unsafe {
            let _ = GlobalFree(Some(h_global));
        }
        return empty_result;
    }

    // SetClipboardData for CF_HDROP transfers ownership of h_global to Windows.
    // After this call we MUST NOT free h_global.
    // SAFETY: h_global contains a valid CF_HDROP block; clipboard is open.
    let set_result = unsafe { SetClipboardData(CF_HDROP.0 as u32, Some(HANDLE(h_global.0))) };
    if let Err(err) = set_result {
        // SAFETY: Clipboard is open; close before returning.
        unsafe {
            let _ = CloseClipboard();
        }
        return Err(err);
    }
    // h_global is now owned by Windows — do not free it.

    // ── Write the Preferred DropEffect format ────────────────────────────────
    // Allocate a DWORD for the effect value and hand it to the clipboard.
    let effect_name: Vec<u16> = "Preferred DropEffect\0".encode_utf16().collect();
    // SAFETY: effect_name is a valid, NUL-terminated wide string.
    let format_id = unsafe { RegisterClipboardFormatW(PCWSTR(effect_name.as_ptr())) };
    if format_id != 0 {
        // SAFETY: GlobalAlloc with a positive size is always valid.
        if let Ok(h_effect) = unsafe { GlobalAlloc(GMEM_MOVEABLE, std::mem::size_of::<u32>()) } {
            // SAFETY: h_effect is valid; lock it to write the DWORD.
            let effect_ptr = unsafe { GlobalLock(h_effect) };
            if !effect_ptr.is_null() {
                // SAFETY: effect_ptr points to at least sizeof::<u32>() bytes.
                unsafe { *(effect_ptr as *mut u32) = effect.dropeffect_value() };
                // SAFETY: h_effect was locked above; unlock before SetClipboardData.
                let _ = unsafe { GlobalUnlock(h_effect) };
                // Transfer ownership to Windows.
                // SAFETY: h_effect contains a valid effect DWORD; clipboard is open.
                let _ = unsafe { SetClipboardData(format_id, Some(HANDLE(h_effect.0))) };
                // Do not free h_effect — Windows owns it now.
            } else {
                // SAFETY: h_effect still owned by us; lock failed so we free it.
                unsafe {
                    let _ = GlobalFree(Some(h_effect));
                }
            }
        }
    }

    // SAFETY: Clipboard is open; close after writing all formats.
    unsafe { CloseClipboard() }
}

/// Reads `CF_HDROP` paths and the optional `Preferred DropEffect` from the clipboard.
///
/// Returns absolute paths only. Returns `ClipboardEffect::Copy` when the effect format
/// is absent (Explorer's default for a plain copy). Returns `Ok(None)` when the clipboard
/// holds no file drop data.
pub fn read_clipboard() -> windows::core::Result<Option<(Vec<PathBuf>, ClipboardEffect)>> {
    // SAFETY: None HWND is valid; we open the clipboard for reading.
    unsafe { OpenClipboard(None)? };

    let result = read_clipboard_inner();

    // SAFETY: Clipboard was opened successfully above; close unconditionally.
    unsafe {
        let _ = CloseClipboard();
    }

    result
}

fn read_clipboard_inner() -> windows::core::Result<Option<(Vec<PathBuf>, ClipboardEffect)>> {
    // SAFETY: CF_HDROP.0 is a valid format; the handle is owned by the clipboard.
    let hdrop_handle = match unsafe { GetClipboardData(CF_HDROP.0 as u32) } {
        Ok(h) => h,
        Err(_) => return Ok(None),
    };

    // Cast to HDROP for DragQueryFileW.
    let hdrop = HDROP(hdrop_handle.0);

    // SAFETY: hdrop is a valid HDROP obtained from an open clipboard.
    let file_count = unsafe { DragQueryFileW(hdrop, u32::MAX, None) };
    if file_count == 0 {
        return Ok(None);
    }

    let mut paths = Vec::with_capacity(file_count as usize);
    for i in 0..file_count {
        // First call: get the required buffer length (including NUL).
        // SAFETY: hdrop is valid; passing None queries the length.
        let len = unsafe { DragQueryFileW(hdrop, i, None) } as usize + 1;
        let mut buf = vec![0u16; len];
        // SAFETY: hdrop is valid; buf is large enough for the returned path.
        let written = unsafe { DragQueryFileW(hdrop, i, Some(&mut buf)) } as usize;
        if written == 0 {
            continue;
        }
        let os_str = OsString::from_wide(&buf[..written]);
        let path = PathBuf::from(os_str);
        // Absolute paths only — skip relative entries (which should not appear in practice).
        if path.is_absolute() {
            paths.push(path);
        }
    }

    if paths.is_empty() {
        return Ok(None);
    }

    // ── Read Preferred DropEffect ────────────────────────────────────────────
    let effect_name: Vec<u16> = "Preferred DropEffect\0".encode_utf16().collect();
    // SAFETY: effect_name is a valid, NUL-terminated wide string.
    let format_id = unsafe { RegisterClipboardFormatW(PCWSTR(effect_name.as_ptr())) };
    let effect = if format_id != 0 {
        // SAFETY: format_id is a registered format; the returned handle is clipboard-owned.
        match unsafe { GetClipboardData(format_id) } {
            Ok(h) if !h.0.is_null() => {
                let hglobal = HGLOBAL(h.0);
                // SAFETY: hglobal is valid and clipboard-owned; lock to read the DWORD.
                let ptr = unsafe { GlobalLock(hglobal) };
                let value = if !ptr.is_null() {
                    // SAFETY: ptr points to at least sizeof::<u32>() bytes.
                    let v = unsafe { *(ptr as *const u32) };
                    // SAFETY: hglobal was locked above.
                    let _ = unsafe { GlobalUnlock(hglobal) };
                    v
                } else {
                    1 // DROPEFFECT_COPY
                };
                ClipboardEffect::from_dropeffect(value)
            }
            _ => ClipboardEffect::Copy,
        }
    } else {
        ClipboardEffect::Copy
    };

    Ok(Some((paths, effect)))
}

/// Empties the clipboard, matching Explorer's cut tracking: after a successful
/// move-paste the stale cut entries must not be pasteable again.
///
/// Must be called off the UI thread like any clipboard write — it contends for the
/// same global lock and retries.
pub fn clear_clipboard() -> windows::core::Result<()> {
    for _ in 0..5 {
        // SAFETY: None HWND is valid for a clipboard open with no window owner;
        // EmptyClipboard is only called while this thread holds the open clipboard
        // and CloseClipboard runs on every path afterwards.
        let opened = unsafe { OpenClipboard(None) };
        if opened.is_ok() {
            let empty = unsafe { EmptyClipboard() };
            unsafe {
                let _ = CloseClipboard();
            }
            return empty;
        }
        thread::sleep(Duration::from_millis(20));
    }
    Err(windows::core::Error::from_thread())
}
