//! Reparse-point classification and symbolic link creation.
//!
//! `std` can tell "this is a link" from "this is not", but not a symbolic link from a
//! junction — that distinction needs the reparse tag, read with
//! `FSCTL_GET_REPARSE_POINT`. Spec decision 6: symbolic links are recreated, junctions
//! stay skipped, so the tag is the whole reason this code exists.

use std::{ffi::c_void, io, iter, path::Path};

use windows::{
    Win32::{
        Foundation::CloseHandle,
        Storage::FileSystem::{
            CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
            FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
            OPEN_EXISTING,
        },
        System::{IO::DeviceIoControl, Ioctl::FSCTL_GET_REPARSE_POINT},
    },
    core::PCWSTR,
};

/// `IO_REPARSE_TAG_SYMLINK`: the only tag Windows uses for a real symbolic link.
pub const IO_REPARSE_TAG_SYMLINK: u32 = 0xA000_000C;

/// `IO_REPARSE_TAG_MOUNT_POINT`: junctions and mounted volumes.
pub const IO_REPARSE_TAG_MOUNT_POINT: u32 = 0xA000_0003;

/// `ERROR_NOT_A_REPARSE_POINT` — the ordinary answer for a file that is not a link.
const ERROR_NOT_A_REPARSE_POINT: i32 = 4390;

/// The reparse tag of `path`, or `None` when it is not a reparse point.
pub fn reparse_tag(path: &Path) -> io::Result<Option<u32>> {
    let wide: Vec<u16> = path
        .to_string_lossy()
        .encode_utf16()
        .chain(iter::once(0))
        .collect();

    // SAFETY: `wide` is a NUL-terminated UTF-16 path that outlives the call; the open is
    // read-only on attributes, so no write access is requested.
    let handle = unsafe {
        CreateFileW(
            PCWSTR(wide.as_ptr()),
            FILE_READ_ATTRIBUTES.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS,
            None,
        )
    };
    let Ok(handle) = handle else {
        // A directory needs FILE_FLAG_BACKUP_SEMANTICS, which is set above; anything else
        // is reported unchanged so the caller can distinguish "denied" from "absent".
        return Err(io::Error::last_os_error());
    };

    // Only the first four bytes (the tag) are read, but the buffer has to be large enough
    // for the smallest accepted reparse buffer, and generous enough not to truncate the
    // common tags.
    let mut buffer = [0u8; 2048];
    let mut returned = 0u32;

    // SAFETY: `handle` is open for the duration; `buffer` is 2048 writable bytes and the
    // length passed matches; `returned` is a live u32.
    let outcome = unsafe {
        DeviceIoControl(
            handle,
            FSCTL_GET_REPARSE_POINT,
            None,
            0,
            Some(buffer.as_mut_ptr() as *mut c_void),
            buffer.len() as u32,
            Some(&mut returned),
            None,
        )
    };
    // SAFETY: `handle` came from CreateFileW on this thread and is closed exactly once.
    let _ = unsafe { CloseHandle(handle) };

    if outcome.is_err() {
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(ERROR_NOT_A_REPARSE_POINT) {
            return Ok(None);
        }
        return Err(error);
    }
    if returned < 4 {
        return Ok(None);
    }
    Ok(Some(u32::from_le_bytes([
        buffer[0], buffer[1], buffer[2], buffer[3],
    ])))
}

/// Creates a symbolic link at `link` pointing at `target`.
///
/// Requires the SeCreateSymbolicLink privilege or Developer Mode; otherwise Windows fails
/// the call with `ERROR_PRIVILEGE_NOT_HELD` (1314) and the engine reports the item failed.
pub fn create_link(target: &Path, link: &Path, is_dir: bool) -> io::Result<()> {
    if is_dir {
        std::os::windows::fs::symlink_dir(target, link)
    } else {
        std::os::windows::fs::symlink_file(target, link)
    }
}
