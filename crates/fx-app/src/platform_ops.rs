//! `PlatformOps` implementation for Tranche 2.
//!
//! Delegates Win32 calls to `fx-shell`; no unsafe code here.

use std::{
    io,
    path::{Path, PathBuf},
    ptr,
};

use fx_ops::port::{
    PlatformOps, RecycleError, RecyclePrecheck, RecycledItem, ReparseKind, RestoreError, VolumeKind,
};
use windows::Win32::Foundation::HWND;

/// Converts `fx_shell::volume::VolumeKind` to `fx_ops::port::VolumeKind`.
fn shell_volume_kind_to_ops(kind: fx_shell::volume::VolumeKind) -> VolumeKind {
    match kind {
        fx_shell::volume::VolumeKind::Fixed => VolumeKind::Fixed,
        fx_shell::volume::VolumeKind::Removable => VolumeKind::Removable,
        fx_shell::volume::VolumeKind::Network => VolumeKind::Network,
        fx_shell::volume::VolumeKind::Other => VolumeKind::Other,
    }
}

pub struct WindowsPlatformOps;

/// Bytes held by the top-level files of `paths`.
///
/// The walk does not recurse and directories count as nothing, so the quota pre-check can
/// only under-estimate the payload — the one direction that cannot cause a false refusal.
fn payload_floor_bytes(paths: &[PathBuf]) -> u64 {
    let mut sum = 0u64;
    for path in paths {
        if let Ok(meta) = std::fs::metadata(path)
            && meta.is_file()
        {
            sum = sum.saturating_add(meta.len());
        }
    }
    sum
}

impl PlatformOps for WindowsPlatformOps {
    fn volume_id(&self, path: &Path) -> io::Result<u64> {
        fx_shell::volume::get_volume_serial(path)
    }

    fn volume_kind(&self, path: &Path) -> io::Result<VolumeKind> {
        fx_shell::volume::get_volume_kind(path).map(shell_volume_kind_to_ops)
    }

    fn free_space(&self, path: &Path) -> io::Result<u64> {
        fx_shell::volume::get_free_space(path)
    }

    fn replace_file(&self, replacement: &Path, destination: &Path) -> io::Result<()> {
        fx_shell::volume::replace_file_atomic(replacement, destination)
    }

    fn can_recycle(&self, paths: &[std::path::PathBuf]) -> Result<RecyclePrecheck, RecycleError> {
        for path in paths {
            match self.volume_kind(path) {
                Ok(VolumeKind::Fixed) => {}
                Ok(other) => {
                    return Ok(RecyclePrecheck::Impossible(format!(
                        "not a fixed volume: {:?}",
                        other
                    )));
                }
                Err(err) => {
                    return Err(RecycleError {
                        code: 1,
                        message: err.to_string(),
                    });
                }
            }
        }

        // The quota half of the pre-check. Refuses only on an overflow that survives every
        // reading of the configured maximum, so a mistake in the undocumented unit can never
        // turn a recyclable delete into a refusal; `FOF_WANTNUKEWARNING` remains the net.
        let payload = payload_floor_bytes(paths);
        if payload == 0 {
            return Ok(RecyclePrecheck::Possible);
        }
        let usage = match fx_shell::recycle_quota::query_recycle_bin(&paths[0]) {
            Ok(usage) => usage,
            Err(err) => {
                tracing::debug!(%err, "SHQueryRecycleBinW failed; quota pre-check skipped");
                return Ok(RecyclePrecheck::Possible);
            }
        };
        if fx_shell::recycle_quota::exceeds_generous_quota(payload, usage.max_bytes) {
            let limit = usage.max_bytes.unwrap_or_default();
            return Ok(RecyclePrecheck::Impossible(format!(
                "{} of items does not fit the Recycle Bin limit of {}",
                crate::orchestration::format_bytes(payload),
                crate::orchestration::format_bytes(limit)
            )));
        }
        Ok(RecyclePrecheck::Possible)
    }

    fn recycle(&self, paths: &[std::path::PathBuf]) -> Result<Vec<RecycledItem>, RecycleError> {
        let hwnd = HWND(ptr::null_mut());
        let shell_items =
            fx_shell::recycle::recycle_paths(paths, hwnd).map_err(|err| RecycleError {
                code: match &err {
                    fx_shell::recycle::ShellRecycleError::Failed(c) => *c,
                    _ => 1,
                },
                message: err.to_string(),
            })?;

        let items = shell_items
            .into_iter()
            .map(|item| {
                // An empty recycled_name means the before/after diff was ambiguous.
                let verified = !item.recycled_name.is_empty();
                RecycledItem {
                    original: item.original,
                    recycled_name: if verified {
                        Some(item.recycled_name)
                    } else {
                        None
                    },
                    verified,
                }
            })
            .collect();
        Ok(items)
    }

    fn restore_recycled(&self, item: &RecycledItem) -> Result<(), RestoreError> {
        fx_shell::recycle::restore_recycled_item(&item.original).map_err(|err| RestoreError {
            message: err.to_string(),
        })
    }

    /// Copy metadata from `from` to `to`, including creation time via `SetFileTime`.
    fn apply_metadata(&self, from: &Path, to: &Path) -> io::Result<()> {
        // Apply the standard portable metadata first (modification time, access time,
        // read-only permission bit).
        fx_ops::port::std_apply_metadata(from, to)?;

        // Additionally restore the creation time, which `std` cannot set; the
        // Win32 call lives in `fx-shell` (rule 3).
        fx_shell::file_time::set_creation_time(from, to)
    }

    /// Classifies a reparse point by its tag, so the engine can recreate symbolic links
    /// and keep skipping junctions (spec decision 6).
    fn reparse_kind(&self, path: &Path) -> Option<ReparseKind> {
        let tag = fx_shell::symlink::reparse_tag(path).ok().flatten()?;
        Some(match tag {
            fx_shell::symlink::IO_REPARSE_TAG_SYMLINK => ReparseKind::SymbolicLink,
            fx_shell::symlink::IO_REPARSE_TAG_MOUNT_POINT => ReparseKind::Junction,
            _ => ReparseKind::Other,
        })
    }

    fn create_symlink(&self, target: &Path, link: &Path, is_dir: bool) -> io::Result<()> {
        fx_shell::symlink::create_link(target, link, is_dir)
    }
}
