//! Thumbnail loading for the list — Tranche 3 slice 4.
//!
//! Rule 4: no Shell call on the UI thread. `IShellItemImageFactory::GetImage` can block on
//! a third-party thumbnail handler, so fetches run on the blocking pool, at most
//! [`MAX_CONCURRENT_FETCHES`] at a time, and results come back through a channel that the
//! Slint thread drains. A result carries the listing generation it was requested under, so
//! the view model can refuse it once the user has navigated away.

use std::sync::Arc;

use fx_shell::thumbnails::{DiskCache, ThumbnailKey, fetch_thumbnail_into};
use fx_ui::viewmodel::ThumbnailRequest;
use tokio::sync::{Semaphore, mpsc::UnboundedSender};

/// The edge length asked of the Shell. The row is ~22 px, so 64 covers a 200% display
/// without asking the Shell to render something the list will only shrink.
pub const THUMBNAIL_SIZE: u32 = 64;

/// How many Shell fetches may be in flight at once.
///
/// Each one occupies a blocking-pool thread and may call into a shell extension; keeping the
/// number small keeps a folder full of images from saturating the pool the file operations
/// also use.
pub const MAX_CONCURRENT_FETCHES: usize = 4;

/// A finished fetch, waiting to be applied on the Slint thread.
#[derive(Clone, Debug)]
pub struct FetchedThumbnail {
    /// The row it belongs to.
    pub entry_id: u64,
    /// The listing generation it was requested under.
    pub generation: u32,
    /// The image, or `None` when the Shell had none (the row keeps its icon).
    pub image: Option<fx_shell::RgbaImage>,
    /// True when the disk cache answered, which the gate's log line reports.
    pub from_cache: bool,
}

/// Schedules thumbnail fetches against one cache.
#[derive(Clone)]
pub struct ThumbnailLoader {
    cache: Arc<DiskCache>,
    limit: Arc<Semaphore>,
}

impl ThumbnailLoader {
    /// A loader over the default cache, or `None` when the platform gives no
    /// `%LOCALAPPDATA%` — in that case the list simply shows no thumbnails.
    pub fn new() -> Option<Self> {
        let cache = DiskCache::default_location()?;
        Some(Self {
            cache: Arc::new(cache),
            limit: Arc::new(Semaphore::new(MAX_CONCURRENT_FETCHES)),
        })
    }

    /// Where the cache lives, for the startup log line.
    pub fn cache_root(&self) -> &std::path::Path {
        self.cache.root()
    }

    /// Schedules one fetch on the blocking pool.
    ///
    /// The semaphore is acquired *inside* the task rather than awaited by the caller, so
    /// asking for a hundred thumbnails costs a hundred cheap spawns and no queued work on
    /// the thread that asks.
    pub fn request(
        &self,
        runtime: &tokio::runtime::Handle,
        request: ThumbnailRequest,
        sender: UnboundedSender<FetchedThumbnail>,
    ) {
        let cache = Arc::clone(&self.cache);
        let limit = Arc::clone(&self.limit);
        runtime.spawn_blocking(move || {
            let permit = limit.blocking_acquire().expect("the loader is still alive");
            let entry_id = request.entry_id;
            let generation = request.generation;
            let path = request.path.clone();
            let fetched = ThumbnailKey::for_path(&path, request.size)
                .ok()
                .and_then(|key| fetch_thumbnail_into(&cache, &key));
            let (image, from_cache) = match fetched {
                Some(thumbnail) => (Some(thumbnail.image), thumbnail.from_cache),
                None => (None, false),
            };
            drop(permit);
            // A closed channel means the window is gone; dropping the result is the
            // correct behaviour there.
            let _ = sender.send(FetchedThumbnail {
                entry_id,
                generation,
                image,
                from_cache,
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn request(id: u64) -> ThumbnailRequest {
        ThumbnailRequest {
            entry_id: id,
            generation: 1,
            path: PathBuf::from(r"C:\pictures\missing.png"),
            size: THUMBNAIL_SIZE,
        }
    }

    /// A request for a path that does not exist still produces exactly one result, with no
    /// image: the list must not be left waiting on a row the Shell cannot answer for.
    #[test]
    fn a_missing_file_produces_one_empty_result() {
        let loader = ThumbnailLoader::new().expect("a cache location on Windows");
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("a runtime");
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        loader.request(runtime.handle(), request(7), sender);
        let result = runtime
            .block_on(async { receiver.recv().await })
            .expect("the fetch answers");
        assert_eq!(result.entry_id, 7);
        assert_eq!(result.generation, 1);
        assert!(result.image.is_none(), "a missing file has no thumbnail");
        assert!(!result.from_cache);
    }

    /// A batch of requests all answer: the concurrency limit delays them, it does not drop
    /// them, and every result names the row it belongs to.
    #[test]
    fn every_scheduled_request_answers_once() {
        let loader = ThumbnailLoader::new().expect("a cache location on Windows");
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("a runtime");
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        for id in 0..(MAX_CONCURRENT_FETCHES as u64 * 2) {
            loader.request(runtime.handle(), request(id), sender.clone());
        }
        drop(sender);
        let mut seen: Vec<u64> = runtime.block_on(async {
            let mut seen = Vec::new();
            while let Some(result) = receiver.recv().await {
                seen.push(result.entry_id);
            }
            seen
        });
        seen.sort_unstable();
        let expected: Vec<u64> = (0..(MAX_CONCURRENT_FETCHES as u64 * 2)).collect();
        assert_eq!(seen, expected);
    }
}
