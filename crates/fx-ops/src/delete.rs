//! Deleting: to the Recycle Bin through the Shell, or permanently with confirmation.
//!
//! Recycling is gated on a fixed local volume because `FOF_ALLOWUNDO` silently turns into a
//! permanent delete elsewhere, and on the adapter's quota pre-check. Nothing here deletes
//! permanently unless the caller asked for it.

use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::{
    control::OperationControl,
    error::{ItemFailure, OperationError},
    port::{PlatformOps, RecyclePrecheck, RecycledItem, VolumeKind},
    progress::{OperationId, OperationKind, OperationSummary},
};

/// Sends `paths` to the Recycle Bin and verifies they are gone.
pub fn delete_to_recycle(
    paths: &[PathBuf],
    port: &dyn PlatformOps,
) -> Result<Vec<RecycledItem>, OperationError> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }

    for path in paths {
        let parent = path.parent().unwrap_or(path);
        match port.volume_kind(parent) {
            Ok(VolumeKind::Fixed) => {}
            Ok(kind) => return Err(OperationError::RecycleUnavailable(kind)),
            Err(error) => return Err(OperationError::from_io(&error)),
        }
    }

    match port.can_recycle(paths) {
        Ok(RecyclePrecheck::Possible) => {}
        Ok(RecyclePrecheck::Impossible(reason)) => {
            return Err(OperationError::CannotRecycle(reason));
        }
        Err(error) => return Err(OperationError::CannotRecycle(error.message)),
    }

    let items = port
        .recycle(paths)
        .map_err(|error| OperationError::RecycleFailed(error.code))?;

    for path in paths {
        if fs::symlink_metadata(path).is_ok() {
            return Err(OperationError::VerificationFailed);
        }
    }

    Ok(items)
}

/// Removes `paths` for good. Reachable only through an explicit, confirmed command.
pub fn delete_permanent(
    id: OperationId,
    paths: &[PathBuf],
    control: &OperationControl,
) -> OperationSummary {
    let mut summary = OperationSummary::new(id, OperationKind::DeletePermanent);
    let mut cancelled = false;

    for path in paths {
        if control.checkpoint().is_err() {
            cancelled = true;
            break;
        }
        match remove_tree(path, control) {
            Ok(()) => summary.files_done += 1,
            Err(OperationError::Cancelled) => {
                cancelled = true;
                break;
            }
            Err(error) => summary.record_failure(ItemFailure::new(path, error)),
        }
    }

    summary.finish(cancelled)
}

/// Recursive delete, bottom-up, without following reparse points.
pub fn remove_tree(path: &Path, control: &OperationControl) -> Result<(), OperationError> {
    control.checkpoint()?;
    let metadata = fs::symlink_metadata(path).map_err(|error| OperationError::from_io(&error))?;

    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        for entry in fs::read_dir(path).map_err(|error| OperationError::from_io(&error))? {
            let entry = entry.map_err(|error| OperationError::from_io(&error))?;
            remove_tree(&entry.path(), control)?;
        }
        fs::remove_dir(path).map_err(|error| OperationError::from_io(&error))
    } else {
        fs::remove_file(path)
            .or_else(|_| fs::remove_dir(path))
            .map_err(|error| OperationError::from_io(&error))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::port::{RecycleError, RestoreError};

    struct FixedVolume {
        kind: VolumeKind,
        precheck: RecyclePrecheck,
    }

    impl PlatformOps for FixedVolume {
        fn volume_id(&self, _path: &Path) -> std::io::Result<u64> {
            Ok(0)
        }
        fn volume_kind(&self, _path: &Path) -> std::io::Result<VolumeKind> {
            Ok(self.kind)
        }
        fn free_space(&self, _path: &Path) -> std::io::Result<u64> {
            Ok(u64::MAX)
        }
        fn replace_file(&self, _replacement: &Path, _destination: &Path) -> std::io::Result<()> {
            Err(std::io::Error::other("not used"))
        }
        fn can_recycle(&self, _paths: &[PathBuf]) -> Result<RecyclePrecheck, RecycleError> {
            Ok(self.precheck.clone())
        }
        fn recycle(&self, paths: &[PathBuf]) -> Result<Vec<RecycledItem>, RecycleError> {
            Ok(paths
                .iter()
                .map(|path| RecycledItem {
                    original: path.clone(),
                    recycled_name: Some("$R1".to_owned()),
                    verified: true,
                })
                .collect())
        }
        fn restore_recycled(&self, _item: &RecycledItem) -> Result<(), RestoreError> {
            Err(RestoreError {
                message: "not used".to_owned(),
            })
        }
    }

    #[test]
    fn refuses_recycling_on_a_network_volume() {
        let platform = FixedVolume {
            kind: VolumeKind::Network,
            precheck: RecyclePrecheck::Possible,
        };
        let error =
            delete_to_recycle(&[PathBuf::from(r"\\server\share\file.txt")], &platform).unwrap_err();
        assert_eq!(
            error,
            OperationError::RecycleUnavailable(VolumeKind::Network)
        );
    }

    #[test]
    fn refuses_when_the_precheck_says_impossible() {
        let platform = FixedVolume {
            kind: VolumeKind::Fixed,
            precheck: RecyclePrecheck::Impossible("bin full".to_owned()),
        };
        let error =
            delete_to_recycle(&[PathBuf::from(r"C:\Temp\file.txt")], &platform).unwrap_err();
        assert_eq!(error, OperationError::CannotRecycle("bin full".to_owned()));
    }

    #[test]
    fn verifies_that_recycled_items_are_gone() {
        let platform = FixedVolume {
            kind: VolumeKind::Fixed,
            precheck: RecyclePrecheck::Possible,
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("still-here.txt");
        fs::write(&path, b"x").unwrap();

        let error = delete_to_recycle(&[path], &platform).unwrap_err();
        assert_eq!(error, OperationError::VerificationFailed);
    }

    #[test]
    fn permanent_delete_removes_a_tree() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a").join("b");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("file.txt"), b"x").unwrap();

        let summary = delete_permanent(
            OperationId::next(),
            &[dir.path().join("a")],
            &OperationControl::new(),
        );
        assert_eq!(summary.files_done, 1);
        assert!(summary.state.is_success());
        assert!(!dir.path().join("a").exists());
    }

    #[test]
    fn cancelled_delete_reports_cancelled() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("keep.txt");
        fs::write(&file, b"x").unwrap();

        let control = OperationControl::new();
        control.cancel();
        let summary = delete_permanent(OperationId::next(), std::slice::from_ref(&file), &control);
        assert_eq!(summary.state, crate::OperationState::Cancelled);
        assert!(file.exists());
    }
}
