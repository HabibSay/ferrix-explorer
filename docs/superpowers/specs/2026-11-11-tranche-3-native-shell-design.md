# Tranche 3 — Native shell: design

- **Date:** 2026-11-11
- **Status:** Draft for maintainer review. The module structure was approved by the maintainer on 2026-11-11 with thirteen adjustments; the numbered decisions in section 2 are the questions that approval left open and each needs an explicit yes/no. No code is written against this document until it is approved.
- **Scope source:** `PRD.md` section 16.3 Tranche 3 (`PRD.md:957-963`) and, as the normative detail, section 13 — `PRD.md:629` states that in a conflict section 13 prevails over Annex A. Relevant subsections: 13.4 file operations (`PRD.md:629-640`), 13.8 context menu (`PRD.md:659-665`), 13.9 drag-and-drop (`PRD.md:667-671`), 13.19 properties (`PRD.md:756-760`), 13.21 accessibility (`PRD.md:772-776`), 13.22 shortcuts (`PRD.md:792`), 15.2-15.4 tests (`PRD.md:876-886`), the API-fallback rule (`PRD.md:112`), the risk rows (`PRD.md:1078`, `PRD.md:1080`), and Annex A rows **OPS-07, OPS-08, OPS-14, OPS-15** (`PRD.md:1314-1322`), **SYS-03, SYS-04** (`PRD.md:1403-1404`). ADR 0002 (file-operations engine) is the accepted baseline this tranche extends.
- **Depends on:** Tranche 2 as implemented on `tranche-2-file-operations` (transfer engine, conflict dialog, activity center, undo journal, `PlatformOps` seam, command dispatch). This branch stacks on it until Tranche 2 merges; one tranche, one branch (`CLAUDE.md` rule 1).
- **Inherits open risks:** Tranche 0/1/2 manual validation rows are still open (`docs/tranche-2-validation.md`). The maintainer's structure validation on 2026-11-11 is the go-ahead for this tranche; it does not close those rows, and the destructive native-transfer slice ships last regardless.

## 1. Scope

Tranche 3 gives Ferrix the Windows shell behaviors users expect from Explorer: a real context menu with third-party extensions, OLE drag-and-drop in both directions, the native transfer dialog, native properties and "Open with", and list thumbnails with a disk cache.

In scope, from PRD 16.3 (`PRD.md:961`), 13.4, 13.8, 13.9, 13.19 and Annex A rows assigned to tranche 3:

- **SYS-03** native context menu: `IContextMenu` merge of Shell verbs, submenus and third-party extensions (7-Zip, Git) for a file, a multi-selection and the folder background, plus the Ferrix command section — on right-click and on the keyboard (`Menu`, `Shift+F10`, `PRD.md:792`).
- **SYS-04** the Ferrix section itself: the existing Tranche 2 command set plus copy path, copy name, properties and open with; custom commands stay data-driven so later tranches extend them without touching menu plumbing.
- **OPS-08** `IFileOperation` with the native progress dialog for copy and move, integrated with the activity center (progress, errors, cancel, retry) and the undo journal.
- **OPS-07** OLE drag-and-drop in both directions: a real `IDropTarget` for drops from Explorer and other OLE applications into Ferrix, and a `DoDragDrop` source for drags out of Ferrix to the Desktop, Explorer and OLE applications, with the copy/move/shortcut modifiers (`PRD.md:669`).
- **OPS-14** native properties sheet (`ShellExecuteEx`, verb `properties`, `PRD.md:1321`).
- **OPS-15** native "Open with".
- **Miniatures** (`PRD.md:961`): list thumbnails fetched through the Shell's thumbnail pipeline with a purgeable disk cache under `%LOCALAPPDATA%\FerrixExplorer\cache` (`PRD.md:457`). Decision 6 covers the interface question.
- **Stability architecture** (`PRD.md:663`): opening-time measurement, a timeout, technical logging, a local extension blocklist, extension disablement, an out-of-process auxiliary helper if that isolation proves feasible, and a fallback to a minimal Ferrix menu.
- **Routing** (`PRD.md:633`): operations pass through `IFileOperation` "when possible", with the native progress dialog as the visible result of that path.

Out of scope, with the PRD location that owns it:

- **OPS-17 full operation queue** (reorder, parallelism) — Tranche 12 (`PRD.md:1324`). One operation at a time stays, and the native dialog is that one operation.
- **OPS-18 post-copy integrity verification** — Tranche 12 (`PRD.md:1325`, also stated in the Tranche 2 spec). The native path must simply not preclude it later: the sink already yields source/destination pairs per item, so no T3 work is required. *(Correction: an earlier conformance pass in this session listed OPS-18 as a Tranche 3 gap; the backlog assigns it to tranche 12.)*
- **PRV-09 rich metadata panel** (checksum, EXIF, alternate streams in our own sheet) — Tranche 12 (`PRD.md:1345`). Tranche 3 opens the native sheet only.
- **Compression/extraction** — Tranche 8. **Batch rename** — Tranche 7. **Duplicate, create shortcut as menu commands, command palette** — Tranche 7 (the Tranche 2 spec's scope ruling, `2026-11-06` spec line 35). The `Ctrl+Shift` shortcut *modifier during a drag* is in scope (decision 8).
- **Sidebar and tab drop targets** — Tranche 4/5 own those surfaces (`PRD.md:669` lists them; this tranche delivers folder, pane-free targets: rows, current folder, background, external apps).
- **ACL operations** — v2, SYS-08 (`PRD.md:760`, `PRD.md:1408`).
- **Registering Ferrix as a thumbnail provider for other applications** — see decision 6; no Ferrix file types exist yet that would need it.
- **Cloud, network, WSL, archive, virtual locations** — later tranches, same refusal discipline as Tranches 1-2.

## 2. Decisions this tranche must settle before slice 1

Each item is a yes/no for the maintainer. The document is written so that each answer changes one section, not the whole design.

1. **Routing is decided by a plan-time gate: native only for clean copy/move.** Conflicts are discovered mid-transfer by `fx-ops/transfer.rs` today, so `fx-ops` gains a cheap destination scan (one stat per planned entry; a recursive probe when the plan contains directories) that runs after `plan_transfer`. The route is `IFileOperation` only when all of these hold: the scan finds no existing destination, the plan contains no reparse point (Tranche 2's conservative rule 6 must not silently change behaviour under the shell), the kind is copy or move, and the native API probes available. Anything else runs the Tranche 2 engine with its four-choice dialog. Rationale: PRD 13.4 requires the full OPS-09 conflict UX (replace, skip, keep both, compare metadata, apply-to-all per class) which the native dialog cannot express, while PRD 16.3 assigns the native progress dialog to this tranche and the 2 GB acceptance criterion (`PRD.md:963`) is a clean copy. The cost is one extra stat pass over the planned entries; the benefit is that no user-visible conflict behaviour regresses.
2. **A conflict that appears after the gate on the native path is handled by the native dialog, and that deviation is recorded.** The race window between gate and execution is real (another process creates the destination). `IFileOperation` then shows its own three-choice confirmation — replace, skip, keep both — which lacks "compare metadata" and per-class apply-to-all. Rejected alternative: routing every copy through the built-in engine "just in case", which would abandon the native dialog entirely and fail `PRD.md:961`. The deviation is listed in section 14 and validated as rare, not eliminated.
3. **Native transfers feed the existing undo journal; the Shell's own undo records are not used.** The progress sink's pre/post item pairs become `ReversibleAction::Move` entries (copy still pushes nothing, matching the Tranche 2 semantics). `FOF_ADDUNDORECORD` stays unset. A conformance test in `fx-app` runs the same fixture through both routes and asserts the resulting undo stacks are equivalent, so `Ctrl+Z` cannot tell the routes apart.
4. **The menu is shown on a dedicated STA helper thread with a watchdog timeout, pending spike validation.** `QueryContextMenu` is a synchronous call into third-party code; in-process there is no way to interrupt it. The design therefore runs `QueryContextMenu` + `TrackPopupMenuEx` on a helper STA thread with its own message pump: if the menu has not appeared within the timeout budget (proposed 2 s, one tunable constant), the UI thread logs a warning, shows the minimal Ferrix menu itself, and abandons the helper thread (logged; killed at process exit). If the slice 0 spike proves a cross-thread popup unreliable (activation, foreground lock), the fallback is a UI-thread menu with measurement only — and the inability to survive a hung extension becomes a documented gap rather than a silent assumption. The timeout guards menu *appearance*; an open menu is unbounded by design.
5. **In-process isolation ships; the out-of-process helper is a stretch goal gated by the slice 0 spike.** PRD 13.8 makes the auxiliary process conditional ("si l'isolation hors-processus est réalisable"), and PRD 16.3 softens the crash criterion to "autant que techniquement possible" (`PRD.md:963`). In-process, Ferrix persists nothing an extension can damage (undo and activity state are memory-only), so an extension crash cannot *corrupt* the session; it can still terminate it. The spike timeboxes a surrogate design (helper COM local server that loads third-party CLSIDs out-of-proc and marshals menu items and invocations over IPC) and records either a credible implementation estimate or evidence that it does not fit this tranche. Shipping it is a stretch slice-5 add-on, not a blocker.
6. **Thumbnails are consumed from the Shell pipeline, not implemented as an `IThumbnailProvider`.** `PRD.md:961` names `IThumbnailProvider`, which is the interface a *provider* implements to serve thumbnails of its own file types to other applications. Ferrix's acceptance criterion is that *Ferrix's list* shows thumbnails, which the Shell already produces for every registered type: `fx-shell/thumbnails.rs` fetches through `IShellItemImageFactory` (spike-verified against `IThumbnailCache` for quality and speed, decision recorded in slice 0) and stores results in the purgeable L3 cache (`PRD.md:457`). Implementing a provider interface would only matter once Ferrix registers file types (none exist), so it is deferred with the rest of Tranche 12's preview work. **This is an interpretation of `PRD.md:961` and needs the maintainer's explicit sign-off.**
7. **Recycle and restore stay on `SHFileOperationW`.** Tranche 2 validated that path on real Windows, including the long-path fixture and the restore round-trip (run `43042532218`). OPS-08's acceptance criterion is about copy progress (`PRD.md:963`), and re-routing a working, tested delete path inside the same tranche that changes everything else is risk without PRD content behind it. A future consolidation onto `IFileOperation` stays open in section 15.
8. **OLE becomes the single transport for row drags; the Tranche 2 internal drag semantics are reused, not rewritten.** When a row drag starts, `fx-shell/drop_source.rs` runs `DoDragDrop` (a modal pump on the UI thread, which is what Explorer does). Ferrix's own `IDropTarget` receives the self-drop and dispatches it to the existing internal-drop logic, so `Ctrl`/`Shift`/default same-volume rules (`PRD.md:669`, Tranche 2 OPS-06 tests) carry over unchanged; the Slint-internal drag transport is retired in slice 2 with its semantics tests migrated, not deleted. The `Ctrl+Shift` shortcut modifier creates a `.lnk` through `IShellLink` — 13.9 requires the modifier even though the standalone "create shortcut" *command* stays Tranche 7. Clipboard behaviour is untouched: `CF_HDROP` + `Preferred DropEffect` was validated in Tranche 2 and only the drag data object is new.
9. **Tranche 2's operational rules carry over unchanged.** One operation at a time (a native transfer counts as the one), in-memory session-scoped undo, post-condition checks before any success is displayed (rule 13), per-item failures yielding `CompletedWithWarnings`, and the free-space precheck — all preserved on the native path by translating sink events into the existing `OpEvent` pipeline (section 6). The Tranche 2 manual rows stay open in `docs/tranche-2-validation.md`; this tranche neither closes nor depends on them, and no destructive native slice starts before slice 0's findings are recorded.

## 3. New dependencies and the rule 6 review

No new workspace dependency. Everything builds on the existing `windows = "=0.62.2"` crate (workspace features already include `Win32_Foundation`, `Win32_System_Com`, `Win32_System_Ole`, `Win32_UI_Shell`, `Win32_UI_WindowsAndMessaging`, `Win32_Storage_FileSystem`, `Win32_Graphics_Gdi`; `fx-shell` adds `Win32_System_Memory`, `Win32_System_DataExchange`, `Win32_System_Registry`, `Win32_Security`, `Win32_UI_Shell_Common` and others). The APIs this tranche needs — `IContextMenu`, `IFileOperation`, `IShellItemImageFactory`, `SHOpenWithDialog`, `DoDragDrop`, `IShellLink`, the progress sink — live in modules those features already cover.

Any feature that compiling in slice 0 reveals to be missing (candidates: streaming/structured-storage bits for property storage, `Win32_System_Com_StructuredStorage`) is added to `fx-shell`'s feature list in the same change that first uses it, with `cargo check` as the proof. `docs/LEGAL.md` and `THIRD_PARTY_NOTICES.md` do not change: no crate enters the graph, so no license review is triggered. `cargo deny check` and `cargo audit` still run before the first commit, per rule 6's checklist.

No new UI dependency. No new unsafe surface outside `fx-shell` (rule 3): every new COM/Win32 block carries a `// SAFETY:` comment naming its invariant — STA thread owns the interface, ownership of returned handles, the double-null buffer lifetimes, and so on, exactly as the Tranche 2 modules do.

## 4. Module layout

Boundaries from PRD 10.1/10.2 and `CLAUDE.md`: no `unsafe` outside `fx-shell`, `fx-terminal`, `fx-git`, `fx-a11y`; `fx-ops` never depends on `fx-ui` or `fx-shell`; user-visible strings live in `resources/`.

```
crates/fx-ops/src/
  routing.rs        NEW: ConflictScan (destination probe over a plan), TransferRoute,
                     choose_route(plan, NativeCapability) — pure std, fake-able, the
                     policy half of decision 1; port.rs itself is unchanged (the
                     existing apply_metadata is reused as the metadata post-pass)

crates/fx-shell/src/
  context_menu.rs   NEW: IContextMenu merge, Ferrix-section insertion, helper STA
                     thread + watchdog (decision 4), measurement, verb logging  unsafe + SAFETY
  file_operation.rs NEW: IFileOperation on a STA worker, IFileOperationProgressSink,
                     owner window, cancellation, NativeEvent channel            unsafe + SAFETY
  drop_target.rs    NEW: real IDropTarget replacing the lib.rs no-op; effect
                     negotiation from cached paths + modifier keys             unsafe + SAFETY
  drop_source.rs    NEW: drag data object (CF_HDROP + Preferred DropEffect) and
                     DoDragDrop; shortcut modifier via IShellLink               unsafe + SAFETY
  properties.rs     NEW: ShellExecuteEx verb "properties", SHOpenWithDialog
                     (multi-select probe in slice 0)                           unsafe + SAFETY
  thumbnails.rs     NEW: IShellItemImageFactory/IThumbnailCache fetch + disk
                     cache under %LOCALAPPDATA%\FerrixExplorer\cache           unsafe + SAFETY
  lib.rs            re-exports; show_popup_menu stays as the fallback primitive;
                     OleDropRegistration/FileDropTarget move into drop_target.rs

crates/fx-app/src/
  context_menu.rs   NEW: menu command registry (Ferrix section labels ↔ dispatch),
                     measurement/timeout policy, fallback invocation
  drag_drop.rs      NEW: IDropTarget callbacks → orchestration; DoDragDrop entry
                     point for the Slint drag gesture
  properties.rs     NEW: properties and open-with command wiring
  operations.rs     + native runner: spawn the STA worker, translate NativeEvent
                     into the existing OpEvent pipeline, derive undo entries,
                     metadata post-pass, post-condition check (decision 9)
  orchestration.rs  on_context_menu delegates to context_menu.rs; drop dispatch
                     routes through drag_drop.rs (semantics unchanged)

crates/fx-app/tests/
  native_conformance.rs  NEW: same fixture through built-in and native routes —
                     undo-stack equivalence (decision 3), metadata parity, failure
                     parity (locked file), long-path, Unicode, gate fall-back

crates/fx-settings/src/
  view_state.rs     + persisted menu-extension state: blocklist, per-extension
                     enabled flags (file-level management; UI is not in this
                     tranche — section 15)

crates/fx-ui/src/
  viewmodel.rs      + asynchronous thumbnail slots per row (generation-guarded)
  ui.slint          + row thumbnail rendering with the existing icon as fallback
  (dialogs.slint, activity.slint unchanged: the native dialog is the Shell's)

resources/en-US.rs, resources/en-US.slint
                    + Ferrix menu labels (copy path, copy name, properties,
                    open with), fallback-menu status message, announcements

crates/fx-shell/examples/
  spike_menu.rs, spike_fileop.rs, spike_thumbnails.rs, spike_openwith.rs
                    throwaway slice-0 probes (PRD.md:1078 "prototypes isolés dans
                    fx-shell"); not library code, not shipped, findings recorded
                    in docs/tranche-3-validation.md
```

`fx-ops` depends only on `fx-domain` as before. `fx-shell` keeps no dependency on `fx-ops`; the adapter that joins them lives in `fx-app`, exactly like the Tranche 1-2 ports. The `fx-ops` port trait gains **no new method**: routing needs no capability call beyond a boolean `NativeCapability` that `fx-app` probes once by co-creating `IFileOperation`, and the metadata post-pass reuses `apply_metadata`.

## 5. Data model

```rust
// fx-ops/routing.rs
pub struct NativeCapability { pub file_operation: bool }   // probed once by fx-app

pub struct ConflictScan {
    pub existing_destinations: Vec<PathBuf>,   // planned entries whose destination exists
    pub reparse_points: bool,                  // any reparse point in the plan
}

pub enum TransferRoute { Native, BuiltIn }

pub fn scan_destination(plan: &OperationPlan) -> io::Result<ConflictScan>;
pub fn choose_route(scan: &ConflictScan, kind: OperationKind,
                    native: NativeCapability) -> TransferRoute;
// Native only for Copy|Move with an empty scan and native.file_operation;
// every other combination is BuiltIn — total function, unit-tested as a matrix.

// fx-shell/context_menu.rs
pub struct MenuQuery {
    pub owner: HWND,
    pub cursor: (i32, i32),
    pub selection: Vec<PathBuf>,     // empty = folder background
    pub background: Option<PathBuf>,
}
pub enum MenuOutcome {
    Shell { verb: String },          // canonical verb from GetCommandString, for logging
    Ferrix { command: FerrixCommand },
    Cancelled,
}
pub struct MenuTimings { pub query: Duration, pub visible: Duration }

pub fn show_native_menu(query: MenuQuery, timeout: Duration)
    -> Result<(MenuOutcome, MenuTimings), MenuError>;
// MenuError::Timeout | MenuError::Hwnd | MenuError::Shell(code) — never a panic.

// fx-shell/file_operation.rs
pub struct NativeTransferRequest {
    pub kind: NativeKind,            // Copy | Move
    pub sources: Vec<PathBuf>,
    pub destination: PathBuf,
    pub owner: Option<HWND>,         // SetOwnerWindow: the native dialog parents to Ferrix
}
pub enum NativeEvent {
    Started,
    Progress { files_done: u64, bytes_done: u64, current: Option<PathBuf> },
    ItemDone { source: PathBuf, destination: PathBuf },   // pairs feed undo (decision 3)
    ItemFailed { path: PathBuf, code: HRESULT },
    Finished { outcome: NativeOutcome },                  // Completed | CompletedWithWarnings |
                                                          // Failed | Cancelled
}
pub fn run_native_transfer(request: NativeTransferRequest,
                           events: Sender<NativeEvent>,
                           cancel: CancellationToken) -> NativeHandle;

// fx-shell/drop_source.rs and drop_target.rs
pub enum DragEffect { Copy, Move, Shortcut }              // maps to DROPEFFECT_*
pub struct DragPayload { pub paths: Vec<PathBuf>, pub effect: DragEffect }
pub enum DropOutcome { Executed { effect: DragEffect }, Cancelled, Rejected }

// fx-shell/thumbnails.rs
pub struct ThumbnailKey { pub path: PathBuf, pub size: u32, pub mtime: SystemTime }
pub fn fetch_thumbnail(key: &ThumbnailKey) -> Option<RgbaImage>;  // cache → shell → None
```

Notes the types carry deliberately:

- **`ItemDone` is a pair, not a single path.** Undo of a native move needs both endpoints, and a rename-by-shell (keep-both) would break a naive pairing; the gate from decision 1 keeps the native path conflict-free, so the pairing invariant holds — the conformance test asserts it.
- **`NativeOutcome` reuses `OperationState`** rather than inventing a parallel enum, so the activity center, announcements and error lists downstream are untouched.
- **The timeout is a parameter, not a global.** Tests pass a fake budget; the product value lives in one `const` next to the menu code, tunable when slice 0 measurements speak.
- **Thumbnail keys include size and mtime**, so an edited file misses the cache naturally; entries are written atomically (temp file + rename) and evicted oldest-first by file time against a size cap, which is what "purgeable, reconstruction possible" (`PRD.md:457`) asks for.

## 6. Threading, cancellation, pause

```
Slint UI thread (HWND)     menu helper STA thread      transfer STA worker      UI event loop
----------------------     ----------------------      -------------------      ------------
right-click / Shift+F10 ->  QueryContextMenu (timed)        |                       |
   |                       TrackPopupMenuEx + pump         |                       |
   | watchdog: no menu in 2 s? --> WARN log, UI thread     |                       |
   | shows minimal Ferrix menu, abandons helper            |                       |
   |                       <--- verb / command channel --> |                       |
dispatch Ferrix command -->                               |                       |
                                                          IFileOperation + sink --+
Ctrl+V / drop (route=Native) ----------------------------> |  NativeEvent (bounded)
   |                                                        |  --> OpEvent pipeline
   +-- CancellationToken ----------------------------------> |  --> activity row, undo,
                                                             |     refresh, announce
```

- **Menus and drags run on UI-affine threads; transfers never block the Slint thread** (rule 4). `DoDragDrop` is a modal pump started from the UI thread — the same shape Explorer uses — and the actual copy it triggers still executes in the operation runner.
- **The transfer worker is a dedicated STA thread with an explicit, RAII-scoped `CoInitializeEx`** (PRD 10.5 discipline, same as the Tranche 2 restore path). The sink is implemented with `windows_implement`; every callback translates to a channel message and returns immediately.
- **Cancellation**: the token is observed by the sink — an abort requested from callbacks returns a failure HRESULT out of the sink call, which stops `IFileOperation` (exact mechanism verified in the slice 0 spike; if the documented abort path proves unreliable, cancellation degrades to "not cancellable mid-native-op" and the activity center hides the button for that row rather than showing a lie).
- **Pause is available on the built-in route only.** `IFileOperation` has no pause. PRD 13.4 asks for native progress "lorsque c'est possible" (`PRD.md:633`), and the native dialog is precisely the thing this tranche must ship, so the native row offers cancel and retry but not pause — recorded as a deviation in section 14, not hidden.
- **Events keep the Tranche 2 disciplines**: bounded channel with `blocking_send`, coalescing of `Progress` to about 10 Hz, terminal events (`ItemFailed`, `Finished`) never dropped, late events discarded by operation id on the UI side. `NativeEvent` → `OpEvent` translation lives in `fx-app/operations.rs`, so no consumer of the activity pipeline changes.
- **The menu helper is abandoned, not killed.** A stuck third-party call keeps one thread and its COM state until process exit; the log names the duration and the item count so the blocklist (section 8) can be used. Terminating a thread mid-COM is rejected as unsafe.

## 7. Shell surface

All of it lives in `fx-shell`, each block with a `// SAFETY:` comment, each call validated on real Windows before it is claimed (rule 8). Slice 0 measures; slices 1-5 implement.

**Context menu (`context_menu.rs`).** `IShellFolder::BindToObject`/desktop binding to obtain the `IContextMenu` for the selection (or the folder background), `QueryContextMenu` to populate an `HMENU`, insertion of the Ferrix section by index with a separator (command ids reserved outside the shell's `idCmdFirst..idCmdLast` range so a translation can never collide with a Shell verb), then `TrackPopupMenuEx`. On selection: shell ids resolve through `GetCommandString` to a canonical verb for logging and `InvokeCommand` executes; Ferrix ids dispatch through `fx-app/context_menu.rs`'s registry. Keyboard support needs no extra work beyond what `TrackPopupMenuEx` gives (`Menu`/`Shift+F10` already reach `ShortcutAction::ContextMenu`, `orchestration.rs:941`). Measurement wraps `QueryContextMenu` and the visible interval; both are logged through `tracing` at `info` with item counts (`PRD.md:663` journalisation technique).

**Transfer (`file_operation.rs`).** `CoCreateInstance::<IFileOperation>`, `SetOwnerWindow`, `SetOperationFlags` (no confirmation UI beyond what decision 1's gate allows, `FOF_ADDUNDORECORD` off, error UI surfaced per item rather than modal-per-file), `CopyItem`/`MoveItem` per planned entry, `ApplyPropertiesToItem` skipped (metadata post-pass instead, decision 9), then `PerformOperations` on the worker with the sink attached. Long paths: the spike runs the >260-character fixture and records the result — `IFileOperation` takes paths where `SHFileOperationW` refused them, which is the improvement the Tranche 2 spec flagged as future Tranche 3 work. Metadata parity (Tranche 2 rule 7: modification time, creation time, attributes) is asserted by the conformance test; if the shell drops creation time, the existing `PlatformOps::apply_metadata` runs as a post-pass per completed file — the seam exists for exactly this.

**Drag target (`drop_target.rs`).** The `lib.rs` no-op `FileDropTarget` becomes real: `DragEnter` reads `CF_HDROP` once and caches the paths plus the `Preferred DropEffect`; `DragOver` computes the effect from the cached payload and live modifier keys (`GetAsyncKeyState` for `Ctrl`/`Shift`/`Ctrl+Shift`, decision 8) against the same default rule the Tranche 2 engine uses; `DragLeave` clears the cache; `Drop` snapshots paths and hands them to `fx-app/drag_drop.rs`, which runs the existing paste/move command path — no file I/O on the UI thread (rule 4). Rejected drops return `DROPEFFECT_NONE` with the standard "no" cursor rather than failing silently.

**Drag source (`drop_source.rs`).** A minimal `IDataObject` exposing `CF_HDROP` and `Preferred DropEffect` (built from the same `DROPFILES` layout the clipboard already writes — `clipboard.rs` stays untouched), `IDropSource` reporting `DROPEFFECT_NONE` on invalid modifiers, and `DoDragDrop` until drop or escape. Self-drops arrive at our own target and reuse the internal-drop semantics (decision 8). The shortcut modifier builds a `.lnk` at the drop destination through `IShellLink` + `IPersistFile`.

**Properties and Open with (`properties.rs`).** `ShellExecuteExW` with `SEE_MASK_INVOKEIDLIST` and the `properties` verb for the selection — the API Annex A names (`PRD.md:1321`). Multi-select properties needs the spike's answer: Explorer shows one combined sheet (total size etc.) and the single-path verb does not obviously extend to arrays; candidates are probed in slice 0 and the finding decides whether multi-select opens one sheet, opens per item, or documents a gap. "Open with" is `SHOpenWithDialogW` with an `OPENASINFO` struct; both calls take the owner `HWND` so the sheet parents to Ferrix.

**Thumbnails (`thumbnails.rs`).** Per visible image-likely row: build the shell item from the path, request an image at the list's icon size through `IShellItemImageFactory::GetImage` (spike compares against `IThumbnailCache::GetThumbnail` for quality, cost and thumbnail-type coverage), check the disk cache first, write on success. Fetches run on a small bounded worker pool — never on the Slint thread, never blocking a listing; the view model carries a generation guard so a scrolled-away row's late result is discarded, and a miss or failure leaves the existing icon from `icons.rs` in place. Only the fetch path is new: Ferrix registers no handlers and no `IThumbnailProvider` (decision 6).

**Fallback primitive (`lib.rs`).** `show_popup_menu` (multi-item, already used by `on_context_menu`, `orchestration.rs:597`) is what the minimal Ferrix menu uses when the native menu fails, times out or is disabled — carrying the same command registry, so the fallback is functional, not a stub.

## 8. Isolation, timeout, blocklist, logging, fallback (PRD 13.8)

The seven architectural bullets of `PRD.md:663`, one by one:

1. **Measure opening time.** `MenuTimings` is always collected and logged: `tracing::info!(target: "menu", query_ms, visible_ms, items, source = "shell" | "ferrix_fallback")`. Measurement exists even when the helper-thread design is unavailable, so the Win10/Win11 parity statement (`PRD.md:665`) rests on numbers, not impressions.
2. **Timeout.** One `const MENU_TIMEOUT` (proposed 2 s) guarding menu *appearance*, enforced by the UI-thread watchdog per decision 4. Expiry = log + minimal Ferrix menu + abandon helper. No timeout ever silently picks a command.
3. **Technical logging.** Verb names from `GetCommandString`, item counts, durations, every `MenuError`, every fallback — all through the existing `tracing` pipeline (`PRD.md:847` rules apply: no file contents, paths handled by the existing conventions).
4. **Local blocklist.** `fx-settings` persists a list of disabled extension identities (CLSID or ProgID string as `GetCommandString` reveals them). Before showing the menu, blocked extensions are excluded from the query where the Shell lets us (the blocklist primarily informs the post-hoc diagnosis loop: hang → log → user adds identity → next run skips it). File-level storage; a management UI is not in this tranche (section 15).
5. **Disable an extension.** Same storage, a per-identity enabled flag, so one extension can be turned off without blocking the rest. Both mechanisms share the settings fields added in section 4.
6. **Out-of-process auxiliary helper, if feasible.** Decision 5: spike-timeboxed surrogate design, evidence recorded, stretch goal otherwise. The in-process limitations are stated, not hidden: a hard hang cannot be interrupted (decision 4 mitigates by abandoning), a hard crash terminates Ferrix, and per-extension attribution of a hang is impossible in-process — the blocklist is therefore a diagnostic loop, not an automatic guard.
7. **Fallback to a minimal Ferrix menu.** Always available, always the same command registry, reachable on any `MenuError`, on timeout, and by disabling the native menu wholesale (a settings escape hatch that also serves development machines without extensions).

Parity (`PRD.md:665`) is validated, not claimed: the slice 1 checklist runs right-click on a file, a multi-selection and the folder background against Explorer with 7-Zip and Git installed, on Windows 11 and Windows 10 22H2, and records where they differ.

## 9. Activity center and undo integration

- A native transfer is one operation row, exactly like a built-in one: kind, targets summary, state, per-file progress from the sink, current file, errors, cancel, retry. Pause appears only when the route can honour it (decision 9, section 6 deviation).
- `ItemFailed` becomes `ItemFailure`; `Finished` runs the same post-condition discipline as the built-in route (rule 13): the operation reports `Completed` only when the sink's completed count matches the plan and a final stat check of the planned top-level destinations passes — otherwise `CompletedWithWarnings` or `Failed`. No false success on either route.
- Undo entries are derived from `ItemDone` pairs (decision 3); `Ctrl+Z` after a native move performs the Tranche 2 move-back with its identity checks. Copy pushes nothing. The conformance test is the contract.
- After `Finished`, the existing refresh path runs (explicit refresh plus the `notify` watcher), so Explorer-side changes made by the Shell's own copy are picked up like any other.
- One operation at a time: starting a native transfer while anything else runs queues through the same gate as the built-in route — no second native dialog beside a running one.

## 10. Error handling

`MenuError` and `NativeError` are typed and total:

```rust
pub enum MenuError { Timeout, HwndUnavailable, Hwnd(u32), QueryFailed(HRESULT), Abandoned }
pub enum NativeError { Unsupported(reason), CoCreateFailed(HRESULT), Aborted,
                       OperationFailed(HRESULT), WorkerLost }
```

The fallback ladder is mechanical: native route unavailable at start → the built-in engine runs instead (transparent, logged once — PRD `PRD.md:112`); native route fails mid-operation → the failure lands in the activity center and Retry re-runs through `choose_route` again, which will pick the built-in engine if the scan or capability now says so. Menu errors never leave the user without a menu. Nothing panics on a user path, nothing is swallowed: `tracing` records it, the activity center lists it, the status region announces it — the Tranche 2 rule stands.

## 11. User-visible strings

Every new string goes into `resources/en-US.rs` (menu labels: copy path, copy name, properties, open with; the fallback status message; announcements for native-route failures) and `resources/en-US.slint` if a dialog needs one, per `CLAUDE.md` and PRD 859. No literal user-visible text in UI code. The existing resource/native-title consistency test is extended to cover anything new. English only, `fr-FR` layout unchanged.

Shell-provided strings (extension verbs, the native dialog, the properties sheet) are the Shell's, not ours — they are deliberately not externalized.

## 12. Testing

- **`fx-ops` unit tests** (pure, no Windows): `choose_route` as a full matrix (kind × scan emptiness × reparse × capability), `scan_destination` over `tempfile` fixtures (file conflict, directory-tree conflict, no conflict, reparse flagged), routing never selects native for delete/rename/checksum kinds, and the existing Tranche 2 suites stay green untouched.
- **`fx-app` integration tests, Windows-labelled** (`native_conformance.rs`): the same fixture through both routes asserting — undo stacks equivalent (decision 3); metadata parity (mtime, creation time, attributes) including the post-pass branch; failure parity (a locked file yields one `ItemFailure` and `CompletedWithWarnings` on both routes); long-path fixture >260 chars; Unicode names; move post-condition (source gone) and copy post-condition (destination present with length); gate fall-back when the destination already exists.
- **`fx-shell`**: no COM unit tests (PRD 15.3, `PRD.md:880-882`: COM is hard to simulate — documented manual tests). `docs/tranche-3-validation.md` carries the checklist: 7-Zip and Git entries present on right-click for file/multi/background; `Menu` and `Shift+F10` open the same menu and every entry is reachable by keyboard; a disabled extension disappears; the timeout path shows the minimal Ferrix menu (forced by a zero budget in a debug build); drag Ferrix→Desktop, Ferrix→Explorer, Explorer→Ferrix, Explorer→Ferrix multi-select, with `Ctrl`/`Shift`/`Ctrl+Shift` and the default rule; a 2 GB copy shows the native progress dialog and its cancel works; properties sheet single- and multi-select; Open with; thumbnails appear in the list and the second visit is served from the cache (log proof); native long-path copy; metadata spot-check; Narrator/UIA pass over the menu and the activity rows (`PRD.md:774-776`, a11y in *every* UI tranche).
- **Robustness (PRD 15.4, `PRD.md:884-886`)**: a hung extension (real one if available, otherwise a test extension registered only on the validation machine) exercising the timeout and abandon path; disk full during a native copy (destination volume sized for the test) → per-item failures, no false success; access-denied destination → typed failure in the activity center; cancel mid-native-copy → `Cancelled`, partial output removed or reported per the sink's actual behaviour (spike records which); source deleted between gate and execution → race case of decision 2 behaves as documented.
- **`fx-ui`**: view-model tests for thumbnail slots (generation guard discards stale results) and menu-command registry mapping; UIA assertions for any new accessible names.
- **Regression**: Tranches 0-2 stay green, including the Tranche 2 transfer and conflict suites — the internal-drag semantics tests migrate with decision 8 rather than being deleted. No assertion weakened, no lint suppressed; if a gate fails, the cause is fixed.
- Every slice ends with `cargo fmt --all -- --check`, `cargo check --workspace --all-targets`, `cargo test --workspace --all-targets`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo deny check`, `cargo audit`.

## 13. Delivery slices

Each slice compiles, is tested, and is reviewable on its own. Branch: `tranche-3-native-shell`, one branch for the tranche (`CLAUDE.md` rule 1), stacked on `tranche-2-file-operations` until that merges.

0. **Spikes isolés (PRD `PRD.md:1078`)** — four `fx-shell/examples/` probes plus a memo: (a) menu merge on the UI thread and on the helper STA thread, with timings, verb logging and the 7-Zip/Git reality check; (b) `IFileOperation` + sink + owner window + cancel mechanism + metadata parity + long path on fixtures; (c) `SHOpenWithDialog` and the multi-select properties API question; (d) thumbnail fetch API comparison; (e) timeboxed out-of-process surrogate feasibility note (decision 5). Gate: findings recorded in `docs/tranche-3-validation.md`, decisions 1-8 confirmed or amended in an addendum to this document. No product code.
1. **Menu natif** — `routing.rs` (scan + gate, tested without COM), `context_menu.rs`, `fx-app/context_menu.rs`, settings fields (blocklist/disable), fallback path wired to the existing command registry, strings, measurement/logging. Gate: `fx-ops` route matrix green; manual menu checklist half (entries, keyboard, fallback) signed off on Windows 11.
2. **DnD OLE** — `drop_target.rs`, `drop_source.rs`, `fx-app/drag_drop.rs`, shortcut modifier, retirement of the Slint-internal transport with its semantics tests migrated, strings for drop-related messages. Gate: manual both-directions checklist including modifiers; internal-drag regression suite green.
3. **Propriétés et Ouvrir avec** — `properties.rs`, `fx-app/properties.rs`, multi-select decision from the spike, strings. Gate: manual sheet/open-with checks; no new unsafe outside `fx-shell`.
4. **Miniatures** — `thumbnails.rs` with disk cache and eviction, view-model slots, `ui.slint` rendering, async discipline tests. Gate: cache hit proven by log; listing performance not regressed (spot-checked, 60 FPS claim still open in section 15); icon fallback exercised.
5. **IFileOperation et conformance** — `file_operation.rs`, native runner in `operations.rs`, undo derivation, metadata post-pass, `native_conformance.rs`, activity-center integration (pause visibility by route), robustness tests, full `docs/tranche-3-validation.md` sign-off including the 2 GB native-dialog acceptance criterion, README/CLAUDE status updates. Gate: conformance suite green on the runner (CI, pwsh — the Tranche 2 lesson) and locally; manual checklist complete on Windows 11; Windows 10 22H2 rows executed or explicitly recorded as open.

Slices 0-1 contain no data-destroying path (routing exists but nothing calls the native route yet); slice 5 ships the destructive path last, behind the conformance suite.

## 14. PRD conformance audit

| PRD source | Requirement | Status in this spec |
|---|---|---|
| 961 | native `IContextMenu` merge + Ferrix menu | **Covered** — SYS-03/04, sections 7-8, slice 1 |
| 961 | `IFileOperation` with native progress dialog | **Covered** — decision 1 gate, section 7, slice 5 |
| 961 | native properties | **Covered** — OPS-14 `ShellExecuteEx` verb, slice 3 |
| 961 | "Open with" | **Covered** — OPS-15 `SHOpenWithDialog`, slice 3 |
| 961 | `IThumbnailProvider` thumbnails + disk cache | **Interpretation, decision 6** — Shell-pipeline consumption + L3 cache; provider implementation deferred; needs sign-off |
| 961 | OLE drag-and-drop both directions | **Covered** — OPS-07, decisions 8, slice 2 |
| 961 | opening-time measurement + fallback | **Covered** — `MenuTimings`, section 8, slice 1 |
| 961 | keyboard menu (`Maj+F10`, `PRD.md:792`) | **Covered** — existing `ShortcutAction::ContextMenu`, slice 1 |
| 963 | right-click shows Explorer's entries (7-Zip, Git) | **Validated manually** — section 12 checklist, both OSes |
| 963 | drag to Desktop/Explorer copied correctly, and reverse | **Validated manually** — slice 2 checklist |
| 963 | 2 GB file shows the native progress dialog | **Validated manually** — slice 5 checklist + runner |
| 963 | menu usable by keyboard | **Covered** — `TrackPopupMenuEx` + checklist |
| 963 | extension crash does not corrupt the session "as far as technically possible" | **Covered within limits** — decision 5: memory-only state, blocklist, timeout, fallback; out-of-process is the stretch; residual process-death gap stated in section 15 |
| 661 | file / multi / background menus, Shell verbs, submenus, extensions, Ferrix commands, `Menu` + `Shift+F10` | **Covered** — sections 7-8, slice 1 |
| 663 | measure, timeout, logging, blocklist, disable, aux process if feasible, fallback | **Covered** — section 8, decisions 4-5; aux process spike-gated |
| 665 | parity not guaranteed; same entries as Explorer for common extensions | **Validated, not claimed** — checklist records differences |
| 633 | operations via `IFileOperation` when possible + native dialog | **Covered** — decision 1; route is the "when possible" |
| 635 | conflict UX: replace, skip, keep both, compare metadata, apply-to-all, cancel | **Covered, with decision 2's race deviation** — conflicts route to the Tranche 2 dialog; only a post-gate race sees the native subset |
| 637 | undo honesty | **Covered** — decision 3 + conformance test; no new undo claims |
| 639 | OPS-18 integrity | **Not this tranche** — backlog assigns it to 12 (`PRD.md:1325`); sink pairs keep it possible |
| 669 | Ferrix↔Explorer, OLE apps, modifiers incl. shortcut | **Covered** — decision 8, slice 2; sidebar/tab targets deferred to T4/5 as the owning tranches |
| 671 | keyboard alternative for every essential action | **Already met** — Tranche 2 cut/paste; menu path re-checked in slice 1 |
| 758 | rich property fields (checksum, EXIF, streams…) | **Not this tranche** — PRV-09, tranche 12 (`PRD.md:1345`); T3 opens the native sheet |
| 760 | ACL operations | **Deferred v2** — SYS-08 |
| 774-776 | UIA, Narrator, keyboard, a11y tests in every UI tranche | **Covered in design; manual Narrator/Accessibility Insights half stays open** |
| 112 | fallback when a native API is unavailable or unstable | **Covered** — section 10 ladder |
| 1078 | dedicated time, isolated prototypes in `fx-shell`, documented manual tests | **Covered** — slice 0 examples, section 12 |
| 1080 | `IFileOperation`, robustness tests (15.4) | **Covered** — sections 7 and 12 |
| 1314 / 1315 / 1321 / 1322 / 1403 / 1404 | OPS-07, OPS-08, OPS-14, OPS-15, SYS-03, SYS-04 | **Covered** — slices 1, 3, 5 |
| 10.2:378 / CLAUDE.md | `fx-ops` free of `fx-shell`/`fx-ui`; unsafe confined | **Covered** — section 4 |
| 15.3:880 | COM tests are documented manual tests | **Covered** — section 12 |
| 10.9:459 | SQLite operation state | **Still not this tranche** — Tranche 12/13, unchanged from Tranche 2 decision 5 |

## 15. What this design does not resolve

- **Out-of-process isolation** ships only if the slice 0 spike makes it credible (decision 5); otherwise the residual in-process limits — uninterruptible hang, process-death on crash, no per-extension attribution — are recorded as evidence-backed gaps, satisfying `PRD.md:963`'s "as far as technically possible" but not more.
- **Multi-select properties** depends on the spike finding (slice 0c); until then the sheet behaviour for selections of size > 1 is an open question, not an assumption.
- **Metadata parity under `IFileOperation`** (especially creation time) is measured, not assumed; the post-pass exists but only runs if the conformance test proves it necessary.
- **Windows 10 22H2 behaviours** for the menu, dialog and thumbnails are unvalidated until the manual rows run; nothing in this document claims them (rule 8).
- **A conflict racing past the gate** sees the native three-choice dialog (decision 2) — a documented, rare deviation from OPS-09's four choices.
- **Pause on the native route** does not exist; native rows offer cancel and retry only (section 6 deviation).
- **Recycle/restore remain on `SHFileOperationW`** (decision 7); unifying them onto `IFileOperation` is deferred, as is anything that would re-open the Tranche 2 long-path and restore validation.
- **Thumbnail provider registration** (`IThumbnailProvider` as a *provider*) waits for Ferrix file types that do not exist yet; this is decision 6's interpretation and is revisited if the maintainer reads `PRD.md:961` differently.
- **Extension management UI** is out: blocklist and disable are file-level settings (section 8); a settings surface is later UI work.
- **Sidebar and tab drop targets** belong to Tranche 4/5; OLE lands on rows, the current folder, the background and external applications only.
- **Tranche 2's manual validation rows stay open** (`docs/tranche-2-validation.md`); this tranche neither closes nor blocks on them.
- **60 FPS under menu-heavy interaction** remains unmeasured — the same open instrumentation gap Tranche 2 recorded.
- **A hung extension's blame** cannot be assigned in-process; the blocklist is a manual diagnostic loop until the helper process exists.
