//! File-time helpers beyond what `std` can do (setting creation time).

use std::{fs, io, os::windows::io::AsRawHandle, path::Path};

use windows::Win32::{
    Foundation::{FILETIME, HANDLE},
    Storage::FileSystem::SetFileTime,
};

/// Sets the creation time of `to` to match `from` using `SetFileTime`.
/// `std::fs` exposes creation time for reading only.
pub fn set_creation_time(from: &Path, to: &Path) -> io::Result<()> {
    let from_meta = fs::metadata(from)?;
    let created = from_meta.created()?;

    let filetime = system_time_to_filetime(created)?;

    // Open write access through std and borrow the raw handle, avoiding a
    // dependency on the Win32_Security feature.
    let file = fs::OpenOptions::new().write(true).open(to)?;
    let raw_handle = file.as_raw_handle();

    // SAFETY: raw_handle is the valid handle owned by `file`, which stays alive for
    // the whole call; `&filetime` points to a live in-process value. SetFileTime
    // accepts a write handle and a nullable structure we no longer touch.
    let result = unsafe { SetFileTime(HANDLE(raw_handle as *mut _), Some(&filetime), None, None) };

    result.map_err(io::Error::from)
}

/// Converts `std::time::SystemTime` to a Windows `FILETIME`.
fn system_time_to_filetime(time: std::time::SystemTime) -> io::Result<FILETIME> {
    use std::time::UNIX_EPOCH;

    // Windows FILETIME epoch is 1601-01-01; Unix epoch is 1970-01-01;
    // the offset counted in 100-ns intervals.
    const EPOCH_DIFF: u64 = 116_444_736_000_000_000;

    let duration = time
        .duration_since(UNIX_EPOCH)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "system time before Unix epoch"))?;

    let intervals = duration.as_nanos() as u64 / 100 + EPOCH_DIFF;

    Ok(FILETIME {
        dwLowDateTime: intervals as u32,
        dwHighDateTime: (intervals >> 32) as u32,
    })
}
