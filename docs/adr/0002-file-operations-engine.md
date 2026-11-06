# ADR 0002: File-operations engine for Tranche 2 and Tranche 3

- **Status:** Accepted — Tranche 2 engine implemented (Windows validation pending, `docs/tranche-2-validation.md`); the Tranche 3 `IFileOperation` adapter is **not started** — no `IFileOperation` code exists in the tree
- **Date:** 2026-11-06
- **Decision owners:** Ferrix maintainers
- **Related:** `PRD.en.md` 13.4 (operations, activity center), 16.3 Tranche 2 and Tranche 3; `docs/superpowers/specs/2026-11-06-tranche-2-file-operations-design.md`;

## Context

PRD 13.4 says long operations go through `IFileOperation` "when possible", with the native progress dialog (OPS-08). PRD 16.3, however, assigns `IFileOperation` and the native progress dialog to **Tranche 3**, while Tranche 2 must deliver copy/cut/paste, rename, delete to Recycle Bin, conflicts, an activity center, undo/redo, and checksums.

Those two statements cannot both be satisfied literally in Tranche 2. Something has to be sequenced: either Tranche 2 pulls `IFileOperation` forward from Tranche 3, or Tranche 2 ships its own transfer engine and Tranche 3 replaces it.

`IFileOperation` is a progress-and-conflict UI owned by the Shell. It reports progress through `IFileOperationProgressSink`, which the Shell drives, and it presents its own conflict and progress dialogs. Exposing our own activity center, a cooperative pause, a retry of failed items, a keep-both policy with our own naming, and an undo journal built from the performed actions means owning the transfer loop. Wrapping `IFileOperation` in Tranche 2 would either hide our activity center behind a native dialog or require reimplementing what we just delegated.

## Decision

**Tranche 2 implements its own transfer engine in `fx-ops`** (`std::fs` plus targeted Shell calls for the operations `std::fs` cannot express), and **Tranche 3 replaces the transfer path with `IFileOperation`** for Explorer-identical behavior and the native progress dialog.

Concretely:

- `fx-ops` owns planning, copying, moving, renaming, creating, permanent deletion, conflict policy, progress, pause/cancel, checksums, and the undo stack. It contains no Win32.
- `fx-shell` owns the operations that require the Windows Shell: Recycle Bin delete (`SHFileOperationW` with `FOF_ALLOWUNDO`), Recycle Bin restore for undo (shell-folder enumeration + the item's `restore` verb — decision 3), `ReplaceFileW` for atomic replacement, and free space/volume identity.
- `fx-app` wires the two through a `PlatformOps` trait defined in `fx-ops`, keeping the PRD 10.2 dependency rule intact.
- **Tranche 3 will add (not started):** An `IFileOperation` adapter in `fx-shell` that replaces the `fx-ops` transfer loop. The adapter should preserve undo entries, conflict policies, and name validation from the Tranche 2 engine, while delegating progress and conflict UI to the native dialog.

## Consequences

- **Accepted cost:** The transfer loop in `fx-ops/transfer.rs` will be replaced by the Tranche 3 `IFileOperation` adapter. The planning, conflict policy, name validation, checksum, and undo code survive, because those are not things `IFileOperation` provides. Nothing has been replaced yet; the Tranche 2 loop is the only production path.
- **Planned in Tranche 3:** The `IFileOperation` adapter provides native progress dialog (OPS-08) and native conflict UI. Undo entries and conflict outcomes from Tranche 2 are preserved through a conformance layer.
- **Accepted cost:** Tranche 2 uses `SHFileOperationW` for Recycle Bin deletion; restore for undo uses the shell item's `restore` verb, which is not yet validated on every target build (`docs/tranche-2-validation.md` rows 1–3). The long-path (`\\?\`) weakness remains documented and the code refuses paths ≥ 260 up front instead of guessing.
- **Benefit:** the activity center, pause/resume, per-item error list, keep-both naming, and undo journal are ours from the start, and they are testable without COM.
- **Benefit:** Tranche 3 will deliver OPS-08 (native progress dialog) and OPS-14/OPS-15 (native properties/"Open with") as specified in the PRD.
- **Risk:** Conformance between the two implementations must be verified when Tranche 3 starts — the adapter must produce identical undo entries and conflict outcomes. Recorded then in `docs/tranche-3-validation.md`.
- **Risk:** Windows 10 behavior of `IFileOperation` and of the Recycle Bin restore verb is unverified on this host. The restore verb runs through a Shell API that is validated by the slice-2 checklist on Windows 11 first; failure reduces undo scope and is surfaced to the user rather than papered over.
