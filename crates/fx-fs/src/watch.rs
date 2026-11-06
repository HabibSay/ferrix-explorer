//! Filesystem watching via the `notify` crate.
//!
//! The public surface is a `LocationWatcher` trait so tests and non-Windows hosts can
//! inject a stub.  The production implementation (`NotifyWatcher`) coalesces all
//! raw notify events into a single `WatchEvent { kind: Any }` after 200 ms of
//! silence to avoid spamming the listing layer.

use fx_domain::location::{Location, LocationKind};

// ── Public types ──────────────────────────────────────────────────────────────

/// An event emitted by the watcher when the watched location changes.
#[derive(Debug, Clone)]
pub struct WatchEvent {
    /// The location that changed.
    pub location: Location,
    /// The kind of change that was detected.
    pub kind: WatchEventKind,
}

/// High-level event classification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchEventKind {
    Created,
    Modified,
    Deleted,
    /// Coalesced: some combination of events occurred; the receiver should rescan.
    Any,
}

/// Errors returned by the watcher.
#[derive(Debug, Clone)]
pub enum WatchError {
    /// The location kind is not supported by this watcher.
    Unsupported(LocationKind),
    /// An I/O-level error from the underlying OS watcher.
    Io(String),
}

impl std::fmt::Display for WatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WatchError::Unsupported(k) => write!(f, "Watching {k:?} locations is not supported"),
            WatchError::Io(s) => write!(f, "Watcher I/O error: {s}"),
        }
    }
}

impl std::error::Error for WatchError {}

// ── Trait ─────────────────────────────────────────────────────────────────────

/// Trait implemented by filesystem watchers.
pub trait LocationWatcher: Send {
    /// Start watching `location`.  Change events are sent on `tx`.
    fn watch(
        &mut self,
        location: &Location,
        tx: tokio::sync::mpsc::UnboundedSender<WatchEvent>,
    ) -> Result<(), WatchError>;

    /// Stop watching and release resources.
    fn stop(&mut self);
}

// ── NotifyWatcher ─────────────────────────────────────────────────────────────

/// Production watcher backed by the `notify` crate.
///
/// Events are coalesced: after the first raw event arrives, the watcher waits
/// 200 ms for the burst to settle, then emits a single `WatchEventKind::Any`.
#[derive(Default)]
pub struct NotifyWatcher {
    /// The inner `notify` watcher; kept alive to maintain the watch.
    #[allow(dead_code)] // held for its Drop side-effect
    inner: Option<Box<dyn notify::Watcher + Send>>,
    /// Handle to the coalescence task so we can abort it on `stop`.
    coalesce_handle: Option<tokio::task::JoinHandle<()>>,
}

impl NotifyWatcher {
    pub fn new() -> Self {
        Self::default()
    }
}

impl LocationWatcher for NotifyWatcher {
    fn watch(
        &mut self,
        location: &Location,
        tx: tokio::sync::mpsc::UnboundedSender<WatchEvent>,
    ) -> Result<(), WatchError> {
        let path = match location {
            Location::Local(p) => p.clone(),
            other => return Err(WatchError::Unsupported(other.kind())),
        };

        let watch_location = location.clone();

        // Raw-event channel from notify → coalescence task.
        let (raw_tx, mut raw_rx) = tokio::sync::mpsc::unbounded_channel::<notify::Event>();

        // Coalescence task: wait for the first event, sleep 200 ms, send one Any.
        let handle = tokio::spawn(async move {
            loop {
                // Block until the first event of a burst.
                if raw_rx.recv().await.is_none() {
                    break; // channel closed
                }
                // Drain any additional events that arrive within 200 ms using
                // a blocking sleep inside spawn_blocking to avoid requiring the
                // `time` tokio feature.
                tokio::task::spawn_blocking(|| {
                    std::thread::sleep(std::time::Duration::from_millis(200));
                })
                .await
                .ok();
                while raw_rx.try_recv().is_ok() {}

                let event = WatchEvent {
                    location: watch_location.clone(),
                    kind: WatchEventKind::Any,
                };
                if tx.send(event).is_err() {
                    break; // receiver gone
                }
            }
        });

        // Build the notify watcher; errors from the handler are sent to raw_tx.
        let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(ev) = res {
                let _ = raw_tx.send(ev);
            }
        })
        .map_err(|e| WatchError::Io(e.to_string()))?;

        let mut watcher: Box<dyn notify::Watcher + Send> = Box::new(watcher);
        watcher
            .watch(&path, notify::RecursiveMode::NonRecursive)
            .map_err(|e| WatchError::Io(e.to_string()))?;

        self.inner = Some(watcher);
        self.coalesce_handle = Some(handle);
        Ok(())
    }

    fn stop(&mut self) {
        // Dropping `inner` unregisters the OS watch and closes raw_tx.
        self.inner = None;
        // Abort the coalescence task.
        if let Some(h) = self.coalesce_handle.take() {
            h.abort();
        }
    }
}

impl Drop for NotifyWatcher {
    fn drop(&mut self) {
        self.stop();
    }
}
