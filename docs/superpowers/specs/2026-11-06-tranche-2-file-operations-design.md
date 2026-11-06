# Tranche 2 — File operations: design

- **Date:** 2026-11-06
- **Status:** Draft for maintainer review. No code is written against this document until it is approved. The structural decisions in sections 2 and 3 need an explicit answer, not a silent acceptance.
- **Scope source:** `PRD.en.md` section 16 Tranche 2 (`PRD.en.md:949-955`) and, as the normative detail, section 13 — `PRD.en.md:593` states that in a conflict section 13 prevails over Annex A. Relevant subsections: 13.3 selection, 13.4 file operations, 13.9 drag-and-drop, 13.18 organization, 13.21 accessibility, 13.22 shortcuts, 14.2 reliability, 15.2/15.4 tests, 16.3 Tranche 2.
- **Depends on:** Tranche 1 as implemented on `master` (listing, navigation, view model, selection, watcher, settings), ADR 0001 as accepted, and ADR 0002 (file-operations engine), proposed with this document.
- **Inherits one open risk:** Tranche 0's manual gates are still open (Windows 10 22H2, Narrator/Accessibility Insights, 60 FPS instrumentation). Tranche 1 was delivered on an unvalidated foundation by maintainer choice. Tranche 2 is the first tranche that **destroys and overwrites user data**, so the same choice is materially more expensive here: it must be re-stated, not assumed.

## 1. Scope

Tranche 2 turns the read-only browser into a file manager: copy, cut, paste, rename, create, delete to Recycle Bin, permanent delete, internal drag-and-drop, conflict handling, an activity center, undo/redo, checksums, an empty-folder cleaner, and an enriched status bar.

In scope, from PRD 13.3, 13.4, 13.9, 13.18, 16.3 and Annex A rows marked Tranche 2:

- **OPS-01** copy, cut, paste through the Windows clipboard: `CF_HDROP` plus the `Preferred DropEffect` format, interoperable in both directions with Explorer.
- **OPS-02** inline rename with `F2`, including Explorer's validation rules and case-only renames.
- **OPS-04** delete to the Recycle Bin, and explicit permanent delete with `Shift+Del` after confirmation.
- **OPS-05** undo of the last reversible operation, and redo; the UI must never offer an undo it cannot perform.
- **OPS-06** internal drag-and-drop between rows, the current folder, and (later) panes; `Ctrl` copies, `Shift` moves, default follows Explorer's same-volume/cross-volume rule.
- **OPS-09** conflict dialog: replace, skip, keep both, compare metadata, apply the decision to all similar conflicts, cancel.
- **OPS-10** create folder and file (`Ctrl+Shift+N` for a folder) with Explorer's unique-name numbering and inline editing.
- **OPS-11** rectangular (marquee) selection, invert selection, and selection by pattern, on top of Tranche 1's single/`Ctrl`/`Shift`/`Ctrl+A` selection.
- **OPS-12** checksums MD5, SHA-256 and BLAKE3 for the selection, streamed, cancellable, with progress.
- **OPS-13** empty-folder cleaner, report-first and confirmed, never touching the current root.
- **ORG-01** enriched status bar: item count, selected count, selected file bytes, free space of the current volume.
- **Activity center** (PRD 13.4): per-operation and per-file progress, current file, instantaneous and average speed, ETA, pause, resume, cancel, retry, errors, collapsible pane.
- **Accessibility and keyboard** (PRD 13.21, 13.22, 13.9): `Ctrl+C`, `Ctrl+X`, `Ctrl+V`, `F2`, `Del`, `Shift+Del`, `Ctrl+Shift+N`, `Ctrl+Z`, `Ctrl+Y`; every operation reachable without a mouse or without drag-and-drop; operation, error and undo results announced; dialogs that keep focus inside themselves and return it.

Out of scope, with the PRD location that owns it:

- **`IFileOperation`, the native progress dialog (OPS-08), native properties (OPS-14), "Open with" (OPS-15), OLE drag-and-drop (OPS-07), `IContextMenu` merging (SYS-03/04)** — Tranche 3. Tranche 2 ships its own copy engine and its own progress UI; ADR 0002 records that sequencing and its cost.
- **Batch regex rename (OPS-03)** — Tranche 7.
- **Compression/extraction (OPS-16)** — Tranche 8.
- **Full operation queue with reorder and parallelism (OPS-17 as written in Annex A), automated post-copy integrity verification (OPS-18), duplicate finder, tags** — Tranche 12. Tranche 2 runs one operation at a time and offers retry of failed items, which is the part PRD 13.4 requires of the activity center.
- **Copy path / copy name / duplicate / send-to / create shortcut** — the native menu or palette tranches (3 and 7). They are trivial but they are not in Tranche 2's content list, and this document does not smuggle them in.
- **SQLite persistence of operation state** (`fx-storage`, PRD 10.9) — Tranche 12/13. See decision 5.
- **Cross-window and cross-pane drag** — Tranche 5 owns panes and windows; the internal DnD in Tranche 2 targets rows and the current folder only.
- **Cloud, network, WSL, archive, and virtual locations** — later tranches. Operations are local-only and refuse anything else with a typed error (same discipline as `Location` in Tranche 1).

## 2. Decisions this tranche must settle before slice 1

Each item is a yes/no for the maintainer. The document is written so that each answer changes one section, not the whole design.

1. **Own transfer engine in `fx-ops`, not `IFileOperation`, for Tranche 2.** PRD 13.4 says operations go through `IFileOperation` "when possible", but PRD 16.3 assigns `IFileOperation` and the native dialog to Tranche 3. Following the roadmap means Tranche 2 writes, copies, and deletes through `std::fs` plus targeted Shell calls, and Tranche 3 replaces the transfer path with `IFileOperation`. Cost: some work is thrown away in Tranche 3. Benefit: Tranche 2 owns progress, pause, retry, conflict policy, and undo — none of which the native dialog exposes to us. Recorded in ADR 0002.
2. **Delete to Recycle Bin through legacy `SHFileOperationW` with `FOF_ALLOWUNDO`.** `IFileOperation` is the modern API and is Tranche 3's. `SHFileOperationW` is deprecated but supported, single-threaded, and has a known `MAX_PATH` weakness (section 8). If validation shows it failing on long paths, Tranche 2 reports "cannot recycle this path" and offers an explicit permanent delete instead of quietly doing something else. It never silently degrades a recycle into a permanent delete.
3. **Recycle Bin restore for undo is capability-gated, not promised.** Undo of a delete needs a Shell call to find the recycled item and invoke its restore verb. That call is validated in slice 2 against a real Recycle Bin; until then, and whenever it fails at runtime, the delete is not offered as undoable and the UI says so (PRD 13.4: "The UI must not claim an operation is reversible when it is not"). The alternative — writing the Recycle Bin's `$I`/`$R` records ourselves — is rejected as unsupported.
4. **`fx-ops` defines the platform port; `fx-app` wires it to `fx-shell`.** `fx-ops` must stay free of Win32 and of `fx-shell` (`PRD.en.md:378`), so recycling, atomic replace, free space and volume identity are a trait in `fx-ops` implemented by an adapter in `fx-app` over `fx-shell`. Tests use a fake port, including forced cross-volume and forced failure.
5. **Undo journal is in memory, session-scoped.** PRD 10.9 lists "operation state" in SQLite, but persisting an undo stack across restarts is only meaningful with Tranche 13's crash recovery, and adding `rusqlite` (native SQLite) to Tranche 2 would need its own rule 6 review. Consequence stated plainly: closing Ferrix forgets undo history; a crash loses in-flight operation state. Tranche 2 must not display a "restored session" claim.
6. **One operation at a time per window.** A copy of 500 files is one operation with an internal per-file loop; two copies started back to back are queued, not parallel. Parallelism, reordering and pause-all belong to Tranche 12's queue. This keeps pause/cancel deterministic and the progress model honest.
7. **Checksums from RustCrypto plus `blake3`, not Windows CNG.** CNG has no BLAKE3 and would add more `unsafe` for no benefit. Dependency review in section 3.
8. **Tranche 0's open gates are re-affirmed before anything reaches user data.** Slice 1 contains no UI and no Win32: every copy, move and delete it exercises runs against `tempfile` fixtures, so it can proceed. Slice 2 wires the Shell primitives and also works on fixtures. Slices 3 to 5 put those commands in front of real folders and start only after the maintainer confirms, in writing on the tranche issue, that Windows 11 manual validation is current and that Windows 10 remains an accepted gap.

## 3. New dependencies and the rule 6 review

| Crate | Version | Role | License | Native code | Notes |
|---|---|---|---|---|---|
| `md-5` | `=0.11.0` | MD5 hashing (OPS-12) | MIT OR Apache-2.0 | no | RustCrypto; declared MSRV 1.85, below the pinned 1.92. |
| `sha2` | `=0.11.1` | SHA-256 hashing (OPS-12) | MIT OR Apache-2.0 | no | RustCrypto; declared MSRV 1.88, below the pinned 1.92. |
| `blake3` | `=1.8.7` (feature `pure`) | BLAKE3 hashing (OPS-12) | CC0-1.0 OR Apache-2.0 OR Apache-2.0 WITH LLVM-exception | no, with `pure` | **Correction from compiling it:** the default x86-64 build compiles C and assembly through `cc`, so the `pure` feature is mandatory here; without it this crate is not "no native code". Pure Rust SIMD intrinsics remain, at some speed cost. CC0-1.0 is already allowed in `deny.toml` through `notify`. |

Versions were resolved against the registry on 2026-11-06 with `cargo info` (the RustCrypto 0.11 digest line, `blake3` 1.8.7) and are pinned with `=`. `docs/LEGAL.md` and `THIRD_PARTY_NOTICES.md` gained rows in the same change, and `cargo deny check` / `cargo audit` run before the first commit.

New `windows` crate features for `fx-shell`, validated by compiling in slice 2 rather than assumed here: `Win32_System_Memory` (global memory for `CF_HDROP`), `Win32_System_DataExchange` (clipboard), and whatever `ReplaceFileW`/`GetDiskFreeSpaceExW`/`IShellFolder` actually require beyond the existing `Win32_UI_Shell`, `Win32_System_Com`, `Win32_System_Ole`, `Win32_Storage_FileSystem`.

`tempfile = "=3.20.0"` is already a dev-dependency of `fx-fs` and `fx-settings`; `fx-ops` reuses the same pin.

No new workspace dependency for cancellation either: `fx-ops` gains an edge to `tokio-util` (already pinned, `sync` feature) for `CancellationToken` and to `fx-domain` for shared types. Both crates are already in `Cargo.lock`, so only the manifest edges change.

No new UI dependency. No SQLite.

## 4. Module layout

Boundaries from PRD 10.1/10.2 and `CLAUDE.md`: no `unsafe` outside `fx-shell`, `fx-terminal`, `fx-git`, `fx-a11y`; `fx-ops` never depends on `fx-ui` or `fx-shell`.

```
crates/fx-domain/src/
  keys.rs          + ShortcutAction::{Undo, Redo, CreateFolder, PermanentDelete},
                     a ctrl+shift constructor, and the PRD 13.22 mappings

crates/fx-ops/src/                 (currently an empty stub — this tranche fills it)
  lib.rs           re-exports, crate docs
  error.rs         OperationError, ItemFailure, RecycleError, RestoreError, VerifyError
  plan.rs          OperationPlan, PlanItem, plan_copy/plan_move, self-copy and nesting checks
  progress.rs      OperationId, OperationKind, OperationState, Progress, SpeedMeter
  control.rs       OperationControl: cancellation + cooperative pause
  conflict.rs      ConflictPolicy, ConflictDecision, ConflictRequest, keep-both naming
  transfer.rs      the copy and move engine over a 1 MiB chunk loop
  delete.rs        recycle request, permanent delete, post-conditions
  rename.rs        name validation, collision, case-only rename
  create.rs        create_directory / create_file with Explorer's numbering
  checksum.rs      ChecksumAlgorithm, streaming hash with progress and cancellation
  undo.rs          UndoStack, UndoEntry, ReversibleAction, reversibility capability
  clean.rs         empty-folder scan and clean plan (report first)
  port.rs          PlatformOps trait; test doubles live in the modules' test blocks and in
                   tests/transfer.rs, so no fake ships in the library

crates/fx-shell/src/
  clipboard.rs     CF_HDROP + Preferred DropEffect read/write          unsafe + SAFETY
  recycle.rs       SHFileOperationW(FOF_ALLOWUNDO) + Recycle Bin restore unsafe + SAFETY
  fs.rs            ReplaceFileW atomic replace, GetDiskFreeSpaceExW,
                   volume identity for same-volume detection           unsafe + SAFETY

crates/fx-app/src/
  operations.rs    OperationRunner: spawn_blocking tasks, event channel, pause/cancel,
                   activity model, undo stack ownership
  platform.rs      ShellPlatform: impl fx_ops::port::PlatformOps over fx_shell
  orchestration.rs + command dispatch, selection snapshot, refresh after completion

crates/fx-ui/src/
  viewmodel.rs     + selected_paths(), rename target, status totals, activity rows
  ui.slint         + imports of the two files below, status bar, rename overlay
  dialogs.slint    ConflictDialog, DeleteConfirmation, PatternSelection, ChecksumSheet
  activity.slint   ActivityCenter, ActivityRow, OperationErrors
```

`fx-ops` depends only on `fx-domain` plus the three hash crates. `fx-shell` keeps no dependency on `fx-ops`; the adapter that joins them lives in `fx-app`, exactly like the Tranche 1 ports.

## 5. Data model

```rust
pub struct OperationId(u64);                    // monotonic per process
pub enum OperationKind { Copy, Move, Rename, CreateDirectory, CreateFile,
                         Recycle, DeletePermanent, Restore, Checksum, CleanEmptyFolders }

pub enum OperationState { Planning, Running, Paused, Completed,
                          CompletedWithWarnings, Failed, Cancelled }

pub enum ClipboardEffect { Copy, Move }   // Move is "cut"; maps to DROPEFFECT_COPY/MOVE

pub struct Progress {         // one snapshot; the UI never computes speed itself
    pub files_total: u64, pub files_done: u64,
    pub bytes_total: u64, pub bytes_done: u64,
    pub current: Option<PathBuf>,
    pub bytes_per_second: u64,        // EWMA over 2 s windows
    pub eta: Option<Duration>,        // None when speed is 0 or totals unknown
}

pub enum ConflictChoice { Replace, Skip, KeepBoth, Cancel }
pub struct ConflictAnswer { pub choice: ConflictChoice, pub apply_to_all_in_class: bool }
pub enum ConflictClass { FileFile, DirDir, ReadOnly, Other }

pub struct ItemFailure { pub path: PathBuf, pub error: OperationError }

pub struct OperationSummary {
    pub id: OperationId, pub state: OperationState, pub progress: Progress,
    pub failures: Vec<ItemFailure>,
    pub error: Option<OperationError>,   // operation-level: refused before writing, or
                                         // the destination vanished mid-run; forces `Failed`
}
```

An operation refused up front — insufficient space, an unusable destination — carries its reason in `error` and reports `Failed` with nothing written, rather than an empty `Completed` or a warning list with no items.

The clipboard payload crossing the layers is `(Vec<PathBuf>, ClipboardEffect)`: `fx-ops` owns the effect enum, `fx-shell` maps it to `DROPEFFECT_*`, and `fx-app` passes the payload from one to the other. The paths are snapshotted when the command runs, not when the clipboard was written, so a file deleted in the meantime fails per item instead of making the paste look empty.

"Compare metadata" is not a decision: PRD 13.4's comparison is a dialog affordance (name, size, modified on both sides) that leaves the conflict pending. The engine's decision set stays four-way, which is what the acceptance criterion "conflicts offer the four choices" means.

**"Apply to all" is scoped to the conflict class, not the operation** (maintainer decision, 2026-11-06). `ConflictClass` distinguishes file-versus-file, directory-versus-directory, read-only-target and other conflicts; an answer with `apply_to_all_in_class` is remembered for that class only, so "replace all" on a run of files never silently replaces folders or read-only targets. Directory conflicts themselves are not prompted (rule 3): directories merge, and only files inside raise a conflict.

Undo is a stack of reversible actions, not a stack of operations:

```rust
pub struct UndoEntry { pub operation: OperationId, pub kind: OperationKind,
                       pub action: ReversibleAction, pub occurred_at: SystemTime }

pub enum ReversibleAction {
    Rename  { pairs: Vec<PathPair> },      // performed from -> to
    Move    { pairs: Vec<PathPair> },
    CreateDirectory { paths: Vec<PathBuf> },
    CreateFile      { paths: Vec<PathBuf> },
    Recycle { items: Vec<RecycledItem> },  // original path + identity derived at delete time
}
```

Copy is deliberately absent: `Ctrl+Z` after a copy is a **copy of the copy**, and offering "undo copy" means deleting data the user may have edited. Copy clears the redo stack and pushes no undo entry.

Undo semantics, written so the UI cannot overpromise:

- Undo skips copy entries (there is nothing to invert) and **stops at the first entry it cannot invert**, reporting why, instead of reaching further back and undoing something the user is no longer looking at. That is the literal reading of PRD 13.4's "the last reversible operation" without the surprise of a time-travelling `Ctrl+Z`.
- `Rename` and `Move` invert only if both endpoints are still what the entry recorded (same file identity, size and modification time). An endpoint renamed, replaced or deleted by someone else is refused.
- `CreateDirectory` and `CreateFile` invert only if the item is still empty (no entries, length zero); otherwise refused. The guard is emptiness, not a timestamp, because that is the property that makes deletion lossless. Undoing a creation never deletes user data.
- `Recycle` inverts only if the recycled item is still present and unambiguously identified (section 8); otherwise refused and not offered as undoable.
- **There is no skipping.** `can_undo` exposes only the top entry; if it cannot be inverted, `Ctrl+Z` reports why and stops (maintainer decision, 2026-11-06). Undoing an older action behind the user's back is worse than a no-op.

Every inversion re-verifies its preconditions and reports "cannot undo" rather than guessing. Redo re-applies the action and creates a fresh undo entry; redo of a delete is refused in Tranche 2, because re-deleting must remain an explicit command.

`PlatformOps` is the only way `fx-ops` reaches the outside world:

```rust
pub trait PlatformOps {
    fn volume_id(&self, path: &Path) -> io::Result<u64>;
    fn volume_kind(&self, path: &Path) -> io::Result<VolumeKind>;   // Fixed | Removable | Network | Other
    fn free_space(&self, path: &Path) -> io::Result<u64>;
    fn apply_metadata(&self, from: &Path, to: &Path) -> io::Result<()>;  // std default; Win32 adds creation time
    fn replace_file(&self, replacement: &Path, destination: &Path) -> io::Result<()>;
    fn can_recycle(&self, paths: &[PathBuf]) -> Result<RecyclePrecheck, RecycleError>;
    fn recycle(&self, paths: &[PathBuf]) -> Result<Vec<RecycledItem>, RecycleError>;
    fn restore_recycled(&self, item: &RecycledItem) -> Result<(), RestoreError>;
}
```

`can_recycle` is the cheap pre-check the maintainer asked for: volume kind plus the Recycle Bin's remaining quota (`SHQueryRecycleBin`), so an item that obviously cannot be recycled is refused before the Shell call. It is a heuristic — quota accounting races with other processes — which is exactly why `FOF_WANTNUKEWARNING` stays as the net behind it. The engine refuses on `Impossible`, and the adapter reports `Possible` for macOS-style unknown cases rather than pretending.

`recycle` takes no `OperationControl`: `SHFileOperationW` is not cancellable per item, and pretending otherwise would make the pause button lie. Cancellation around a recycle is honored between batches of items, which is what the adapter can actually deliver.

`RecycledItem` carries the original path, the recycled item's display name and its deletion time. `SHFileOperationW` returns no handle to what it recycled, so the adapter derives that identity by listing the Recycle Bin before and after the call; if the diff is ambiguous, the item is marked unverifiable and undo refuses it (section 8).

## 6. Threading, cancellation, pause

```
UI thread                 tokio blocking pool                 UI event loop
---------                 --------------------                 -------------
Ctrl+V  ---------------->  spawn_blocking(op)  --events-->  invoke_from_event_loop
   |                            |                                  |
   |                     one file at a time, 1 MiB chunks          v
   +-- OperationControl    conflict? -> ConflictRequest      activity row updated
       (cancel + pause)               -> waits for decision    dialog shown
```

- Operations run in `tokio::task::spawn_blocking` on the runtime `fx-app` already owns; no blocking call ever executes on the Slint thread (PRD 10.3, `CLAUDE.md` rule 4).
- **The clipboard is read inside the worker, not in the Slint callback.** `OpenClipboard` takes a contended global lock and can stall; a paste command is created only after the worker has successfully read `CF_HDROP` and the preferred effect.
- Volume kind and free space are queried in the worker as well. The status bar renders the last cached value for the current location and refreshes asynchronously, so a disconnected drive cannot freeze the UI.
- `OperationControl` carries a `CancellationToken` (already pinned) and a pause gate built from `AtomicBool` + `Condvar`. Pause is checked between chunks, so at most one 1 MiB chunk is in flight when the user pauses.
- Progress events are coalesced by the engine to about 10 Hz and sent through a **bounded** channel with `blocking_send`. A UI that stops reading throttles the engine instead of growing memory (PRD 10.4). Terminal events (finished, failed, item failure, conflict request) are never dropped.
- Late events are discarded by `OperationId` on the UI side, the same generation discipline Tranche 1 uses for listings.
- A conflict request pauses the worker until the dialog answers on a oneshot channel. Closing the dialog answers `Cancel`; there is no timeout that silently picks replace.
- After `Completed`, `fx-app` refreshes the current folder if it is still the folder that was operated on. The `notify` watcher stays as a second, coalesced signal, not as the only one.

## 7. Filesystem semantics and safety invariants

These are the rules the tests encode, not aspirations:

1. **Copy never deletes a source.** Move deletes a source only after the destination exists with the expected size and the write has been flushed.
2. **Replace is atomic or nothing; an ordinary copy needs no temporary file.** A copy that must overwrite an existing file is written to `<name>.fxpart-<id>` in the destination directory, then swapped with `ReplaceFileW`; on failure the temporary is removed and the original target is untouched. A copy that creates a new name streams straight to its final name, and a cancelled or failed copy removes the partial output. A crash can leave a `.fxpart-` file; leftovers are removed at the start of the next operation that writes in that directory (a global sweep is Tranche 13's).
3. **Directories merge; they are not replaced.** "Replace" on a directory means "merge and apply this policy to the conflicts inside it", which is Explorer's behavior. Deleting the target directory first is never an option.
4. **Paths are built from validated names only.** A child name containing a separator, `.`/`..`, or a reserved device name is rejected at plan time. Nothing is joined from unvalidated input.
5. **Copying a directory into itself or into its own descendant is refused** at plan time, after verbatim-path normalization.
6. **Reparse points are never traversed.** Slice 1 has no Win32 and `std` cannot reliably tell a symbolic link from a junction, so every reparse point is reported as `ReparsePointUnsupported` and skipped rather than followed, recreated, or guessed at. Slice 2's Windows adapter refines this: symbolic links are recreated as links, junctions stay skipped. The conservative behaviour is what ships until that adapter is validated on Windows (rule 8).
7. **Metadata and read-only targets.** Copies preserve the modification time, creation time and attributes; the read-only attribute is applied after the write, never before it. Alternate data streams, EFS encryption, compression attributes and sparse extents are **not** copied in Tranche 2 (listed in section 15). A read-only target is replaced only after the user's conflict decision.
8. **Locked files fail per item.** The operation continues, the failure is listed in the activity center, and the final state is `CompletedWithWarnings` — never `Completed`.
9. **Free space is checked cheaply, twice.** Before starting, the summed size of the selected *files* plus 32 MiB headroom must fit on the destination, otherwise the operation is refused with a typed error before writing anything. Directories are not pre-scanned — walking a large tree twice is worse than failing late — so free space is checked again before each file; running out mid-operation fails the remaining items as `InsufficientSpace` and yields `CompletedWithWarnings`, never a partial file left behind.
10. **Names follow Explorer's rules**: `< > : " / \ | ? *` and control characters rejected, trailing dot and space rejected, `CON`, `PRN`, `AUX`, `NUL`, `COM1`-`COM9`, `LPT1`-`LPT9` rejected with or without an extension, collisions detected case-insensitively against a directory name set.
11. **Case-only rename** is attempted directly, then as a two-step rename through a temporary name in the same directory if Windows refuses the direct call. One of the two works on NTFS; the test records which, on which build.
12. **Move fast path**: same `volume_id` uses `std::fs::rename`; different volume falls back to copy-then-delete with the copy's verification first.
13. **No false success**: every operation ends with a post-condition check (destination exists with expected length, source absent where it should be). A mismatch downgrades the result and is shown as an error, per PRD 14.2.
14. **Recycling only exists on fixed local volumes.** `FOF_ALLOWUNDO` silently degrades to a permanent delete on network volumes and on some removable media. `volume_kind` gates the command: on a non-fixed volume the recycle is refused with `RecycleUnavailable` and only an explicitly confirmed permanent delete is offered. Nothing is ever converted silently.
15. **Same-folder operations are defined, not accidental.** `Ctrl+V` in the source folder is allowed and uses keep-both naming (`name (2).ext`): the user asked for a duplicate. Dropping a row back onto its own folder, or onto itself, is a no-op with a status message. Copying a file onto its own path is a keep-both duplicate, not a self-copy error; the self-copy refusal in rule 5 is about a directory and its descendants.

Deleting to the Recycle Bin is not a filesystem operation in the `std::fs` sense: `delete.rs` builds the request and the recycled identity, `PlatformOps::recycle` performs it, and the post-condition is "every source path is gone". Permanent delete is `fx-ops`' own recursive walk, bottom-up, refusing reparse-point traversal, and is only reachable through an explicit confirmed command.

## 8. Shell surface: clipboard, Recycle Bin, atomic replace

All of it lives in `fx-shell`, each block with a `// SAFETY:` comment, each call validated on real Windows (PRD 10.5, `CLAUDE.md` rules 3 and 8).

**Clipboard (`clipboard.rs`).** Write: `OpenClipboard(NULL)` (retried about five times over 100 ms because the clipboard is a shared, contended resource, then a typed error), `EmptyClipboard`, a `GMEM_MOVEABLE` `HGLOBAL` holding a `DROPFILES` header with `fWide = TRUE` followed by double-null-terminated UTF-16 paths, `SetClipboardData(CF_HDROP)`, and a `DWORD` under the registered `Preferred DropEffect` format (`DROPEFFECT_COPY` or `DROPEFFECT_MOVE`). Read: `CF_HDROP` via `DragQueryFileW`, optional effect, absolute paths only, copy when the format is absent. Ownership transfers to the system on `SetClipboardData`; the handle is never freed by us. `OleSetClipboard` is not used — that is drag-and-drop plumbing, Tranche 3.

Cut tracking is clipboard state, not app state: after a successful move-paste the clipboard is emptied (Explorer's behavior; validated in slice 2), and a failed paste leaves it intact. Ferrix never mutates the clipboard on its own outside a user command.

**Recycle Bin (`recycle.rs`).** `volume_kind` and `can_recycle` run first: a non-fixed volume or an item that provably exceeds the bin's remaining quota is refused before anything is deleted (rule 14). Then `SHFileOperationW` with `FO_DELETE`, `FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_NOERRORUI | FOF_WANTNUKEWARNING`, a double-null-terminated UTF-16 `pFrom`, `hwnd` from the Slint window when available, and both the return code and `fAnyOperationsAborted` checked. `FOF_WANTNUKEWARNING` is kept deliberately: when Windows cannot recycle an item (too large for the bin, a volume without one) it shows its own warning instead of deleting silently. That is the single native dialog Tranche 2 accepts, and safety wins over the "no native dialog before Tranche 3" aesthetic; it is recorded as a deviation in section 14. The API is documented as not accepting `\\?\` verbatim prefixes, so paths are passed in their display form; long paths are therefore the known weakness of this call. Slice 2 measures it on Windows 11 with a >260-character fixture and the result goes into `docs/tranche-2-validation.md`. Failure yields `RecycleError` and the UI offers a confirmed permanent delete; it never performs one silently.

Restore for undo enumerates the Recycle Bin's shell folder and invokes the item's restore verb. Because the delete call returns no handle, the journal's identity comes from the before/after diff described in section 5: original path, recycled display name, deletion time. If two identical deletes land in the same second and the diff cannot tell them apart, the item is marked unverifiable and the undo is refused rather than restoring someone else's file. This needs a real COM STA on the worker thread (explicitly initialized, per PRD 10.5) and a manual test: delete, `Ctrl+Z`, file back at its original path with its name and contents.

**Atomic replace and volume identity (`fs.rs`).** `ReplaceFileW` for rule 2 above, `GetDiskFreeSpaceExW` for ORG-01 and the precheck, and volume identity from the volume serial number for the same-volume move decision.

## 9. Activity center, dialogs, and accessibility

PRD 13.21 applies to every UI tranche, and the Tranche 0 measurement already showed that Slint's `ListItem` carries an empty accessible name unless the row root sets one. The same discipline applies to the new surfaces:

- **Activity center**: one collapsible pane at the bottom, one row per operation with kind, targets summary, state, progress, bytes, speed, ETA, current file, and buttons Pause/Resume, Cancel, Retry (retry starts a new operation over the failed items). Errors expand in place. History is capped at 50 rows per session and cleared on exit: it is a view, not a log. Every control has a name and a role; if Slint exposes no progress-bar role at the pinned version, the bar is exposed as a `text` element whose accessible name carries "42 percent, 3 of 7 files" rather than being unlabeled — checked against Slint's documented role list at slice 4, not assumed here.
- **Conflict dialog**: modal, focus stays inside, initial focus on the default button, `Escape` cancels the whole operation, four named choices plus "apply to all", metadata comparison for both items, and a readable label naming the file. Closing it answers Cancel.
- **Delete confirmation**: only for permanent delete and for the empty-folder cleaner; a recycle-bin delete needs no confirmation (PRD 14.2 allows either a confirmation or the Recycle Bin). The permanent-delete dialog states the item count and that the action cannot be undone.
- **Inline rename**: an overlay editor on the row, name pre-selected up to the extension, `Enter` commits, `Escape` reverts, validation errors shown next to the field and announced, focus returned to the row afterwards.
- **Announcements**: operation started, finished, finished with errors, failed, cancelled, undone, undo refused — through the existing status region. The selection count announcement from Tranche 1 stays.
- **Keyboard-only path**: every Tranche 2 action is reachable through a shortcut, the menu, or both. Internal drag-and-drop's keyboard alternative is Cut/Paste, which PRD 13.9 requires.
- **Manual half**: Narrator and Accessibility Insights are still unavailable on this host. The document and the acceptance checklist name that as open rather than claiming a pass.

## 10. Error handling

`OperationError` is typed and total: `AccessDenied`, `NotFound`, `AlreadyExists`, `InvalidName(reason)`, `PathTooLong`, `ReparsePointUnsupported`, `SourceInsideDestination`, `InsufficientSpace { needed, available }`, `SourceChanged`, `Locked`, `RecycleUnavailable(VolumeKind)`, `RecycleFailed(code)`, `RecycleRestoreUnavailable`, `Cancelled`, `Io`. Per-item errors become `ItemFailure` entries; operation-level errors end the operation.

The rule from PRD 14.2 is enforced structurally: a success result is only constructed after the post-condition check, so "Completed" cannot be displayed for a copy whose destination is missing. Cancellation is a user action and ends quietly as `Cancelled` with the count of completed items; it is never shown as success. Nothing panics on a user path, and no error is swallowed: `tracing` records it, the activity center lists it, and the status region announces it.

## 11. User-visible strings

Every new string goes into `resources/en-US.rs` (Rust-side: error sentences, confirmation text, announcements, activity labels) and `resources/en-US.slint` (dialog and activity-center text), per `CLAUDE.md` and PRD 859. No literal user-visible text in UI code. The existing test that asserts resource/native title consistency is extended to cover the new dialog titles. English only, with the resource layout ready for `fr-FR` (already a listed good-first contribution).

Paths are shown in display form; any `\\?\` prefix is stripped at the boundary so it never reaches a label, as in Tranche 1.

## 12. Testing

- **`fx-ops` unit tests** (pure, no Windows): plan and collision detection, self-copy and descendant copy refusal, the name-validation table (reserved devices, invalid characters, trailing dot/space, Unicode, case-insensitive collision), keep-both numbering, conflict policy application and "apply to all" propagation, pause/resume/cancel state machine, speed meter and ETA, undo/redo stack behavior including refused undo, checksum test vectors for empty input and `"abc"` for all three algorithms, empty-folder detection (bottom-up, skips non-empty, skips reparse points, never the root), refusal to recycle on a non-fixed volume, the undo-of-creation guards (empty/unmodified versus modified), same-folder paste naming and the no-op drop rules, and temp-file discipline (a successful ordinary copy leaves no `.fxpart-`, a cancelled one leaves no partial file).
- **`fx-ops` integration tests** (`tempfile`, Windows-labelled like Tranche 1's): copy a tree with Unicode names, long-path fixture with the verbatim prefix, read-only attributes, a locked file producing one `ItemFailure` and `CompletedWithWarnings`, a simulated failure after N bytes leaving no partial target and an intact original, forced cross-volume move through the fake port, replace over an existing file, directory merge.
- **Robustness (PRD 15.4)**: full disk simulated by a failing writer port, denied permissions on a real directory, source removed mid-copy, cancel mid-copy, pause then cancel, crash-leftover `.fxpart` cleanup on the next operation in that directory.
- **`fx-ui`**: view-model tests for selection-to-paths, status totals, and conflict/activity row mapping without a window; UIA assertions for the accessible names and roles of the new controls.
- **`fx-shell`**: no COM unit tests. A documented manual checklist in `docs/tranche-2-validation.md` (PRD 15.3): delete to Recycle Bin and find the file there, restore via `Ctrl+Z`, copy Ferrix→Explorer and Explorer→Ferrix with `CF_HDROP`, cross-volume move on two real volumes, `ReplaceFileW` over an open target, long-path recycle behavior.
- **Regression**: Tranche 1's tests stay green, including listing, selection survival, and the long-path fixtures. No assertion is weakened and no lint suppressed; if a gate fails, the cause is fixed.
- Every slice ends with `cargo fmt --all -- --check`, `cargo check --workspace --all-targets`, `cargo test --workspace --all-targets`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo deny check`, `cargo audit`.

## 13. Delivery slices

Each slice compiles, is tested, and is reviewable on its own. Branch: `tranche-2-file-operations`, one branch for the tranche (`CLAUDE.md` rule 1).

1. **Noyau `fx-ops`** — errors, port and fake, plan, progress/control, conflict policies, transfer (copy + move), rename, create, checksum, undo stack, empty-folder scan, permanent delete. No UI, no Win32, no recycling; reparse points are skipped conservatively (rule 6). Gate: `cargo test -p fx-ops` green, including the name table, the transfer integration tests, and the robustness tests.
2. **Adaptateurs Shell** — `fx-shell` clipboard, recycle, atomic replace, free space, volume identity, plus symlink-versus-junction classification and creation-time preservation; `fx-app/platform.rs` adapter; `docs/tranche-2-validation.md` written and executed for the Shell half (delete to bin, restore, clipboard both ways, long paths). Gate: manual checklist signed off, logs attached.
3. **Commandes, édition, statut** — shortcut map additions, dispatch, inline rename, create folder/file, permanent-delete confirmation, status bar totals and free space (ORG-01), announcements, all strings externalized.
4. **Conflits, activité, annulation** — conflict dialog with apply-to-all and metadata, activity center with pause/resume/cancel/retry and error list, undo/redo wiring over the journal, checksum sheet for the selection.
5. **Sélection avancée, DnD interne, rangement** — marquee selection, invert, pattern selection, internal drag-and-drop with `Ctrl`/`Shift` and Explorer's default, empty-folder cleaner, README/CLAUDE status updates, and the Tranche 2 conformance audit against every acceptance criterion in `PRD.en.md:955`.

## 14. PRD conformance audit

| PRD source | Requirement | Status in this spec |
|---|---|---|
| 949-955 | copy/cut/paste, `CF_HDROP` + `Preferred DropEffect` | **Covered** — OPS-01, sections 4 and 8, slice 2-3 |
| 949-955 | rename `F2`, create folder and file | **Covered** — OPS-02, OPS-10, sections 7 and 13 |
| 949-955 | delete to Recycle Bin, permanent delete `Shift+Del` | **Covered** — OPS-04, sections 7 and 8, with the long-path and restore risks named |
| 949-955 | multiple and rectangular selection | **Covered** — OPS-11 plus invert and pattern, slice 5 |
| 949-955 | internal drag-and-drop | **Covered** — OPS-06, slice 5; keyboard alternative is cut/paste (13.9) |
| 13.4:635 | four conflict choices, compare metadata, apply to all similar conflicts, cancel | **Covered** — `ConflictChoice` + `ConflictClass`, section 5 and 9; apply-to-all is per class |
| 13.4:633 | activity center: progress, speed, ETA, pause/resume/cancel/retry, errors, collapsible | **Covered** — section 9, slice 4; queue reorder and parallelism deferred to T12 (OPS-17) |
| 13.4:633 | native progress dialog / `IFileOperation` | **Deviation, safety-motivated** — one Shell warning may appear when Windows cannot recycle (`FOF_WANTNUKEWARNING`); all other progress UI is ours |
| 13.4:637 | undo/redo records rename, move, folder creation, Recycle Bin | **Covered** — `ReversibleAction`; tags and pins belong to T4/T12 and are absent here |
| 13.4:637 | never claim reversibility that does not exist | **Covered** — capability-gated undo, refusals surfaced, section 5 |
| 13.4:639 | checksums MD5, SHA-256, BLAKE3 | **Covered** — OPS-12, `checksum.rs`; automatic post-copy verification is OPS-18, T12 |
| 13.18 / OPS-13 | empty-folder cleaner | **Covered** — report-first, confirmed, slice 5 |
| ORG-01 | enriched status bar | **Covered** — slice 3 |
| 13.22:787-790 | `Ctrl+C/X/V`, `F2`, `Del`, `Shift+Del`, `Ctrl+Z` | **Covered** — `fx-domain/keys.rs` additions |
| 13.21:774-776 | names, roles, states, announcements, focus, keyboard-only | **Covered in design; manual Narrator/Accessibility Insights half stays open** |
| 14.2:816 | no destructive action without confirmation or Recycle Bin; never confirm success early | **Covered** — rules 13 and the post-condition design of section 10 |
| 15.2:876, 15.4:884 | temp-filesystem copy/move, Recycle Bin, locked files, crash during copy | **Covered** — section 12 |
| 10.2:378 | `fx-ops` never depends on `fx-ui`/`fx-shell` | **Covered** — `PlatformOps` in `fx-ops`, adapter in `fx-app` |
| 10.5:403 | COM per-thread init, RAII, `unsafe` confined | **Covered** — section 8 |
| 16.3:953 | `IFileOperation` and native dialog | **Deferred to Tranche 3** — ADR 0002, decision 1; the tranche ships its own engine and dialog |
| 10.9:459 | SQLite for operation state | **Deliberate deviation** — in-memory journal, decision 5 |
| 13.4:631 | activity center "when possible" through `IFileOperation` | **Deliberate deviation** — same as OPS-08; PRD 16.3 puts `IFileOperation` in Tranche 3 |

## 15. What this design does not resolve

- **Tranche 0 acceptance is still open**, and Tranche 2 is the first tranche that can destroy data. No destructive slice starts without the maintainer's written re-affirmation (decision 8).
- **`SHFileOperationW` on long paths** is unmeasured. If it fails above `MAX_PATH`, recycling is unavailable for those paths and the UI offers a confirmed permanent delete. That is a documented limitation, not a workaround.
- **Recycle Bin restore** may not be reachable through the item's restore verb on every Windows build. If the slice 2 spike fails, deletes are not undoable and the UI must say so, which satisfies PRD 13.4's honesty rule but not its "restore from Recycle Bin" content. That gap would need a Tranche 3 (`IFileOperation`) fix and would be raised on the tranche issue.
- **Cross-volume move** cannot be fully tested on a single-volume CI machine. The fake port covers the branch; the real two-volume test is manual.
- **Pause/resume granularity** is one 1 MiB chunk; a single huge file pauses within one chunk of the button press, never mid-write.
- **Slint dialog semantics** (focus trap, modal behavior, accessible progress role, IME in the inline editor) are unverified until slice 4 and are named in the checklist rather than assumed.
- **60 FPS scrolling** still has no frame-level instrumentation, so "the activity center did not regress scrolling" cannot be claimed, only inspected.
- **Crash recovery** of in-flight operations is Tranche 13; Tranche 2 leaves `.fxpart` leftovers that are cleaned lazily.
- **Operation persistence** is absent by decision 5; a restart forgets undo history and any paused operation.
- **Reparse points are skipped wholesale in slice 1** because `std` cannot distinguish a symlink from a junction; recreating links is deferred to the slice 2 adapter.
- **Redo of a delete is refused** rather than re-recycling: re-deleting stays an explicit command.
- **Copies are not metadata-complete.** Alternate data streams, EFS encryption, the compression attribute and sparse extents are not preserved; a copy of an encrypted or sparse file is a plain file. Windows' own `copy` behaves the same way, but the document states it rather than letting a user discover it.
- **One native dialog can appear**: the Shell's "cannot recycle, delete permanently?" warning (`FOF_WANTNUKEWARNING`). It is accepted on safety grounds and listed as a deviation; the `SHQueryRecycleBin` pre-check keeps it rare, not routine.
- **Undo of a delete can be unavailable**: the Recycle Bin diff is ambiguous for two identical deletions in the same second, and the restore verb itself is unvalidated until slice 2.
