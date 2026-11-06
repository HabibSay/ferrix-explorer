//! Operation identity, state, progress snapshots and speed measurement.
//!
//! The engine computes speed and ETA; the UI only renders the snapshot. That keeps the
//! Slint layer free of arithmetic and makes the numbers testable without a window.

use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use crate::error::ItemFailure;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct OperationId(pub u64);

impl OperationId {
    /// Monotonic within the process; stale events are discarded by comparing ids.
    pub fn next() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperationKind {
    Copy,
    Move,
    Rename,
    CreateDirectory,
    CreateFile,
    Recycle,
    DeletePermanent,
    Restore,
    Checksum,
    CleanEmptyFolders,
}

impl OperationKind {
    /// Kinds whose completion changes or removes user data.
    pub fn is_destructive(self) -> bool {
        matches!(
            self,
            Self::Recycle | Self::DeletePermanent | Self::CleanEmptyFolders
        )
    }

    pub fn is_transfer(self) -> bool {
        matches!(self, Self::Copy | Self::Move)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperationState {
    Planning,
    Running,
    Paused,
    Completed,
    CompletedWithWarnings,
    Failed,
    Cancelled,
}

impl OperationState {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::CompletedWithWarnings | Self::Failed | Self::Cancelled
        )
    }

    /// True only when every item succeeded. `Completed` is never shown for a partial result.
    pub fn is_success(self) -> bool {
        matches!(self, Self::Completed)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Progress {
    pub files_total: u64,
    pub files_done: u64,
    pub bytes_total: u64,
    pub bytes_done: u64,
    pub current: Option<PathBuf>,
    pub bytes_per_second: u64,
    /// `None` while the speed is zero or the total is unknown (directories are not pre-scanned).
    pub eta: Option<Duration>,
}

impl Progress {
    pub fn percent(&self) -> Option<u8> {
        if self.bytes_total > 0 {
            let done = self.bytes_done.min(self.bytes_total);
            Some(((done * 100) / self.bytes_total) as u8)
        } else if self.files_total > 0 {
            let done = self.files_done.min(self.files_total);
            Some(((done * 100) / self.files_total) as u8)
        } else {
            None
        }
    }
}

/// Smoothed transfer speed.
///
/// The first sample is instantaneous; later samples are an exponential moving average with
/// `alpha = 0.3`, which is steady enough to read and still reacts within a second.
#[derive(Debug, Default)]
pub struct SpeedMeter {
    started: bool,
    last_elapsed: Duration,
    last_bytes: u64,
    ewma: f64,
}

impl SpeedMeter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn update(&mut self, bytes_done: u64, elapsed: Duration) -> u64 {
        if !self.started {
            self.started = true;
            self.last_elapsed = elapsed;
            self.last_bytes = bytes_done;
            return 0;
        }
        let seconds = elapsed.saturating_sub(self.last_elapsed).as_secs_f64();
        if seconds <= f64::EPSILON {
            return self.ewma as u64;
        }
        let delta = bytes_done.saturating_sub(self.last_bytes) as f64;
        let instant = delta / seconds;
        self.ewma = if self.ewma == 0.0 {
            instant
        } else {
            0.3 * instant + 0.7 * self.ewma
        };
        self.last_elapsed = elapsed;
        self.last_bytes = bytes_done;
        self.ewma as u64
    }

    pub fn bytes_per_second(&self) -> u64 {
        self.ewma as u64
    }
}

/// Final result of one operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperationSummary {
    pub id: OperationId,
    pub kind: OperationKind,
    pub state: OperationState,
    pub files_done: u64,
    pub files_failed: u64,
    pub files_skipped: u64,
    pub bytes_done: u64,
    pub failures: Vec<ItemFailure>,
    /// Operation-level error: insufficient space, a destination that vanished. Per-item
    /// errors stay in `failures`.
    pub error: Option<crate::error::OperationError>,
}

impl OperationSummary {
    pub fn new(id: OperationId, kind: OperationKind) -> Self {
        Self {
            id,
            kind,
            state: OperationState::Running,
            files_done: 0,
            files_failed: 0,
            files_skipped: 0,
            bytes_done: 0,
            failures: Vec::new(),
            error: None,
        }
    }

    /// Records an operation-level failure. The state becomes `Failed`, never a partial success.
    pub fn fail_with(mut self, error: crate::error::OperationError) -> Self {
        self.error = Some(error);
        self.state = OperationState::Failed;
        self
    }

    pub fn record_failure(&mut self, failure: ItemFailure) {
        self.files_failed += 1;
        self.failures.push(failure);
    }

    /// Terminal state derived from what actually happened, never asserted by the caller.
    pub fn finish(mut self, cancelled: bool) -> Self {
        if self.error.is_some() {
            self.state = OperationState::Failed;
            return self;
        }
        self.state = if cancelled {
            OperationState::Cancelled
        } else if self.files_failed > 0 {
            OperationState::CompletedWithWarnings
        } else {
            OperationState::Completed
        };
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_increasing() {
        let first = OperationId::next();
        let second = OperationId::next();
        assert!(second > first);
    }

    #[test]
    fn percent_prefers_bytes_then_files() {
        let mut progress = Progress::default();
        assert_eq!(progress.percent(), None);
        progress.files_total = 4;
        progress.files_done = 1;
        assert_eq!(progress.percent(), Some(25));
        progress.bytes_total = 200;
        progress.bytes_done = 50;
        assert_eq!(progress.percent(), Some(25));
        progress.bytes_done = 400;
        assert_eq!(progress.percent(), Some(100));
    }

    #[test]
    fn speed_meter_smooths_after_the_first_sample() {
        let mut meter = SpeedMeter::new();
        assert_eq!(meter.update(0, Duration::ZERO), 0);
        assert_eq!(meter.update(1_000_000, Duration::from_secs(1)), 1_000_000);
        // Second sample: another 1 MB in one second, EWMA equals the instantaneous rate.
        assert_eq!(meter.update(2_000_000, Duration::from_secs(2)), 1_000_000);
        // A stall pulls the average down but does not zero it instantly.
        let stalled = meter.update(2_000_000, Duration::from_secs(3));
        assert!(stalled < 1_000_000);
        assert!(stalled > 0);
    }

    #[test]
    fn partial_results_are_not_success() {
        let summary = OperationSummary::new(OperationId::next(), OperationKind::Copy);
        assert_eq!(summary.finish(false).state, OperationState::Completed);

        let mut summary = OperationSummary::new(OperationId::next(), OperationKind::Copy);
        summary.record_failure(ItemFailure::new("C:\\x", crate::OperationError::Locked));
        let finished = summary.finish(false);
        assert_eq!(finished.state, OperationState::CompletedWithWarnings);
        assert!(!finished.state.is_success());
    }

    #[test]
    fn cancellation_wins_over_failures() {
        let mut summary = OperationSummary::new(OperationId::next(), OperationKind::Move);
        summary.record_failure(ItemFailure::new("C:\\x", crate::OperationError::Locked));
        assert_eq!(summary.finish(true).state, OperationState::Cancelled);
    }

    #[test]
    fn an_operation_level_error_is_failed_even_without_items() {
        let summary = OperationSummary::new(OperationId::next(), OperationKind::Copy).fail_with(
            crate::OperationError::InsufficientSpace {
                needed: 10,
                available: 0,
            },
        );
        let summary = summary.finish(false);
        assert_eq!(summary.state, OperationState::Failed);
        assert!(!summary.state.is_success());
    }
}
