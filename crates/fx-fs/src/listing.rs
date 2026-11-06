//! Directory listing via `jwalk`.
//!
//! `list_directory` walks exactly one level of a local directory, emitting
//! `FileEntry` batches over an mpsc channel.  It honours a `CancellationToken`
//! between batches and tags every batch with a `generation` so stale responses
//! can be discarded by the caller.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use fx_domain::entry::{EntryId, EntryKind, FileAttributes, FileEntry};
use fx_domain::location::{Location, LocationKind};
use tokio_util::sync::CancellationToken;

use crate::longpath::strip_verbatim_prefix;

// ── Public surface ────────────────────────────────────────────────────────────

/// A batch of entries produced by a single listing call.
#[derive(Debug)]
pub struct ListingBatch {
    /// Generation counter supplied by the caller; unchanged from the input.
    pub generation: u64,
    /// Up to 512 entries in this batch.
    pub entries: Vec<FileEntry>,
    /// Non-fatal error that occurred while building this batch, if any.
    pub error: Option<ListingError>,
}

/// Errors that can occur during a directory listing.
#[derive(Debug, Clone)]
pub enum ListingError {
    AccessDenied(PathBuf),
    NotFound(PathBuf),
    NotADirectory(PathBuf),
    UnsupportedLocation(LocationKind),
    Cancelled,
    Io(String),
}

impl std::fmt::Display for ListingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ListingError::AccessDenied(p) => write!(f, "Access denied: {}", p.display()),
            ListingError::NotFound(p) => write!(f, "Not found: {}", p.display()),
            ListingError::NotADirectory(p) => write!(f, "Not a directory: {}", p.display()),
            ListingError::UnsupportedLocation(k) => {
                write!(f, "Unsupported location kind: {k:?}")
            }
            ListingError::Cancelled => write!(f, "Listing cancelled"),
            ListingError::Io(s) => write!(f, "I/O error: {s}"),
        }
    }
}

impl std::error::Error for ListingError {}

/// Maximum entries per batch sent over the channel.
const BATCH_SIZE: usize = 512;

/// Streams `FileEntry` batches from a local directory.
///
/// - Sends at most `BATCH_SIZE` entries per message.
/// - Checks `token` between batches; returns `Err(Cancelled)` when triggered.
/// - Returns `Err(UnsupportedLocation)` immediately for non-local locations.
/// - The `tx` channel capacity is managed by the caller (512 suggested).
pub async fn list_directory(
    location: &Location,
    generation: u64,
    token: CancellationToken,
    tx: tokio::sync::mpsc::Sender<ListingBatch>,
) -> Result<(), ListingError> {
    // Only Local paths are supported in Tranche 1.
    let path = match location {
        Location::Local(p) => p.clone(),
        other => return Err(ListingError::UnsupportedLocation(other.kind())),
    };

    // Validate the path before starting the walk.
    match std::fs::metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(ListingError::NotFound(path));
        }
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            return Err(ListingError::AccessDenied(path));
        }
        Err(e) => return Err(ListingError::Io(e.to_string())),
        Ok(meta) if !meta.is_dir() => return Err(ListingError::NotADirectory(path)),
        Ok(_) => {}
    }

    // Atomic counter for cheap EntryId assignment within this listing.
    let counter = AtomicU64::new(1);

    // jwalk walks the directory tree; max_depth(1) gives only the direct children.
    let walker = jwalk::WalkDir::new(&path)
        .max_depth(1)
        .skip_hidden(false)
        .sort(false);

    let mut batch: Vec<FileEntry> = Vec::with_capacity(BATCH_SIZE);

    for result in walker {
        // Flush a full batch, checking for cancellation between sends.
        if batch.len() == BATCH_SIZE {
            if token.is_cancelled() {
                return Err(ListingError::Cancelled);
            }
            let msg = ListingBatch {
                generation,
                entries: std::mem::take(&mut batch),
                error: None,
            };
            // If the receiver is gone we simply stop.
            if tx.send(msg).await.is_err() {
                return Ok(());
            }
            batch = Vec::with_capacity(BATCH_SIZE);
        }

        let entry = match result {
            Ok(e) => e,
            Err(err) => {
                // Emit the error inside a batch so the caller can surface it.
                let listing_err = io_error_to_listing_error(&err.into(), &path);
                let msg = ListingBatch {
                    generation,
                    entries: Vec::new(),
                    error: Some(listing_err),
                };
                let _ = tx.send(msg).await;
                continue;
            }
        };

        // Depth 0 is the root entry itself — skip it.
        if entry.depth == 0 {
            continue;
        }

        let entry_path = entry.path();

        // Build display name: strip \\?\ prefix, then take the file name.
        let path_str = entry_path.to_string_lossy();
        let clean_str = strip_verbatim_prefix(&path_str);
        let clean_path = PathBuf::from(clean_str);
        let display_name = clean_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();

        let file_type = entry.file_type;
        let kind = if file_type.is_symlink() {
            EntryKind::Symlink
        } else if file_type.is_dir() {
            EntryKind::Directory
        } else {
            EntryKind::File
        };

        // Retrieve metadata for size and modified time; failures are non-fatal.
        let (size, modified, attributes) = match entry.metadata() {
            Ok(meta) => {
                let size = if file_type.is_file() {
                    Some(meta.len())
                } else {
                    None
                };
                let modified = meta.modified().ok();
                let attrs = platform_attributes(&meta);
                (size, modified, attrs)
            }
            Err(_) => (None, None, FileAttributes::default()),
        };

        let id = EntryId(counter.fetch_add(1, Ordering::Relaxed));

        batch.push(FileEntry {
            id,
            path: clean_path,
            display_name,
            kind,
            size,
            modified,
            attributes,
            icon_key: 0,
        });
    }

    // Send the final (possibly partial) batch.
    if !batch.is_empty() {
        if token.is_cancelled() {
            return Err(ListingError::Cancelled);
        }
        let msg = ListingBatch {
            generation,
            entries: batch,
            error: None,
        };
        let _ = tx.send(msg).await;
    }

    Ok(())
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn io_error_to_listing_error(err: &std::io::Error, path: &std::path::Path) -> ListingError {
    match err.kind() {
        std::io::ErrorKind::PermissionDenied => ListingError::AccessDenied(path.to_path_buf()),
        std::io::ErrorKind::NotFound => ListingError::NotFound(path.to_path_buf()),
        _ => ListingError::Io(err.to_string()),
    }
}

/// Extract `FileAttributes` from OS metadata.
///
/// On Windows the `file_attributes` method is available via `std::os::windows::fs::MetadataExt`.
/// On other platforms we return a zeroed struct (tests still compile and run on Linux/macOS CI).
#[cfg(target_os = "windows")]
fn platform_attributes(meta: &std::fs::Metadata) -> FileAttributes {
    use std::os::windows::fs::MetadataExt;
    // FILE_ATTRIBUTE_* constants from WinNT.h
    const FILE_ATTRIBUTE_READONLY: u32 = 0x0000_0001;
    const FILE_ATTRIBUTE_HIDDEN: u32 = 0x0000_0002;
    const FILE_ATTRIBUTE_SYSTEM: u32 = 0x0000_0004;
    const FILE_ATTRIBUTE_ARCHIVE: u32 = 0x0000_0020;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;

    let raw = meta.file_attributes();
    FileAttributes {
        hidden: raw & FILE_ATTRIBUTE_HIDDEN != 0,
        system: raw & FILE_ATTRIBUTE_SYSTEM != 0,
        readonly: raw & FILE_ATTRIBUTE_READONLY != 0,
        archive: raw & FILE_ATTRIBUTE_ARCHIVE != 0,
        reparse_point: raw & FILE_ATTRIBUTE_REPARSE_POINT != 0,
    }
}

#[cfg(not(target_os = "windows"))]
fn platform_attributes(_meta: &std::fs::Metadata) -> FileAttributes {
    FileAttributes::default()
}
