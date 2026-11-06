//! The seam between `fx-ops` and Windows.
//!
//! `fx-ops` never depends on `fx-ui` or on `fx-shell` (PRD 10.2), so everything the Shell
//! must do — recycling, restoring, atomic replacement, free space, volume identity — is a
//! trait here, implemented by an adapter in `fx-app`. Tests use a fake.

use std::{
    fs::{self, FileTimes},
    io,
    path::{Path, PathBuf},
};

/// What kind of volume a path lives on.
///
/// Recycling only exists on `Fixed`: `SHFileOperationW` with `FOF_ALLOWUNDO` turns into a
/// permanent delete on network shares and on some removable media, so the engine refuses
/// instead of silently destroying data (design rule 14).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VolumeKind {
    Fixed,
    Removable,
    Network,
    Other,
}

/// One item the Shell moved to the Recycle Bin.
///
/// `SHFileOperationW` returns no handle, so `recycled_name` is derived by the adapter from a
/// before/after listing of the bin. When that diff is ambiguous the item is stored with
/// `verified = false` and undo refuses it rather than restoring the wrong file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecycledItem {
    pub original: PathBuf,
    pub recycled_name: Option<String>,
    pub verified: bool,
}

/// Overflow check that saves the risky Shell call when it can be predicted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecyclePrecheck {
    Possible,
    Impossible(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecycleError {
    pub code: u32,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RestoreError {
    pub message: String,
}

pub trait PlatformOps {
    /// Identity of the volume holding `path`. Used for the same-volume move fast path.
    fn volume_id(&self, path: &Path) -> io::Result<u64>;

    fn volume_kind(&self, path: &Path) -> io::Result<VolumeKind>;

    /// Free bytes available to the current user.
    fn free_space(&self, path: &Path) -> io::Result<u64>;

    /// Replace `destination` with `replacement` atomically.
    fn replace_file(&self, replacement: &Path, destination: &Path) -> io::Result<()>;

    /// Cheap pre-check: volume kind plus the Recycle Bin's remaining quota.
    fn can_recycle(&self, paths: &[PathBuf]) -> Result<RecyclePrecheck, RecycleError>;

    /// Send `paths` to the Recycle Bin. Must fail rather than delete permanently.
    fn recycle(&self, paths: &[PathBuf]) -> Result<Vec<RecycledItem>, RecycleError>;

    /// Restore one recycled item to its original path.
    fn restore_recycled(&self, item: &RecycledItem) -> Result<(), RestoreError>;

    /// Copy timestamps and attributes from `from` to `to`.
    ///
    /// The default uses `std` (modification and access times, read-only flag). The Windows
    /// adapter overrides it to also restore the creation time with `SetFileTimeW`.
    fn apply_metadata(&self, from: &Path, to: &Path) -> io::Result<()> {
        std_apply_metadata(from, to)
    }

    /// Classify a reparse point so the engine can recreate symbolic links instead
    /// of skipping them (slice 2 refinement of rule 6). `None` — the `std`-only
    /// default — keeps the conservative skip for junctions and unknown tags.
    fn reparse_kind(&self, _path: &Path) -> Option<ReparseKind> {
        None
    }

    /// Create a symbolic link at `link` pointing at `target`.
    ///
    /// `fx-ops` stays platform-neutral: link creation is a platform capability, so the
    /// default refuses and the Windows adapter implements it.
    fn create_symlink(&self, _target: &Path, _link: &Path, _is_dir: bool) -> io::Result<()> {
        Err(io::Error::other(
            "this platform adapter cannot create symbolic links",
        ))
    }
}

/// What kind of reparse point a source is, as classified by the platform.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReparseKind {
    SymbolicLink,
    Junction,
    Other,
}

/// Portable metadata copy: modification and access times, then the read-only flag.
///
/// The read-only flag is applied last; applying it first would block the timestamp write.
pub fn std_apply_metadata(from: &Path, to: &Path) -> io::Result<()> {
    let metadata = fs::metadata(from)?;
    let times = FileTimes::new()
        .set_modified(metadata.modified()?)
        .set_accessed(metadata.accessed()?);
    {
        let target = fs::OpenOptions::new().write(true).open(to)?;
        target.set_times(times)?;
    }
    fs::set_permissions(to, metadata.permissions())
}
