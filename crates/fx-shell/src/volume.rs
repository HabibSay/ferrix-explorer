//! Volume utilities: serial number, kind, free space, and atomic file replacement.
//!
//! All functions call Win32 directly; `fx-ops` must not depend on this crate.
//! The adapter in `fx-app/src/platform_ops.rs` converts between the local `VolumeKind`
//! defined here and `fx_ops::port::VolumeKind`.

use std::{io, path::Path};

use windows::{
    Win32::Storage::FileSystem::{
        GetDiskFreeSpaceExW, GetDriveTypeW, GetVolumeInformationW, REPLACEFILE_IGNORE_MERGE_ERRORS,
        ReplaceFileW,
    },
    core::PCWSTR,
};

/// What kind of volume a path lives on.
///
/// This is a local mirror of `fx_ops::port::VolumeKind`; `fx-shell` must not depend on
/// `fx-ops`. The adapter in `fx-app` converts between the two.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VolumeKind {
    Fixed,
    Removable,
    Network,
    Other,
}

/// Returns the volume serial number for the volume holding `path`.
///
/// Calls `GetVolumeInformationW` on the root of the path, which must be reachable.
pub fn get_volume_serial(path: &Path) -> io::Result<u64> {
    let root = volume_root(path)?;
    let root_wide: Vec<u16> = root.encode_utf16().chain(std::iter::once(0)).collect();

    let mut serial: u32 = 0;

    // SAFETY: root_wide is a NUL-terminated wide string for a valid root path.
    // lpVolumeSerialNumber, lpMaximumComponentLength, lpFileSystemFlags, lpVolumeNameBuffer,
    // and lpFileSystemNameBuffer are optional and set to null/0 when not needed.
    let ok = unsafe {
        GetVolumeInformationW(
            PCWSTR(root_wide.as_ptr()),
            None,
            Some(&mut serial),
            None,
            None,
            None,
        )
    };

    ok.map_err(io::Error::from)?;
    Ok(u64::from(serial))
}

/// Returns the kind of volume (fixed, removable, network, other) for the volume holding `path`.
///
/// Calls `GetDriveTypeW` on the root of the path.
pub fn get_volume_kind(path: &Path) -> io::Result<VolumeKind> {
    // Drive type constants for GetDriveTypeW.
    // Source: https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-getdrivetypew
    const DRIVE_REMOVABLE: u32 = 2;
    const DRIVE_FIXED: u32 = 3;
    const DRIVE_REMOTE: u32 = 4;

    let root = volume_root(path)?;
    let root_wide: Vec<u16> = root.encode_utf16().chain(std::iter::once(0)).collect();

    // SAFETY: root_wide is a NUL-terminated wide string; GetDriveTypeW returns a DRIVE_* constant.
    let drive_type = unsafe { GetDriveTypeW(PCWSTR(root_wide.as_ptr())) };

    let kind = match drive_type {
        DRIVE_FIXED => VolumeKind::Fixed,
        DRIVE_REMOVABLE => VolumeKind::Removable,
        DRIVE_REMOTE => VolumeKind::Network,
        _ => VolumeKind::Other,
    };
    Ok(kind)
}

/// Returns the number of bytes available to the current user on the volume holding `path`.
///
/// Calls `GetDiskFreeSpaceExW`. The returned value is the caller-available bytes
/// (may differ from total free bytes on a volume with per-user quotas).
pub fn get_free_space(path: &Path) -> io::Result<u64> {
    let mut available_to_caller: u64 = 0;

    let path_wide: Vec<u16> = path
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    // SAFETY: path_wide is a NUL-terminated wide string pointing to a valid path.
    // The output pointers are valid stack variables; the second and third are optional nulls.
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            PCWSTR(path_wide.as_ptr()),
            Some(&mut available_to_caller),
            None,
            None,
        )
    };

    ok.map_err(io::Error::from)?;
    Ok(available_to_caller)
}

/// Atomically replaces `destination` with `replacement` using `ReplaceFileW`.
///
/// If `destination` does not exist, falls back to `std::fs::rename` (which is atomic on
/// the same volume and NTFS).
pub fn replace_file_atomic(replacement: &Path, destination: &Path) -> io::Result<()> {
    if !destination.exists() {
        return std::fs::rename(replacement, destination);
    }

    let replacement_wide: Vec<u16> = replacement
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let destination_wide: Vec<u16> = destination
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    // SAFETY: both wide strings are NUL-terminated and point to valid paths.
    // lpBackupFileName is null (no backup). dwReplaceFlags uses a safe subset.
    let ok = unsafe {
        ReplaceFileW(
            PCWSTR(destination_wide.as_ptr()),
            PCWSTR(replacement_wide.as_ptr()),
            PCWSTR(std::ptr::null()),
            REPLACEFILE_IGNORE_MERGE_ERRORS,
            None,
            None,
        )
    };

    ok.map_err(io::Error::from)
}

/// Derives the volume root (e.g. `C:\`) from an arbitrary path.
///
/// For a UNC path (`\\server\share\…`) returns `\\server\share\`. For a drive-letter path
/// returns `X:\`. Falls back to the path itself when the prefix is not recognised.
pub(crate) fn volume_root(path: &Path) -> io::Result<String> {
    use std::path::Component;

    let mut components = path.components();
    match components.next() {
        Some(Component::Prefix(prefix)) => {
            use std::path::Prefix;
            match prefix.kind() {
                Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => {
                    Ok(format!("{}:\\", letter as char))
                }
                Prefix::UNC(server, share) | Prefix::VerbatimUNC(server, share) => {
                    let server = server.to_string_lossy();
                    let share = share.to_string_lossy();
                    Ok(format!("\\\\{server}\\{share}\\"))
                }
                _ => Ok(path.to_string_lossy().into_owned()),
            }
        }
        _ => {
            // Relative path or unusual root — use the path as-is.
            Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "cannot determine volume root from relative path",
            ))
        }
    }
}
