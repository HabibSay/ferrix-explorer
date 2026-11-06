//! File operations for Ferrix Explorer — Tranche 2, slice 1.
//!
//! This crate owns planning, copying, moving, renaming, creation, permanent deletion,
//! conflict policy, progress, pause/cancel, checksums, the undo journal, and the
//! empty-folder scan. It contains no Win32 and no UI: the four operations that need the
//! Windows Shell (Recycle Bin delete and restore, atomic replace, free space) sit behind
//! [`port::PlatformOps`], implemented in `fx-app` over `fx-shell`.
//!
//! Design: `docs/superpowers/specs/2026-11-06-tranche-2-file-operations-design.md`.

pub mod checksum;
pub mod clean;
pub mod conflict;
pub mod control;
pub mod create;
pub mod delete;
pub mod error;
pub mod plan;
pub mod port;
pub mod progress;
pub mod rename;
pub mod transfer;
pub mod undo;

pub use error::{ItemFailure, OperationError};
pub use plan::{OperationPlan, PathPair, PlanItem, TransferKind, plan_transfer};
pub use port::{PlatformOps, RecycledItem, VolumeKind};
pub use progress::{OperationId, OperationKind, OperationState, OperationSummary, Progress};
pub use transfer::Transfer;
