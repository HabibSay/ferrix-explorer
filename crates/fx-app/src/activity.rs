//! Pure activity-center store: one row per operation, capped history, and the
//! retry request derived from what actually failed.

use std::{path::PathBuf, time::Duration};

use fx_ops::progress::{OperationId, Progress};

/// PRD 13.4: the pane is a view, not a log — capped at 50 rows per session.
pub const HISTORY_CAP: usize = 50;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowState {
    Pending,
    Running,
    Paused,
    Completed,
    CompletedWithWarnings,
    Failed,
    Cancelled,
}

impl RowState {
    /// The state word shown in the activity row. The UI never branches on this text:
    /// `activity.slint` reads the `is-paused` flag instead, so a translation cannot
    /// change what the Pause button does.
    pub fn label(self) -> &'static str {
        match self {
            Self::Pending => crate::strings::STATE_PENDING,
            Self::Running => crate::strings::STATE_RUNNING,
            Self::Paused => crate::strings::STATE_PAUSED,
            Self::Completed => crate::strings::STATE_COMPLETED,
            Self::CompletedWithWarnings => crate::strings::STATE_COMPLETED_WITH_WARNINGS,
            Self::Failed => crate::strings::STATE_FAILED,
            Self::Cancelled => crate::strings::STATE_CANCELLED,
        }
    }

    fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::CompletedWithWarnings | Self::Failed | Self::Cancelled
        )
    }
}

/// What re-running this operation would mean; `sources` come from the failures.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RetrySpec {
    Paste {
        destination: PathBuf,
    },
    Delete {
        permanent: bool,
    },
    Rename {
        path: PathBuf,
        new_name: String,
    },
    Create {
        parent: PathBuf,
        name: String,
        dir: bool,
    },
    Checksum {
        paths: Vec<PathBuf>,
    },
    Clean {
        root: PathBuf,
    },
    Undo,
    Redo,
}

pub struct RetryRequest {
    pub spec: RetrySpec,
    pub sources: Vec<PathBuf>,
}

#[derive(Clone, Debug)]
pub struct RowMeta {
    pub label: String,
    pub retry: RetrySpec,
}

pub struct ActivityRow {
    pub id: OperationId,
    pub label: String,
    pub state: RowState,
    pub files_done: u64,
    pub files_total: u64,
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub current: String,
    pub speed_text: String,
    pub eta_text: String,
    pub error_text: String,
    pub failed_paths: Vec<PathBuf>,
    pub retry: RetrySpec,
}

pub struct ActivityStore {
    pub rows: Vec<ActivityRow>,
}

impl ActivityStore {
    pub fn new() -> Self {
        Self { rows: Vec::new() }
    }

    pub fn push(&mut self, id: OperationId, meta: RowMeta) {
        self.rows.push(ActivityRow {
            id,
            label: meta.label,
            state: RowState::Pending,
            files_done: 0,
            files_total: 0,
            bytes_done: 0,
            bytes_total: 0,
            current: String::new(),
            speed_text: String::new(),
            eta_text: String::new(),
            error_text: String::new(),
            failed_paths: Vec::new(),
            retry: meta.retry,
        });
        let overflow = self.rows.len().saturating_sub(HISTORY_CAP);
        if overflow > 0 {
            self.rows.drain(0..overflow);
        }
    }

    pub fn row_id(&self, index: usize) -> Option<OperationId> {
        self.rows.get(index).map(|row| row.id)
    }

    pub fn note_progress(&mut self, id: OperationId, p: &Progress) {
        let Some(row) = self.find_mut(id) else {
            return;
        };
        if !row.state.is_terminal() {
            row.state = RowState::Running;
        }
        row.files_done = p.files_done;
        row.files_total = p.files_total;
        row.bytes_done = p.bytes_done;
        row.bytes_total = p.bytes_total;
        row.current = p
            .current
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default();
        row.speed_text = speed_text(p.bytes_per_second);
        row.eta_text = p.eta.map(eta_text).unwrap_or_default();
    }

    pub fn note_paused(&mut self, id: OperationId, paused: bool) {
        if let Some(row) = self.find_mut(id) {
            match (row.state, paused) {
                (RowState::Running, true) => row.state = RowState::Paused,
                (RowState::Paused, false) => row.state = RowState::Running,
                _ => {}
            }
        }
    }

    pub fn note_terminal(
        &mut self,
        id: OperationId,
        state: RowState,
        errors: Vec<String>,
        failed_paths: Vec<PathBuf>,
    ) {
        if let Some(row) = self.find_mut(id) {
            row.state = state;
            row.error_text = errors.join("; ");
            row.failed_paths = failed_paths;
        }
    }

    pub fn retry_request(&self, index: usize) -> Option<RetryRequest> {
        let row = self.rows.get(index)?;
        let sources = match &row.retry {
            RetrySpec::Checksum { paths } => paths.clone(),
            _ => row.failed_paths.clone(),
        };
        Some(RetryRequest {
            spec: row.retry.clone(),
            sources,
        })
    }

    fn find_mut(&mut self, id: OperationId) -> Option<&mut ActivityRow> {
        self.rows.iter_mut().find(|row| row.id == id)
    }
}

/// Same 1024 steps and decimal style as the status bar's byte formatting.
fn speed_text(bytes_per_second: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;
    if bytes_per_second >= GB {
        format!("{:.1} GB/s", bytes_per_second as f64 / GB as f64)
    } else if bytes_per_second >= MB {
        format!("{:.1} MB/s", bytes_per_second as f64 / MB as f64)
    } else if bytes_per_second >= KB {
        format!("{:.1} KB/s", bytes_per_second as f64 / KB as f64)
    } else {
        format!("{bytes_per_second} B/s")
    }
}

fn eta_text(eta: Duration) -> String {
    let seconds = eta.as_secs();
    if seconds >= 3600 {
        format!("{} h {} min", seconds / 3600, (seconds % 3600) / 60)
    } else if seconds >= 60 {
        format!("{} min {} s", seconds / 60, seconds % 60)
    } else {
        format!("{seconds} s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fx_ops::progress::OperationId;

    fn meta(label: &str) -> RowMeta {
        RowMeta {
            label: label.to_string(),
            retry: RetrySpec::Undo,
        }
    }

    #[test]
    fn pushed_rows_start_pending_and_are_indexable() {
        let mut store = ActivityStore::new();
        let id = OperationId::next();
        store.push(id, meta("Paste"));
        assert_eq!(store.row_id(0), Some(id));
        assert_eq!(store.rows[0].state, RowState::Pending);
        assert_eq!(store.row_id(1), None);
    }

    #[test]
    fn progress_moves_row_to_running_and_formats_speed() {
        let mut store = ActivityStore::new();
        let id = OperationId::next();
        store.push(id, meta("Copy"));
        let progress = fx_ops::Progress {
            files_total: 7,
            files_done: 3,
            bytes_total: 3_000_000,
            bytes_done: 1_536,
            bytes_per_second: 1_536_000,
            eta: Some(std::time::Duration::from_secs(37)),
            ..Default::default()
        };
        store.note_progress(id, &progress);
        let row = &store.rows[0];
        assert_eq!(row.state, RowState::Running);
        assert_eq!(row.files_done, 3);
        assert_eq!(row.files_total, 7);
        assert_eq!(row.speed_text, "1.5 MB/s");
        assert_eq!(row.eta_text, "37 s");
    }

    #[test]
    fn terminal_updates_state_errors_and_failed_paths() {
        let mut store = ActivityStore::new();
        let id = OperationId::next();
        store.push(id, meta("Paste"));
        store.note_terminal(
            id,
            RowState::CompletedWithWarnings,
            vec!["Access is denied".to_string()],
            vec![std::path::PathBuf::from(r"C:\tmp\locked.txt")],
        );
        let row = &store.rows[0];
        assert_eq!(row.state, RowState::CompletedWithWarnings);
        assert_eq!(row.error_text, "Access is denied");
        let request = store.retry_request(0).expect("retry available");
        assert_eq!(
            request.sources,
            vec![std::path::PathBuf::from(r"C:\tmp\locked.txt")]
        );
    }

    #[test]
    fn retry_follows_the_rows_spec() {
        let mut store = ActivityStore::new();
        let id = OperationId::next();
        store.push(
            id,
            RowMeta {
                label: "Paste".to_string(),
                retry: RetrySpec::Paste {
                    destination: std::path::PathBuf::from(r"C:\dst"),
                },
            },
        );
        store.note_terminal(
            id,
            RowState::Failed,
            vec![],
            vec![std::path::PathBuf::from(r"C:\src\a.txt")],
        );
        let request = store.retry_request(0).expect("retry");
        match &request.spec {
            RetrySpec::Paste { destination } => {
                assert_eq!(destination, &std::path::PathBuf::from(r"C:\dst"));
            }
            other => panic!("wrong spec: {other:?}"),
        }
    }

    #[test]
    fn history_is_capped_at_fifty_rows_keeping_the_newest() {
        let mut store = ActivityStore::new();
        let first = OperationId::next();
        store.push(first, meta("first"));
        for _ in 0..50 {
            store.push(OperationId::next(), meta("newer"));
        }
        assert_eq!(store.rows.len(), 50);
        assert_eq!(store.row_id(0), Some(OperationId(first.0 + 1)));
    }

    #[test]
    fn updates_for_unknown_ids_are_dropped_not_panic() {
        let mut store = ActivityStore::new();
        store.note_progress(OperationId::next(), &fx_ops::Progress::default());
        store.note_terminal(OperationId::next(), RowState::Failed, vec![], vec![]);
        assert!(store.rows.is_empty());
    }

    #[test]
    fn state_labels_match_the_slint_vocabulary() {
        assert_eq!(RowState::Pending.label(), "pending");
        assert_eq!(RowState::Paused.label(), "paused");
        assert_eq!(
            RowState::CompletedWithWarnings.label(),
            "completed with warnings"
        );
    }
}
