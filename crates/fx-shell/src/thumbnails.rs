//! Thumbnails: the Shell's pipeline plus the purgeable disk cache — Tranche 3 slice 4.
//!
//! Decision 6: Ferrix *consumes* thumbnails, it does not provide them. Every registered
//! file type already has one, and `IShellItemImageFactory::GetImage` returns it at a
//! requested size — the slice-0 spike measured 2-3 ms warm and preferred it over
//! `IThumbnailCache` (see `docs/tranche-3-validation.md`).
//!
//! Spec §5: a key carries the path, the requested size and the file's modification time,
//! so an edited file misses the cache by construction. Entries are written atomically
//! (temporary file then rename) and evicted oldest-first by file time against a size cap,
//! which is what "purgeable, reconstruction possible" (`PRD.md:457`) asks for: nothing
//! here is the only copy of anything.
//!
//! The cache stores raw RGBA behind a small header rather than an encoded format, so no
//! image codec becomes a dependency of a business crate (rule 6).

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use windows::{
    Win32::{
        Foundation::SIZE,
        Graphics::Gdi::{
            BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, DeleteObject, GetDC, GetDIBits, HBITMAP,
            HGDIOBJ, ReleaseDC,
        },
        UI::Shell::{IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_RESIZETOFIT},
    },
    core::PCWSTR,
};

use crate::drop_source::Apartment;

/// Header of a cache entry: `FXT1`, then width and height as little-endian `u32`.
const CACHE_MAGIC: &[u8; 4] = b"FXT1";
/// Bytes of header before the pixels.
const CACHE_HEADER_LEN: usize = 12;
/// Largest image the cache will accept from the Shell, in either dimension.
///
/// A Shell extension can return anything; a refusal here keeps one bad thumbnail from
/// allocating a gigabyte in a listing (rule 4's spirit: bounded work per row).
const MAX_DIMENSION: u32 = 4096;

/// One decoded image: 8-bit RGBA, rows top-down, no padding between rows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RgbaImage {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// `width * height * 4` bytes of RGBA.
    pub pixels: Vec<u8>,
}

impl RgbaImage {
    /// An image built from `pixels`, or `None` when the buffer does not match the size or
    /// the size is past what the cache accepts.
    pub fn from_pixels(width: u32, height: u32, pixels: Vec<u8>) -> Option<Self> {
        if width == 0 || height == 0 || width > MAX_DIMENSION || height > MAX_DIMENSION {
            return None;
        }
        let expected = width as usize * height as usize * 4;
        if pixels.len() != expected {
            return None;
        }
        Some(Self {
            width,
            height,
            pixels,
        })
    }

    /// Bytes of pixel data.
    pub fn byte_len(&self) -> usize {
        self.pixels.len()
    }

    /// The image serialised as a cache entry.
    fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(CACHE_HEADER_LEN + self.pixels.len());
        bytes.extend_from_slice(CACHE_MAGIC);
        bytes.extend_from_slice(&self.width.to_le_bytes());
        bytes.extend_from_slice(&self.height.to_le_bytes());
        bytes.extend_from_slice(&self.pixels);
        bytes
    }

    /// The image an entry holds, or `None` for a truncated or foreign file.
    fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < CACHE_HEADER_LEN || &bytes[..4] != CACHE_MAGIC {
            return None;
        }
        let width = u32::from_le_bytes(bytes[4..8].try_into().ok()?);
        let height = u32::from_le_bytes(bytes[8..12].try_into().ok()?);
        Self::from_pixels(width, height, bytes[CACHE_HEADER_LEN..].to_vec())
    }
}

/// What a thumbnail request identifies (spec §5).
///
/// The modification time is part of the key: an edited file produces a different key, so
/// its stale thumbnail is unreachable instead of being served.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThumbnailKey {
    /// The file the thumbnail belongs to.
    pub path: PathBuf,
    /// Requested edge length in pixels.
    pub size: u32,
    /// The file's modification time when the request was made.
    pub mtime: SystemTime,
}

impl ThumbnailKey {
    /// The key for `path` at `size`, reading its current modification time.
    pub fn for_path(path: &Path, size: u32) -> std::io::Result<Self> {
        let mtime = std::fs::metadata(path)?.modified()?;
        Ok(Self {
            path: path.to_path_buf(),
            size,
            mtime,
        })
    }

    /// The cache file name for this key: a stable hash, so the name is filesystem-safe
    /// whatever the path contains, and two different keys never share a file in practice.
    pub fn cache_file_name(&self) -> String {
        // FNV-1a over the path, the size and the modification time. The hash decides a
        // cache file name only; it is not a security boundary.
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        let mut feed = |bytes: &[u8]| {
            for byte in bytes {
                hash ^= *byte as u64;
                hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
        };
        feed(self.path.as_os_str().to_string_lossy().as_bytes());
        feed(&self.size.to_le_bytes());
        let nanos = self
            .mtime
            .duration_since(UNIX_EPOCH)
            .map(|delta| delta.as_nanos() as u64)
            .unwrap_or(0);
        feed(&nanos.to_le_bytes());
        format!("{hash:016x}.fxt")
    }

    /// The key as a log-friendly one-liner, for the cache-hit line the gate asks for.
    pub fn describe(&self) -> String {
        format!("{}@{}", self.path.display(), self.size)
    }
}

/// The purgeable on-disk thumbnail cache.
#[derive(Clone, Debug)]
pub struct DiskCache {
    root: PathBuf,
    cap_bytes: u64,
}

impl DiskCache {
    /// Default cap: 256 MiB of thumbnails, evicted oldest-first past that.
    pub const DEFAULT_CAP_BYTES: u64 = 256 * 1024 * 1024;

    /// A cache rooted at `root` holding at most `cap_bytes`.
    pub fn at(root: PathBuf, cap_bytes: u64) -> Self {
        Self { root, cap_bytes }
    }

    /// The cache under `%LOCALAPPDATA%\FerrixExplorer\cache\thumbnails` (`PRD.md:457`),
    /// or `None` when the platform gives no local application-data directory.
    pub fn default_location() -> Option<Self> {
        let base = directories_local_app_data()?;
        Some(Self::at(
            base.join("FerrixExplorer").join("cache").join("thumbnails"),
            Self::DEFAULT_CAP_BYTES,
        ))
    }

    /// The directory the entries live in.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The size cap in bytes.
    pub fn cap_bytes(&self) -> u64 {
        self.cap_bytes
    }

    /// The cached image for `key`, or `None` on a miss or an unreadable entry.
    ///
    /// An entry that fails to decode is deleted: it can only be a truncated write or a
    /// leftover from a different format, and keeping it would miss forever.
    pub fn get(&self, key: &ThumbnailKey) -> Option<RgbaImage> {
        let path = self.entry_path(key);
        let bytes = std::fs::read(&path).ok()?;
        match RgbaImage::decode(&bytes) {
            Some(image) => Some(image),
            None => {
                let _ = std::fs::remove_file(&path);
                None
            }
        }
    }

    /// Writes `image` for `key`.
    ///
    /// The entry is written to a temporary file in the same directory and renamed into
    /// place, so a reader never sees a half-written entry and a crash leaves only a
    /// temporary file behind.
    pub fn put(&self, key: &ThumbnailKey, image: &RgbaImage) -> std::io::Result<()> {
        if image.byte_len() as u64 > self.cap_bytes {
            // Larger than the whole cache: storing it would immediately evict everything
            // else and then itself. Skip the write; the thumbnail is rebuilt on demand.
            return Ok(());
        }
        std::fs::create_dir_all(&self.root)?;
        let target = self.entry_path(key);
        let temporary = self.root.join(format!(
            "{}.{}.tmp",
            key.cache_file_name(),
            std::process::id()
        ));
        std::fs::write(&temporary, image.encode())?;
        match std::fs::rename(&temporary, &target) {
            Ok(()) => Ok(()),
            Err(error) => {
                let _ = std::fs::remove_file(&temporary);
                Err(error)
            }
        }
    }

    /// Deletes entries oldest-first until the cache fits under its cap.
    ///
    /// Returns the number of bytes reclaimed. A missing directory is not an error: a cache
    /// that was never written has nothing to evict.
    pub fn evict(&self) -> std::io::Result<u64> {
        let entries = self.entries()?;
        let mut total: u64 = entries.iter().map(|entry| entry.len).sum();
        if total <= self.cap_bytes {
            return Ok(0);
        }
        let mut reclaimed = 0_u64;
        for entry in entries {
            if total <= self.cap_bytes {
                break;
            }
            match std::fs::remove_file(&entry.path) {
                Ok(()) => {
                    total = total.saturating_sub(entry.len);
                    reclaimed += entry.len;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    total = total.saturating_sub(entry.len);
                }
                Err(_) => continue,
            }
        }
        Ok(reclaimed)
    }

    /// Every cache file, oldest first (ties broken by name so the order is stable).
    fn entries(&self) -> std::io::Result<Vec<CacheEntry>> {
        let directory = match std::fs::read_dir(&self.root) {
            Ok(directory) => directory,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };
        let mut entries = Vec::new();
        for entry in directory {
            let Ok(entry) = entry else { continue };
            let path = entry.path();
            if path.extension().and_then(OsStr::to_str) != Some("fxt") {
                continue;
            }
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if !metadata.is_file() {
                continue;
            }
            entries.push(CacheEntry {
                modified: metadata.modified().unwrap_or(UNIX_EPOCH),
                name: entry.file_name(),
                len: metadata.len(),
                path,
            });
        }
        entries.sort_by(|left, right| {
            left.modified
                .cmp(&right.modified)
                .then_with(|| left.name.cmp(&right.name))
        });
        Ok(entries)
    }

    /// Where the entry for `key` lives.
    fn entry_path(&self, key: &ThumbnailKey) -> PathBuf {
        self.root.join(key.cache_file_name())
    }
}

/// One file in the cache directory.
#[derive(Debug)]
struct CacheEntry {
    modified: SystemTime,
    name: std::ffi::OsString,
    len: u64,
    path: PathBuf,
}

/// `%LOCALAPPDATA%`, the same root the logs use.
///
/// Read from the environment rather than through a base-directory crate: this crate must
/// not gain a dependency just to concatenate a path, and the host already resolved the
/// variable when it started.
fn directories_local_app_data() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
}

/// A thumbnail and where it came from.
///
/// The caller logs the hit, not this crate: `fx-shell` holds no logger, and the gate asks
/// for the hit to be provable from the application's log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FetchedThumbnail {
    /// The image itself.
    pub image: RgbaImage,
    /// True when the disk cache served it without asking the Shell.
    pub from_cache: bool,
}

/// The thumbnail for `key`: the cache first, then the Shell, then `None`.
///
/// `None` is not an error — the row keeps the icon `icons.rs` already drew (spec §7). The
/// caller runs this off the UI thread: `GetImage` is a Shell call that can block on a
/// third-party thumbnail handler.
pub fn fetch_thumbnail(key: &ThumbnailKey) -> Option<RgbaImage> {
    let cache = DiskCache::default_location()?;
    fetch_thumbnail_into(&cache, key).map(|fetched| fetched.image)
}

/// [`fetch_thumbnail`] against an explicit cache, so tests and the app can place it, and
/// reporting whether the cache answered.
pub fn fetch_thumbnail_into(cache: &DiskCache, key: &ThumbnailKey) -> Option<FetchedThumbnail> {
    if let Some(image) = cache.get(key) {
        return Some(FetchedThumbnail {
            image,
            from_cache: true,
        });
    }
    let image = fetch_from_shell(&key.path, key.size)?;
    if let Err(_error) = cache.put(key, &image) {
        // A cache that cannot be written is not a thumbnail failure: the image is still
        // returned, it is just fetched again next time.
    }
    let _ = cache.evict();
    Some(FetchedThumbnail {
        image,
        from_cache: false,
    })
}

/// Asks the Shell for `path`'s thumbnail at `size` pixels a side.
///
/// Runs on a thread with an apartment of its own: the pool threads the app uses are not
/// the UI thread, so this initialises COM for the duration of the call.
pub fn fetch_from_shell(path: &Path, size: u32) -> Option<RgbaImage> {
    if size == 0 {
        return None;
    }
    let _apartment = Apartment::new().ok()?;
    let wide = wide_null(path);
    // SAFETY: the apartment guard is alive for the whole block, `wide` outlives every call
    // below, the item and factory are live COM objects, and the HBITMAP is deleted on the
    // one path that owns it.
    unsafe {
        let item: windows::Win32::UI::Shell::IShellItem =
            SHCreateItemFromParsingName(PCWSTR(wide.as_ptr()), None).ok()?;
        let factory: IShellItemImageFactory = windows::core::Interface::cast(&item).ok()?;
        let bitmap: HBITMAP = factory
            .GetImage(
                SIZE {
                    cx: size as i32,
                    cy: size as i32,
                },
                SIIGBF_RESIZETOFIT,
            )
            .ok()?;
        let image = image_from_bitmap(bitmap);
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        image
    }
}

/// Reads an `HBITMAP` out as top-down RGBA.
///
/// The Shell returns a 32-bit top-down DIB through `GetImage`, but `GetDIBits` reports the
/// real layout in the header, so the row order is read from `biHeight`'s sign instead of
/// assumed. The pixels come back as BGRA and are swapped to RGBA here.
fn image_from_bitmap(bitmap: HBITMAP) -> Option<RgbaImage> {
    // SAFETY: the screen DC is released on every return path; `bitmap` is caller-owned and
    // live; the second `GetDIBits` writes into a buffer sized from the first call's header.
    unsafe {
        let dc = GetDC(None);
        if dc.is_invalid() {
            return None;
        }
        let mut info: BITMAPINFO = std::mem::zeroed();
        info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        // First call with no buffer: fills width, height and bit count into the header.
        let measured = GetDIBits(dc, bitmap, 0, 0, None, &mut info, DIB_RGB_COLORS);
        let width = info.bmiHeader.biWidth;
        let signed_height = info.bmiHeader.biHeight;
        let height = signed_height.unsigned_abs();
        let depth = info.bmiHeader.biBitCount;
        if measured == 0 || width <= 0 || height == 0 || depth != 32 {
            ReleaseDC(None, dc);
            return None;
        }
        if width as u32 > MAX_DIMENSION || height > MAX_DIMENSION {
            ReleaseDC(None, dc);
            return None;
        }

        let pixel_count = width as usize * height as usize;
        let mut bits = vec![0u8; pixel_count * 4];
        // A negative `biHeight` means the rows are stored top-down; ask for them that way
        // so no manual flip is needed.
        info.bmiHeader.biHeight = -(height as i32);
        let read = GetDIBits(
            dc,
            bitmap,
            0,
            height,
            Some(bits.as_mut_ptr().cast()),
            &mut info,
            DIB_RGB_COLORS,
        );
        ReleaseDC(None, dc);
        if read == 0 {
            return None;
        }

        // BGRA in, RGBA out.
        for pixel in bits.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
        RgbaImage::from_pixels(width as u32, height, bits)
    }
}

/// `path` as a NUL-terminated wide string.
fn wide_null(path: &Path) -> Vec<u16> {
    OsStr::new(path.as_os_str())
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch directory under `target/tmp`, removed when the guard drops.
    struct Workspace(PathBuf);

    impl Workspace {
        fn new(name: &str) -> Self {
            let base = std::env::var("TEMP")
                .map(PathBuf::from)
                .unwrap_or_else(|_| std::env::temp_dir());
            let root = base.join(format!("ferrix-thumbnails-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).expect("create the scratch directory");
            Self(root)
        }
    }

    impl Drop for Workspace {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn key(name: &str, size: u32, seconds: u64) -> ThumbnailKey {
        ThumbnailKey {
            path: PathBuf::from(format!(r"C:\pictures\{name}")),
            size,
            mtime: UNIX_EPOCH + std::time::Duration::from_secs(seconds),
        }
    }

    fn image(width: u32, height: u32, seed: u8) -> RgbaImage {
        let count = (width as usize) * (height as usize) * 4;
        let pixels = (0..count)
            .map(|index| (index as u8).wrapping_add(seed))
            .collect();
        RgbaImage::from_pixels(width, height, pixels).expect("a valid image")
    }

    /// A key must miss when the file is edited: the modification time is part of the name,
    /// which is what makes the cache safe to keep across runs.
    #[test]
    fn the_cache_name_changes_with_the_modification_time() {
        let before = key("a.jpg", 96, 1);
        let after = key("a.jpg", 96, 2);
        assert_ne!(before.cache_file_name(), after.cache_file_name());
    }

    /// The size belongs to the key too: a larger render must not be served a 96 px entry.
    #[test]
    fn the_cache_name_changes_with_the_requested_size() {
        assert_ne!(
            key("a.jpg", 96, 1).cache_file_name(),
            key("a.jpg", 256, 1).cache_file_name()
        );
    }

    /// Two different files never share an entry, whatever their names look like.
    #[test]
    fn the_cache_name_is_stable_and_per_path() {
        assert_eq!(
            key("a.jpg", 96, 1).cache_file_name(),
            key("a.jpg", 96, 1).cache_file_name()
        );
        assert_ne!(
            key("a.jpg", 96, 1).cache_file_name(),
            key("b.jpg", 96, 1).cache_file_name()
        );
        let name = key("a.jpg", 96, 1).cache_file_name();
        assert!(name.ends_with(".fxt"), "unexpected entry name: {name}");
        assert!(
            !name.contains('\\') && !name.contains(':'),
            "the entry name must be filesystem-safe: {name}"
        );
    }

    /// An entry round-trips through the file format unchanged.
    #[test]
    fn an_entry_round_trips_through_the_cache() {
        let workspace = Workspace::new("round-trip");
        let cache = DiskCache::at(workspace.0.clone(), DiskCache::DEFAULT_CAP_BYTES);
        let key = key("a.jpg", 96, 1);
        let original = image(4, 3, 7);

        assert!(cache.get(&key).is_none(), "an empty cache must miss");
        cache.put(&key, &original).expect("write the entry");
        assert_eq!(cache.get(&key), Some(original));
    }

    /// A truncated or foreign file is a miss, and is deleted rather than kept forever.
    #[test]
    fn a_corrupt_entry_is_a_miss_and_is_removed() {
        let workspace = Workspace::new("corrupt");
        let cache = DiskCache::at(workspace.0.clone(), DiskCache::DEFAULT_CAP_BYTES);
        let key = key("a.jpg", 96, 1);
        std::fs::create_dir_all(&workspace.0).expect("cache root");
        let entry = workspace.0.join(key.cache_file_name());
        std::fs::write(&entry, b"not a thumbnail").expect("write the corrupt entry");

        assert!(cache.get(&key).is_none());
        assert!(
            !entry.exists(),
            "a corrupt entry must be deleted, not retried forever"
        );
    }

    /// Eviction is oldest-first by file time until the cache fits under its cap.
    #[test]
    fn eviction_drops_the_oldest_entries_first() {
        let workspace = Workspace::new("evict");
        // Room for roughly two entries of 4x4 RGBA (64 bytes each, 76 with the header).
        let cache = DiskCache::at(workspace.0.clone(), 180);
        let newest = key("new.png", 16, 400);
        let middle = key("mid.png", 16, 300);
        let oldest = key("old.png", 16, 200);
        let payload = image(4, 4, 1);

        for (order, entry) in [&oldest, &middle, &newest].into_iter().enumerate() {
            cache.put(entry, &payload).expect("write the entry");
            // Distinct file times: the filesystem's resolution is coarse, so step them.
            let stamp = SystemTime::now() - std::time::Duration::from_secs(60)
                + std::time::Duration::from_secs(order as u64 * 20);
            set_file_time(&workspace.0.join(entry.cache_file_name()), stamp);
        }

        let reclaimed = cache.evict().expect("evict");
        assert!(
            reclaimed > 0,
            "the cache was over its cap, so something had to go"
        );
        assert!(
            cache.get(&oldest).is_none(),
            "the oldest entry must go first"
        );
        assert!(
            cache.get(&newest).is_some(),
            "the newest entry must survive one eviction"
        );
    }

    /// An entry nobody has written yet is not an error: a cache directory that does not
    /// exist must not turn a first listing into a failure.
    #[test]
    fn evicting_a_missing_cache_is_not_an_error() {
        let workspace = Workspace::new("missing");
        let cache = DiskCache::at(workspace.0.join("never-written"), 1024);
        assert_eq!(cache.evict().expect("evict"), 0);
    }

    /// An image larger than the whole cap is skipped rather than evicting the cache to
    /// store something that cannot fit.
    #[test]
    fn an_oversized_image_is_not_stored() {
        let workspace = Workspace::new("oversized");
        let cache = DiskCache::at(workspace.0.clone(), 64);
        let key = key("huge.png", 1024, 1);
        cache.put(&key, &image(8, 8, 3)).expect("a skipped write");
        assert!(cache.get(&key).is_none());
    }

    /// The decoder refuses a buffer whose length contradicts its header instead of
    /// trusting the header.
    #[test]
    fn a_mismatched_pixel_buffer_is_refused() {
        assert!(RgbaImage::from_pixels(2, 2, vec![0; 15]).is_none());
        assert!(RgbaImage::from_pixels(2, 2, vec![0; 16]).is_some());
        assert!(RgbaImage::from_pixels(0, 2, Vec::new()).is_none());
        assert!(RgbaImage::from_pixels(MAX_DIMENSION + 1, 1, vec![0; 4]).is_none());
    }

    /// Sets a file's modification time so eviction order is deterministic.
    ///
    /// `File::set_modified` is `std`, so this test needs no Win32 call of its own.
    fn set_file_time(path: &Path, stamp: SystemTime) {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open(path)
            .expect("reopen the cache entry");
        file.set_modified(stamp).expect("set the modification time");
    }
}
