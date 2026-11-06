//! Cooperative cancellation and pause for a running operation.
//!
//! Pause is checked between 1 MiB chunks, so at most one chunk is in flight when the user
//! pauses. Nothing here blocks the UI thread: the worker parks on a condition variable and
//! the UI only flips a flag.

use std::sync::{Condvar, Mutex};

use tokio_util::sync::CancellationToken;

use crate::error::OperationError;

#[derive(Debug)]
pub struct OperationControl {
    cancel: CancellationToken,
    paused: Mutex<bool>,
    wake: Condvar,
}

impl Default for OperationControl {
    fn default() -> Self {
        Self::new()
    }
}

impl OperationControl {
    pub fn new() -> Self {
        Self {
            cancel: CancellationToken::new(),
            paused: Mutex::new(false),
            wake: Condvar::new(),
        }
    }

    /// A handle for a UI or a parent task to request cancellation.
    pub fn token(&self) -> CancellationToken {
        self.cancel.clone()
    }

    /// Cancels and wakes a paused worker.
    ///
    /// The mutex is held while cancelling: a waiter that has already tested the token but
    /// has not entered `wait` would otherwise miss both the notification and the flag.
    pub fn cancel(&self) {
        let _guard = self.paused.lock().expect("pause lock poisoned");
        self.cancel.cancel();
        self.wake.notify_all();
    }

    pub fn pause(&self) {
        *self.paused.lock().expect("pause lock poisoned") = true;
    }

    pub fn resume(&self) {
        *self.paused.lock().expect("pause lock poisoned") = false;
        self.wake.notify_all();
    }

    pub fn is_paused(&self) -> bool {
        *self.paused.lock().expect("pause lock poisoned")
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    /// Blocks while paused and fails once cancelled. Call between chunks and between files.
    pub fn checkpoint(&self) -> Result<(), OperationError> {
        if self.is_cancelled() {
            return Err(OperationError::Cancelled);
        }
        let mut paused = self.paused.lock().expect("pause lock poisoned");
        while *paused {
            if self.is_cancelled() {
                return Err(OperationError::Cancelled);
            }
            paused = self.wake.wait(paused).expect("pause lock poisoned");
        }
        if self.is_cancelled() {
            Err(OperationError::Cancelled)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use super::*;

    #[test]
    fn checkpoint_passes_when_running() {
        let control = OperationControl::new();
        assert!(control.checkpoint().is_ok());
        assert!(!control.is_paused());
        assert!(!control.is_cancelled());
    }

    #[test]
    fn pause_blocks_until_resume() {
        let control = Arc::new(OperationControl::new());
        control.pause();
        let worker = Arc::clone(&control);
        let handle = std::thread::spawn(move || worker.checkpoint());
        std::thread::sleep(Duration::from_millis(50));
        assert!(control.is_paused());
        control.resume();
        assert!(handle.join().expect("worker thread").is_ok());
    }

    #[test]
    fn cancel_while_paused_returns_cancelled() {
        let control = Arc::new(OperationControl::new());
        control.pause();
        let worker = Arc::clone(&control);
        let handle = std::thread::spawn(move || worker.checkpoint());
        std::thread::sleep(Duration::from_millis(50));
        control.cancel();
        assert_eq!(
            handle.join().expect("worker thread"),
            Err(OperationError::Cancelled)
        );
    }

    #[test]
    fn token_reports_cancellation() {
        let control = OperationControl::new();
        let token = control.token();
        assert!(!token.is_cancelled());
        control.cancel();
        assert!(token.is_cancelled());
        assert_eq!(control.checkpoint(), Err(OperationError::Cancelled));
    }
}
