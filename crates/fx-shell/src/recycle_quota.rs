//! Recycle Bin quota pre-check (spec §8: volume kind plus the bin's remaining quota).
//!
//! No public API returns the Recycle Bin's configured maximum: `SHQueryRecycleBinW` only
//! reports the size and item count currently in the bin, and Explorer keeps the maximum in
//! its own registry settings, where the unit of `MaxCapacity` is not documented. The
//! pre-check therefore refuses only when the payload cannot fit **even under the most
//! generous reading** of that value — a wrong unit can make us skip the refusal, never
//! refuse a recycle that would have worked. `FOF_WANTNUKEWARNING` stays as the net.

use std::{ffi::c_void, io, iter, path::Path};

use windows::{
    Win32::{
        System::Registry::{
            HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RegGetValueW,
        },
        UI::Shell::{SHQUERYRBINFO, SHQueryRecycleBinW},
    },
    core::PCWSTR,
};

/// What the Shell and the registry say about one volume's Recycle Bin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecycleBinUsage {
    /// Bytes currently held by recycled items on this volume.
    pub size_bytes: u64,
    pub item_count: u64,
    /// Configured maximum. `None` when no `MaxCapacity` value is present, which is the
    /// system-managed case and never justifies a refusal.
    pub max_bytes: Option<u64>,
}

/// Every registry location that has been used for the per-volume `MaxCapacity` across
/// Windows versions. All are read and the largest value wins, for the same reason the
/// unit is read generously.
const MAX_CAPACITY_LOCATIONS: [(HKEY, &str); 3] = [
    (
        HKEY_CURRENT_USER,
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\BitBucket\Volume",
    ),
    (
        HKEY_CURRENT_USER,
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\BitBucket",
    ),
    (
        HKEY_LOCAL_MACHINE,
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\BitBucket\Policies",
    ),
];

/// The largest byte count `MaxCapacity` could mean: the MB reading, the most generous of
/// the units in circulation (bytes, sectors, KB, MB).
///
/// Saturating, because a wrapped `u32::MAX` would read as a tiny quota and turn a
/// recyclable deletion into a refused one.
pub fn generous_capacity(raw: u32) -> u64 {
    u64::from(raw).saturating_mul(1024 * 1024)
}

/// True only when `payload_bytes` cannot fit even under the most generous reading of the
/// configured maximum.
pub fn exceeds_generous_quota(payload_bytes: u64, max_bytes: Option<u64>) -> bool {
    payload_bytes > max_bytes.unwrap_or(u64::MAX)
}

/// Queries the Recycle Bin of the volume holding `path` (`SHQueryRecycleBinW`).
pub fn query_recycle_bin(path: &Path) -> io::Result<RecycleBinUsage> {
    let root = crate::volume::volume_root(path)?;
    let root_wide: Vec<u16> = root.encode_utf16().chain(iter::once(0)).collect();

    let mut info = SHQUERYRBINFO {
        cbSize: std::mem::size_of::<SHQUERYRBINFO>() as u32,
        i64Size: 0,
        i64NumItems: 0,
    };

    // SAFETY: `cbSize` is initialised before the call, `&mut info` is a live SHQUERYRBINFO
    // for its duration, and `root_wide` is a NUL-terminated wide string of a volume root.
    unsafe { SHQueryRecycleBinW(PCWSTR(root_wide.as_ptr()), &mut info) }
        .map_err(io::Error::from)?;

    Ok(RecycleBinUsage {
        size_bytes: u64::try_from(info.i64Size).unwrap_or(0),
        item_count: u64::try_from(info.i64NumItems).unwrap_or(0),
        max_bytes: max_capacity(&root),
    })
}

/// Largest `MaxCapacity` found for this volume root, converted generously.
fn max_capacity(root: &str) -> Option<u64> {
    let mut largest: Option<u32> = None;
    for (hive, parent) in MAX_CAPACITY_LOCATIONS {
        let subkey = format!(r"{parent}\{root}");
        if let Some(raw) = read_dword(hive, &subkey, "MaxCapacity") {
            largest = Some(largest.map_or(raw, |current| current.max(raw)));
        }
    }
    largest.map(generous_capacity)
}

fn read_dword(hive: HKEY, subkey: &str, value: &str) -> Option<u32> {
    let subkey_wide: Vec<u16> = subkey.encode_utf16().chain(iter::once(0)).collect();
    let value_wide: Vec<u16> = value.encode_utf16().chain(iter::once(0)).collect();

    let mut data: u32 = 0;
    let mut size = u32::try_from(std::mem::size_of::<u32>()).expect("u32 size fits u32");

    // SAFETY: both names are NUL-terminated wide strings; `data` and `size` are live for
    // the call, and `size` starts at exactly `size_of::<u32>()`, so a DWORD cannot be
    // written past the buffer. A larger value type fails the `RRF_RT_REG_DWORD` filter.
    let status = unsafe {
        RegGetValueW(
            hive,
            PCWSTR(subkey_wide.as_ptr()),
            PCWSTR(value_wide.as_ptr()),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut data as *mut u32 as *mut c_void),
            Some(&mut size),
        )
    };
    if status.is_ok() { Some(data) } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capacity_is_read_in_the_most_generous_unit() {
        // 40 000 "units" is 40 GB if the unit is MB and 40 MB if it is KB: we must use GB,
        // because the smaller reading could refuse a recycle that Windows would allow.
        assert_eq!(generous_capacity(40_000), 40_000 * 1024 * 1024);
    }

    #[test]
    fn capacity_does_not_wrap_into_a_smaller_quota() {
        // The largest DWORD must still read as the largest quota, not as a small number.
        assert_eq!(
            generous_capacity(u32::MAX),
            u64::from(u32::MAX) * 1024 * 1024
        );
        assert!(generous_capacity(u32::MAX) < u64::MAX);
        assert!(generous_capacity(u32::MAX) > generous_capacity(u32::MAX - 1));
    }

    #[test]
    fn an_unknown_maximum_never_refuses() {
        assert!(!exceeds_generous_quota(u64::MAX, None));
    }

    #[test]
    fn only_a_clear_overflow_refuses() {
        let max = generous_capacity(100); // 100 MiB
        assert!(!exceeds_generous_quota(max, Some(max)));
        assert!(exceeds_generous_quota(max + 1, Some(max)));
    }
}
