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
///
/// A copy already carries the source's read-only bit, and a read-only file cannot be opened
/// with write access — so on Windows the timestamp write asks for `FILE_WRITE_ATTRIBUTES`
/// instead, which is what `SetFileTime` needs and what a read-only file still grants.
pub fn std_apply_metadata(from: &Path, to: &Path) -> io::Result<()> {
    let metadata = fs::metadata(from)?;
    let times = FileTimes::new()
        .set_modified(metadata.modified()?)
        .set_accessed(metadata.accessed()?);
    open_for_times(to)?.set_times(times)?;
    fs::set_permissions(to, metadata.permissions())
}

/// Opens `path` with just enough access to write its timestamps.
#[cfg(windows)]
fn open_for_times(path: &Path) -> io::Result<fs::File> {
    /// `FILE_WRITE_ATTRIBUTES` from `winnt.h`; `std` requests it through the Win32
    /// `dwDesiredAccess` of `OpenOptions`. No unsafe call is involved.
    const FILE_WRITE_ATTRIBUTES: u32 = 0x100;

    use std::os::windows::fs::OpenOptionsExt;

    fs::OpenOptions::new()
        .access_mode(FILE_WRITE_ATTRIBUTES)
        .open(path)
}

/// Off Windows the read-only flag is a permission bit, so write access is the only way.
#[cfg(not(windows))]
fn open_for_times(path: &Path) -> io::Result<fs::File> {
    fs::OpenOptions::new().write(true).open(path)
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use super::*;

    /// A read-only copy can still be stamped: the destination carries the source's flag
    /// before the timestamp write, which needs a writable handle.
    #[test]
    fn metadata_is_copied_onto_a_read_only_destination() {
        let dir = tempfile::tempdir().expect("temp dir");
        let source = dir.path().join("source.txt");
        fs::write(&source, b"payload").expect("write the source");
        let stamp = UNIX_EPOCH + Duration::from_secs(1_622_937_600);
        {
            let file = fs::OpenOptions::new()
                .write(true)
                .open(&source)
                .expect("open the source");
            file.set_modified(stamp).expect("stamp the source");
        }
        let mut permissions = fs::metadata(&source).expect("metadata").permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&source, permissions).expect("mark the source read-only");

        let destination = dir.path().join("destination.txt");
        fs::copy(&source, &destination).expect("copy");

        std_apply_metadata(&source, &destination).expect("stamp a read-only copy");
        assert_eq!(
            fs::metadata(&destination)
                .expect("metadata")
                .modified()
                .expect("modification time"),
            stamp
        );
        assert!(
            fs::metadata(&destination)
                .expect("metadata")
                .permissions()
                .readonly(),
            "the read-only flag is restored after the write"
        );
        // Leave the scratch directory removable: borrow the permissions of a plain file.
        let plain = dir.path().join("plain.txt");
        fs::write(&plain, b"").expect("write a plain file");
        let ordinary = fs::metadata(&plain).expect("metadata").permissions();
        let _ = fs::set_permissions(&destination, ordinary);
    }
}
