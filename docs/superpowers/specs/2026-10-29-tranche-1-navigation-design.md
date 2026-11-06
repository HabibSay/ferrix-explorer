# Tranche 1 — Basic navigation: design

- **Date:** 2026-10-29
- **Status:** Draft for maintainer review, revised after a PRD conformance audit. No code is written against this document until it is approved.
- **Scope source:** `PRD.en.md` section 16 Tranche 1 (lines 941-947) and, as the normative detail, section 13 — `PRD.en.md:593` states that in a conflict section 13 prevails over Annex A. Relevant subsections: 13.1 navigation, 13.2 views, 13.3 selection, 13.21 accessibility, 13.22 keyboard shortcuts.
- **Depends on:** ADR 0001 as accepted 2026-10-29 (Slint selected on Windows 11 measurements only).

## 1. Scope

Tranche 1 delivers read-only browsing of the local filesystem: open a folder, see its entries in a virtualized list or a details view, sort and filter them, navigate by path, breadcrumb, history, parent, or refresh, and have all of it usable and legible without a mouse.

In scope, from PRD 13.1, 13.2, 13.3, 13.21, 13.22 and Annex A Tranche 1 rows:

- NAV-01 asynchronous listing with `jwalk`, streamed batches, cancellation.
- NAV-02 list and details views with name, size, date, and type columns; columns resizable and reorderable; view mode remembered per folder.
- NAV-04 breadcrumb showing every segment, clickable, **with a segment's subfolder dropdown**.
- NAV-05 editable address bar with autocomplete, replacing the breadcrumb temporarily.
- NAV-06 history: back, forward, up to parent, refresh; per-pane history storing scroll position and selection where possible; `Alt+←`, `Alt+→`, `Alt+↑`, `F5`, and the mouse back/forward buttons.
- NAV-08 instant filtering.
- NAV-09 multi-criteria sorting (grouping itself is Tranche 9).
- NAV-11 list virtualization: only visible entries are created.
- NAV-12 long paths and Unicode.
- System icons via `SHGetFileInfo`, with the timeout and fallback discipline of PRD 10.5.
- Showing or hiding hidden and system files, and showing or hiding extensions (PRD 13.2).
- Single, `Ctrl+click`, `Shift+click`, and `Ctrl+A` selection, surviving a refresh (PRD 13.3).
- Accessibility per PRD 13.21: names, roles, and states on rows and columns; folder-change announcement; selected-count announcement; visible focus; coherent tab order; everything reachable by keyboard.

Out of scope, because the PRD assigns it elsewhere:

- Anything that writes to disk: copy, move, rename, delete, and the rest of OPS (Tranche 2).
- Native `IContextMenu` merging with Shell extensions and `IFileOperation` (Tranche 3). The Tranche 0 app-local "About Ferrix" menu stays as it is.
- Rectangular selection, selection by pattern, inverting selection (Tranche 2).
- Sidebar and tree (Tranche 4); tabs, multi-window, split and dual-pane (Tranche 5, NAV-13 to NAV-16); icon, tile, and content views, `Ctrl+wheel` zoom, grouping, density (Tranche 9, NAV-03, NAV-10, NAV-09).
- Search, index, USN/MFT (Tranche 6); command palette, terminal, CLI (Tranche 11-13).
- Listing over UNC, mapped drives, WSL, archives, or cloud. `Location` carries those variants so the abstraction is honest, and each returns a typed `UnsupportedLocation`.

## 2. Decisions taken before this document

Both were put to the maintainer on 2026-10-29 and answered:

1. **Slint owns the event loop.** `fx-app` stops hand-writing `GetMessageW`/`DispatchMessageW`. It builds the Slint window and calls `slint::Window::run()`. The native HWND comes from `slint::Window::window_handle()`, which implements `raw_window_handle_06::HasWindowHandle` behind Slint's `raw-window-handle-06` feature — verified against the extracted `i-slint-core-1.18.1/api.rs:813`. `OleDropRegistration` and the `WM_CONTEXTMENU` handler attach to that HWND, so Tranche 0 validation carries forward instead of being discarded.
2. **The runtime follows PRD 10.3 and 10.4 literally**: `tokio` for orchestration, `tokio_util::sync::CancellationToken`, `jwalk` for traversal, `notify` for watching. `rayon` and `crossbeam` arrive transitively through `jwalk` and are not declared directly.

## 3. New dependencies and the rule 6 review

| Crate | Version pinned | License | Declared MSRV | Maintenance signal | Native code | Notes |
|---|---|---|---|---|---|---|
| `jwalk` | `=0.9.0` | MIT | none declared | released 2026-08-05, 120k downloads | no | Parallel walker; pulls `rayon ^1.5` and `crossbeam ^0.8`. |
| `notify` | `=8.2.1` | CC0-1.0 | 1.85 | released 2026-10-25 | no | 9.0.0 is still a release candidate (rc.7); the stable line is used. |
| `tokio` | `=1.54.1` | MIT | 1.85 | released 2026-10-21, 6.7M downloads | no | Features limited to `rt-multi-thread`. |
| `tokio-util` | `=0.7.20` | MIT | 1.85 | released 2026-10-09, 13.3M downloads | no | Feature `sync` only, for `CancellationToken`. |

Three of the four declare an MSRV of 1.85, at or below the pinned toolchain 1.92; `jwalk` declares none, so its compatibility is established by compiling rather than by reading metadata. Every version is pinned with `=` to satisfy `wildcards = "deny"` in `deny.toml`, and all four licenses are already in the `allow` list. `docs/LEGAL.md` and `THIRD_PARTY_NOTICES.md` gain rows, and the `deny.toml` Slint exceptions are re-labelled from "spike only" to a product dependency, as ADR 0001 now requires.

## 4. Module layout

Boundaries follow PRD 10.1 and 10.2; no `unsafe` leaves the four interop crates.

```
crates/fx-domain/src/
  lib.rs            re-exports
  entry.rs          FileEntry, EntryId, EntryKind, FileAttributes
  location.rs       Location (Local, Unc, Wsl, Search, Archive, Cloud, Virtual)
  view.rs           ViewMode { List, Details }, Column, ColumnLayout, SortKey, SortDirection
  filter.rs         FilterQuery, VisibilityRules (hidden, system, extensions)
  history.rs        NavigationHistory, NavigationState (location, scroll anchor, selection)
  keys.rs           the Tranche 1 shortcut map, as data, so it is testable and reusable by the palette later

crates/fx-fs/src/
  lib.rs            re-exports
  listing.rs        list_directory(): streams FileEntry batches, takes CancellationToken + generation
  watch.rs          watch_location(): notify watcher behind a trait, so UI tests can fake it
  longpath.rs       verbatim-path helpers for \\?\ (behaviour measured, see section 8)

crates/fx-shell/src/
  icons.rs          SHGetFileInfo-backed system icons, unsafe + SAFETY comments here only
  execute.rs        ShellExecute wrapper for double-click (slice 3)

crates/fx-ui/src/
  lib.rs            re-exports
  ui.slint          ApplicationWindow, FileList, DetailsTable, AddressBar, Breadcrumb, SubfolderMenu
  viewmodel.rs      the Slint-facing view model: rows, selection, sort, filter, column layout

crates/fx-settings/src/
  view_state.rs     remembered view mode and column layout per folder, alongside the existing window.ini store

crates/fx-app/src/
  main.rs           Slint window construction and run
  orchestration.rs  navigation commands, listing tasks, watchers, cancellation
```

`fx-domain`, `fx-fs`, and `fx-ops` never depend on `fx-ui` or on Slint (PRD 10.2). `fx-ui` depends on `fx-domain` and reaches `fx-fs` only through ports that `fx-app` hands it, so the view model is testable without a window.

## 5. Data model

`FileEntry` follows PRD 10.7 minus the fields whose producers do not exist yet: `git_state`, `tags`, `thumbnail_key`, and `cloud_state` are **not declared**, because a field with no producer is dead code that later tranches will have to audit. They are listed here as deliberate omissions rather than silently dropped. `id` is a `u64` per listing, `path` a `PathBuf`, `display_name` a `String`, `kind` an `EntryKind` of `File | Directory | Symlink | ReparsePoint`, `size` and `modified` `Option`s, `attributes` a bitflags struct, `icon_key` an opaque `u64` cache key filled by `fx-shell`.

`Location` is the seam that keeps UNC, WSL, archive, and cloud out of listing code permanently: `list_directory()` takes `&Location` and answers `ListingError::UnsupportedLocation(kind)` for anything unreadable yet, so a later tranche adds a provider instead of editing a match arm in the UI. PRD 13.1 requires the address bar to *accept* UNC, URI, and WSL forms, so parsing recognises them and the error text says "this location type is not supported yet" rather than "invalid path".

## 6. Threading, cancellation, and the generation guard

The UI thread never touches the filesystem (PRD 10.3, `CLAUDE.md` rule 4).

```
UI thread            tokio blocking pool              Slint event loop
---------            -------------------              ----------------
navigate(loc)  ----> spawn_listing(job)  --jwalk-->  receiver.on_listing_batch()
   |                     |                                |
   |                 bounded mpsc channel                 v
   |              (512 entries per batch)            rows updated, scroll kept
   |
   +-- new token + generation id per navigation
```

- Each navigation creates a `CancellationToken` and a monotonic `generation: u64`. A batch with a stale generation is dropped at the receiver: PRD 10.4's "reject stale results".
- Batches go through a **bounded** channel, so a UI that cannot keep up blocks the walker instead of buffering 50,000 entries: PRD 10.4's backpressure. "Priority to visible tasks" is met by cancelling the previous listing on every navigation rather than by a priority queue, which Tranche 1 does not need.
- Slint state is mutated only from the event loop, through `slint::Window::invoke_from_event_loop`.
- Navigating cancels the previous token, drops the previous watcher, and clears the model before the first batch of the new location arrives, so two folders never mix.
- `notify` events are coalesced: a burst inside 200 ms triggers one rescan of the affected directory. A `git checkout` or an unzip emits hundreds of events.
- Icon handles and `SHGetFileInfo` results live in a **bounded LRU** (PRD 10.8 L1), sized in the plan, with `DestroyIcon` on eviction. An unbounded icon cache is a leak with a pleasant first hour.

## 7. Accessibility, treated as a Tranche 1 requirement

PRD 13.21 requires accessible names, roles, **and states**, folder-change announcements, selected-count announcements, visible focus, coherent tab order, full keyboard reach, and verification with Accessibility Insights — and it says accessibility tests are integrated into **every** UI tranche. PRD D-3 and the risk register rate late accessibility High/Very high, so this is not a polish pass.

The Tranche 0 measurements found that Slint's `ListItem` carries an **empty `Name`**, with the visible text on a child `Text` element. Acceptable for a spike, not for a file list. The rule for Ferrix rows: set the accessible name on the row root itself, keep `accessible-role: list-item`, and assert with a UIA query that the item — not only its child — reports the file name, its type, and its selection state.

Concretely in Tranche 1:

- Row: name = file name, role = `list-item`, state = selected/focused; column headers are buttons carrying the sort direction in their name.
- List: `accessible-role: list` with an item count, so Narrator can announce position.
- Folder change and filter results announced through a status region with `accessible-role: status`.
- Selected count announced when it changes.
- Tab order: address bar → breadcrumb → list → details columns; visible focus on each.
- Every Tranche 1 feature reachable by keyboard alone (PRD 13.21 validation criterion).

**Open, and it needs a human:** Narrator has never been run against either spike, and Accessibility Insights is not installed here. The UIA tree is asserted programmatically; the announcements themselves are a manual validation step, listed in the acceptance checklist rather than claimed by a test.

## 8. Long paths and Unicode

NAV-12 requires paths beyond 260 characters to be readable and navigable. Two mechanisms reach that and differ in what the UI must then strip: a process-wide `longPathAware` application manifest entry, available on Windows 10 1607+, and per-path `\\?\` verbatim prefixes. Whether Rust's `std::fs` already applies a verbatim prefix on Windows, and whether `jwalk` inherits that behaviour, is deliberately **not asserted** — `CLAUDE.md` rule 8 forbids claiming Windows API support without validating it on real Windows.

The measurement comes first: a fixture directory deeper than 260 characters, exercised through manifest only, prefix only, and both. `fx-fs` then exposes one function returning whichever form proved correct, and `display_name` strips any prefix so the UI never shows `\\?\C:\...`. Unicode is covered by `OsString`/UTF-16 throughout plus a fixture with accented, CJK, and RTL names.

## 9. System icons and Shell calls

`fx-shell/icons.rs` calls `SHGetFileInfo` with `SHGFI_ICON | SHGFI_SMALLICON | SHGFI_LARGEICON`, keyed by extension and attributes, and destroys every `HICON` with `DestroyIcon`. PRD 10.5 requires handling slow or failing Shell objects with timeouts and a fallback: icon lookups run off the UI thread, a lookup exceeding the configured budget yields the generic icon rather than blocking the list, and a null handle yields the same generic icon. Nothing in this path is retried.

This is the only new `unsafe` outside the Tranche 0 code, each block gets a `// SAFETY:` comment naming its invariant, and it stays inside `fx-shell`.

## 10. Error handling

`ListingError` is typed and total: `AccessDenied`, `NotFound`, `NotADirectory`, `UnsupportedLocation(LocationKind)`, `Cancelled`, `Io`. Access-denied is the common case, not the exceptional one — a normal profile has dozens of denied directories — so a denied folder renders as one row naming the folder and the reason, and the listing continues. Nothing in this tranche panics and no error is swallowed: `Cancelled` ends the stream quietly because it is the user's own action; everything else reaches the model as a visible row or a status line.

`fx-fs` never retries. Retrying filesystem reads belongs to Tranche 3, where `IFileOperation` owns conflict resolution.

## 11. User-visible strings

`CLAUDE.md` rule 22-23 and PRD 859 require every user-visible string externalized from the start, with no hard-coded text in UI code. Tranche 1 adds several dozen strings — column titles, error sentences, shortcut tooltips, the "location not supported yet" message. They go into `resources/en-US.rs` and `resources/en-US.slint`, which already exist, and the existing test in `fx-app` that asserts the native and Slint resources share the window title is extended to assert that no UI source file contains a literal user-visible string. French ships in 1.0 per PRD 859, so Tranche 1 delivers English only, with the resource layout ready for a second catalogue.

## 12. Testing

- **fx-domain:** pure unit tests — history push/back/forward/parent, selection persistence across refresh, sort ordering with folders-first and case-insensitive collation, filter matching, visibility rules, `Location` parsing including UNC, URI, and WSL forms, shortcut map.
- **fx-fs:** integration tests against a `tempfile` fixture tree — batch size, stale-generation rejection, cancellation mid-list, access-denied on a real directory, long-path behaviour per section 8, Unicode fixture. These run on Windows and are labelled as such rather than silently skipped elsewhere.
- **fx-ui:** view-model tests without a window (sort + filter + selection + column layout composition), plus a UIA assertion that a row's `ListItem` reports a non-empty name, a type, and its selection state.
- **Performance, against the PRD acceptance numbers** (`PRD.en.md:947`): a 50,000-file folder must show first items under 100 ms and scroll at 60 FPS. The first-items measure is scripted; the 60 FPS measure needs frame-level instrumentation that neither spike had, so it is a named gap in `docs/benchmarks/REFERENCE.md` rather than an assumed pass.
- **fx-app:** a manual checklist — OLE registration on the Slint window, native context menu, mouse back/forward buttons, `Alt+↑`, `F5`, `Ctrl+L`, Narrator, Accessibility Insights.
- Every `CLAUDE.md` gate runs on the result: `fmt --all -- --check`, `check --workspace --all-targets`, `test --workspace --all-targets`, `clippy --workspace --all-targets -- -D warnings`, `cargo deny check`, `cargo audit`.

## 13. Delivery slices

Each slice ends with the workspace compiling, tested, and reviewable on its own.

1. **Noyau** — `fx-domain` types, `fx-fs` listing with tokio + `CancellationToken` + generation + bounded batches, visibility rules, unit and fixture tests. No UI. Reviewable with no framework assumption.
2. **Liste virtualisée** — Slint owns the loop, `FileList` renders streamed batches, HWND re-attached to OLE and the context menu, row accessibility names and states verified by UIA query, strings externalized.
3. **Navigation** — `AddressBar` with autocomplete, `Breadcrumb` with subfolder dropdown, `NavigationHistory` with back/forward/parent/refresh, `Alt+←/→/↑`, `Ctrl+L`/`Alt+D`, `F5`, mouse buttons, double-click navigate and `ShellExecute`, per-folder view memory in `fx-settings`.
4. **Tri, colonnes, filtres, icônes, watch** — `DetailsTable` with sortable, resizable, reorderable columns, instant filtering, `SHGetFileInfo` icons with timeout and LRU, `notify` coalesced refresh, long-path fixture work, selection model with `Ctrl`/`Shift`/`Ctrl+A` surviving refresh.

## 14. PRD conformance audit

This is the diff between the first draft of this document and what section 13 of the PRD actually requires. Nine gaps were closed; two deviations are deliberate.

| PRD source | Requirement | Status in this spec |
|---|---|---|
| 13.1:601 | up to parent, refresh | **Gap closed** — `Alt+↑`, `F5` in slice 3 |
| NAV-04:1275, 13.1:610 | breadcrumb shows a segment's subfolders | **Gap closed** — `SubfolderMenu`, was absent |
| 13.1:610 | address bar accepts UNC, URI, WSL | **Clarified** — parsed and reported as unsupported, not as invalid |
| 13.2:618 | columns resizable and reorderable | **Gap closed** — `ColumnLayout`, slice 4 |
| 13.2:618 | view mode remembered per folder | **Gap closed** — `fx-settings/view_state.rs`, slice 3 |
| 13.2:618 | show hidden/system files, show extensions | **Gap closed** — `VisibilityRules`, default matches Explorer |
| 13.3:624-626 | single, Ctrl, Shift, Ctrl+A selection surviving refresh | **Gap closed** — slice 4; rectangular and pattern selection stay Tranche 2 |
| 13.21:774-776 | names, roles, **states**, announcements, focus, tab order, keyboard-only, Accessibility Insights | **Gap closed** — section 7, with the manual half named as open |
| 13.22:787-790 | `Ctrl+L`/`Alt+D`, `Alt+↑`, `F5`, `Ctrl+A` | **Gap closed** — `fx-domain/keys.rs` |
| 947 | 50,000 files: first items < 100 ms, 60 FPS | **Gap closed** — section 12, with 60 FPS flagged as uninstrumented |
| 10.5:407 | timeouts and fallback for slow Shell objects | **Gap closed** — section 9 |
| 10.8:455 | bounded L1 LRU with memory budget | **Gap closed** — section 6 |
| 859, CLAUDE.md | strings externalized from the start | **Gap closed** — section 11 |
| 10.7:426-428 | `git_state`, `tags`, `thumbnail_key`, `cloud_state` | **Deliberate deviation** — not declared until a producer exists; listed in section 5 |
| 937, ADR 0001 | Tranche 0 accepted before Tranche 1 | **Deliberate deviation, maintainer's choice** — see section 15 |

## 15. What this design does not resolve

- **Tranche 0 is not accepted.** Windows 10 22H2 was never tested, Narrator and Accessibility Insights were never run, scroll smoothness was never measured, `fmt` fails on nine pre-existing hunks, and `clippy`, `cargo deny`, and `cargo audit` have never been run here. Tranche 1 builds on an unvalidated foundation by explicit maintainer choice.
- **Mouse back/forward buttons are unproven.** A search of `i-slint-backend-winit-1.18.1/src/` and `i-slint-core-1.18.1/platform/` for a mapping of the two side buttons found none, and `PointerEventButton` is only matched against `Left`, `Right`, and `Middle` inside Slint's own items. The fallback is to read `WM_XBUTTONDOWN` from the HWND we already hold and forward it as a command. This is verified in slice 3, and if it fails, NAV-06 ships with keyboard history only.
- **60 FPS scrolling has no instrumentation** in either spike, so the PRD acceptance number for it cannot be claimed yet.
- `ShellExecute` needs its own STA/MTA decision (PRD 10.5), deferred to slice 3.
- Mica, Snap Layouts, and mixed-DPI window restore stay untouched; saved window size remains physical pixels.
- The Slint Royalty-free attribution obligation is now a distribution blocker, not a spike note. It is a legal review item, not a code task.
