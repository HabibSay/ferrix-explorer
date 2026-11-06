//! Typed errors for file operations.
//!
//! Every variant is a case the activity center or the status region has to explain to the
//! user, so the set is closed and exhaustive rather than a bag of strings. `io::Error` is
//! flattened at the boundary: `io::Error` is neither `Clone` nor `PartialEq`, and the error
//! list needs both.

use std::{io, path::PathBuf};

use crate::port::VolumeKind;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OperationError {
    AccessDenied,
    NotFound,
    AlreadyExists,
    NotADirectory,
    /// The name failed Explorer's rules; the string is the machine-readable reason
    /// (`reserved-device`, `invalid-character`, `trailing-dot`, …), not user text.
    InvalidName(&'static str),
    PathTooLong,
    ReparsePointUnsupported,
    SourceInsideDestination,
    SameSourceAndDestination,
    InsufficientSpace {
        needed: u64,
        available: u64,
    },
    SourceChanged,
    Locked,
    CannotRecycle(String),
    RecycleUnavailable(VolumeKind),
    RecycleFailed(u32),
    RestoreUnavailable,
    VerificationFailed,
    Cancelled,
    Io {
        kind: io::ErrorKind,
        message: String,
    },
}

impl OperationError {
    pub fn is_cancelled(&self) -> bool {
        matches!(self, Self::Cancelled)
    }

    pub fn from_io(error: &io::Error) -> Self {
        let kind = error.kind();
        match kind {
            io::ErrorKind::PermissionDenied => Self::AccessDenied,
            io::ErrorKind::NotFound => Self::NotFound,
            io::ErrorKind::AlreadyExists => Self::AlreadyExists,
            _ => Self::Io {
                kind,
                message: error.to_string(),
            },
        }
    }
}

impl From<io::Error> for OperationError {
    fn from(error: io::Error) -> Self {
        Self::from_io(&error)
    }
}

impl std::fmt::Display for OperationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AccessDenied => f.write_str("access denied"),
            Self::NotFound => f.write_str("not found"),
            Self::AlreadyExists => f.write_str("already exists"),
            Self::NotADirectory => f.write_str("not a directory"),
            Self::InvalidName(reason) => write!(f, "invalid name ({reason})"),
            Self::PathTooLong => f.write_str("path too long"),
            Self::ReparsePointUnsupported => f.write_str("reparse point is not copied"),
            Self::SourceInsideDestination => f.write_str("source is inside the destination"),
            Self::SameSourceAndDestination => f.write_str("source and destination are the same"),
            Self::InsufficientSpace { needed, available } => {
                write!(
                    f,
                    "insufficient space ({needed} bytes needed, {available} available)"
                )
            }
            Self::SourceChanged => f.write_str("source changed during the operation"),
            Self::Locked => f.write_str("file is locked"),
            Self::CannotRecycle(reason) => write!(f, "the Recycle Bin refused the item: {reason}"),
            Self::RecycleUnavailable(kind) => {
                write!(f, "the Recycle Bin is unavailable on {kind:?}")
            }
            Self::RecycleFailed(code) => write!(f, "Recycle Bin error {code}"),
            Self::RestoreUnavailable => {
                f.write_str("the item cannot be restored from the Recycle Bin")
            }
            Self::VerificationFailed => f.write_str("the post-condition check failed"),
            Self::Cancelled => f.write_str("cancelled"),
            Self::Io { kind, message } => write!(f, "I/O error ({kind:?}): {message}"),
        }
    }
}

impl std::error::Error for OperationError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemFailure {
    pub path: PathBuf,
    pub error: OperationError,
}

impl ItemFailure {
    pub fn new(path: impl Into<PathBuf>, error: OperationError) -> Self {
        Self {
            path: path.into(),
            error,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_denied_maps_to_access_denied() {
        let error = io::Error::from(io::ErrorKind::PermissionDenied);
        assert_eq!(
            OperationError::from_io(&error),
            OperationError::AccessDenied
        );
    }

    #[test]
    fn unknown_io_error_keeps_kind_and_message() {
        let error = io::Error::other("disk on fire");
        let mapped = OperationError::from_io(&error);
        assert_eq!(
            mapped,
            OperationError::Io {
                kind: io::ErrorKind::Other,
                message: "disk on fire".to_owned(),
            }
        );
        assert!(mapped.to_string().contains("disk on fire"));
    }

    #[test]
    fn cancellation_is_recognised() {
        assert!(OperationError::Cancelled.is_cancelled());
        assert!(!OperationError::NotFound.is_cancelled());
    }
}
