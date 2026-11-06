# ADR 0002: File-operations engine for Tranche 2

- **Status:** Proposed — awaiting maintainer decision before Tranche 2 slice 2
- **Date:** 2026-11-06
- **Decision owners:** Ferrix maintainers
- **Related:** `PRD.en.md` 13.4 (operations, activity center), 16.3 Tranche 2 and Tranche 3; `docs/superpowers/specs/2026-11-06-tranche-2-file-operations-design.md`

## Context

PRD 13.4 says long operations go through `IFileOperation` "when possible", with the native progress dialog (OPS-08). PRD 16.3, however, assigns `IFileOperation` and the native progress dialog to **Tranche 3**, while Tranche 2 must deliver copy/cut/paste, rename, delete to Recycle Bin, conflicts, an activity center, undo/redo, and checksums.

Those two statements cannot both be satisfied literally in Tranche 2. Something has to be sequenced: either Tranche 2 pulls `IFileOperation` forward from Tranche 3, or Tranche 2 ships its own transfer engine and Tranche 3 replaces it.

`IFileOperation` is a progress-and-conflict UI owned by the Shell. It reports progress through `IFileOperationProgressSink`, which the Shell drives, and it presents its own conflict and progress dialogs. Exposing our own activity center, a cooperative pause, a retry of failed items, a keep-both policy with our own naming, and an undo journal built from the performed actions means owning the transfer loop. Wrapping `IFileOperation` in Tranche 2 would either hide our activity center behind a native dialog or require reimplementing what we just delegated.

## Decision

**Tranche 2 implements its own transfer engine in `fx-ops`** (`std::fs` plus targeted Shell calls for the operations `std::fs` cannot express), and **Tranche 3 replaces the transfer path with `IFileOperation`** for Explorer-identical behavior and the native progress dialog.

Concretely:

- `fx-ops` owns planning, copying, moving, renaming, creating, permanent deletion, conflict policy, progress, pause/cancel, checksums, and the undo stack. It contains no Win32.
- `fx-shell` owns the four operations that require the Windows Shell: Recycle Bin delete (`SHFileOperationW` with `FOF_ALLOWUNDO`), Recycle Bin restore for undo, `ReplaceFileW` for atomic replacement, and free space/volume identity.
- `fx-app` wires the two through a `PlatformOps` trait defined in `fx-ops`, keeping the PRD 10.2 dependency rule intact.

## Consequences

- **Accepted cost:** Tranche 3 discards the transfer loop in `fx-ops/transfer.rs` and its progress plumbing. The planning, conflict policy, name validation, checksum, and undo code survive, because those are not things `IFileOperation` provides.
- **Accepted cost:** Tranche 2 uses the deprecated `SHFileOperationW` for recycling. It is documented as not accepting `\\?\` verbatim paths, which is a known long-path weakness; the alternative (`IFileOperation`) belongs to Tranche 3.
- **Benefit:** the activity center, pause/resume, per-item error list, keep-both naming, and undo journal are ours from the start, and they are testable without COM.
- **Benefit:** Tranche 2 stays inside its own branch and does not implement Tranche 3 features, per `CLAUDE.md` rule 1.
- **Risk:** two transfer implementations exist across two tranches. Mitigated by keeping the platform seam narrow (`PlatformOps`) and by a conformance test that the Tranche 3 replacement reproduces the same undo entries and conflict outcomes.
- **Risk:** Windows 10 behavior of `SHFileOperationW` and of the Recycle Bin restore verb is unverified on this host. Both are validated in Tranche 2 slice 2 and recorded in `docs/tranche-2-validation.md`; failure reduces undo scope and is surfaced to the user rather than papered over.
