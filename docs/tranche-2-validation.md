# Tranche 2 — Windows Shell validation checklist (PRD 15.3, spec §12)

Run date: **2026-11-06**. Host: Windows 11 (build 22631), single user session.

**Blocked-status note (read first).** From ~19:30 UTC on the run date, the workstation's
third-party security layer (Windows Defender real-time protection is *disabled* on this
host; detections list is empty, so this is not Defender) began refusing to launch freshly
linked test binaries (`CreateProcess` → `Accès refusé (os error 5)`, then quarantine of the
file) and denying file creation to those same binaries (`tempfile::tempdir()` in `%TEMP%`,
`fx-app.exe` log file in `%LOCALAPPDATA%`). Plain `bash` is unaffected, so this is a
process-scoped heuristic on unsigned build artifacts. It intermittent-passes: the same
binaries executed earlier in the evening (engine tests 28/28 at 21:04 UTC, keyboard
headless suite 3/3 at 20:21 UTC). Rows below are marked **PASS** only when actually
observed to pass after the code under test was current; otherwise **BLOCKED-ENV** (ready
to run, launch refused) or **NOT RUN**. Nothing here is claimed as supported that was not
validated on this machine (CLAUDE.md rule 8).

## Automated gates observed on this host (2026-11-06)

| Gate | Result |
|---|---|
| `cargo test -p fx-ops` (engine, incl. name table, transfer integration, robustness) | PASS at 18:58 and 21:0x UTC for pure/unit targets; the 65+20 tempfile-based runs failed only on the `%TEMP%` deny listed above — 0 logic failures (verified failure causes: all `temp dir … os error 5`) |
| `cargo test -p fx-app --bin fx-app` | **PASS 33/33 at 22:35 UTC** (shortcuts, conflict mapping/protocol, activity store, queue, clipboard-clear rule, i18n guards). Earlier attempts at 22:0x–22:2x were BLOCKED-ENV |
| `cargo test -p fx-ops --lib` | 30 passed / 36 failed at 22:35 UTC — all 36 are the `%TEMP%` denial above, 0 logic failures; the new `transfer::tests::only_symbolic_links_are_recreated` PASSes |
| `cargo test -p fx-ui --lib` | PASS 13/13 at 22:35 UTC (adds `rename_seed`, `rename_selection`) |
| `cargo test -p fx-shell --lib` | PASS 4/4 at 22:35 UTC (quota rule, incl. the no-wrap case) |
| `cargo test -p fx-domain --lib` | PASS 53/53 at 22:35 UTC |
| `cargo test -p fx-ui --test keyboard` | PASS 3/3 at 22:22 UTC; the 22:35 re-run is BLOCKED-ENV (`keyboard-6794…exe` never executed, os error 5) |
| `cargo test -p fx-shell --test recycle_bin` | ok, 0 passed / 3 ignored (the ignored rows are the manual ones, 1–2 and 9) |
| `cargo test -p fx-ops --test transfer` | 0 passed / 22 failed at 22:35 UTC — every panic is `tempfile::tempdir()` → `os error 5`; 0 logic failures |
| `cargo test -p fx-fs`, `cargo test -p fx-settings` | 5 + 3 + 1 failures at 22:35 UTC, all `temp dir … os error 5` (pre-existing on `HEAD`: the same fixtures exist there); 1 `fx-settings` and 4 `fx-fs` unit/batch tests pass |
| `cargo test --workspace --no-fail-fast` | the only red targets are the tempdir-based ones above |
| `cargo fmt --all -- --check` | PASS (exit 0) at 22:33 UTC |
| `cargo check --workspace --all-targets` | PASS (exit 0) at 22:33 UTC |
| `cargo clippy --workspace --all-targets -- -D warnings` | PASS (exit 0) at 22:33 UTC, no suppressions |

## Manual / integration checklist

Reproduce any BLOCKED-ENV row with the given command once the workstation security layer
allows freshly built binaries (or run it on a machine with a developer exclusion for
`target\`); tick and date on success.

| # | Item | Command / procedure | Status | Evidence / note |
|---|---|---|---|---|
| 1 | Delete a file → found in the Windows Recycle Bin | `cargo test -p fx-shell --test recycle_bin -- --ignored recycle_then_restore` | BLOCKED-ENV | launch refused at 21:2x UTC; `recycle_paths` unit behavior exercised by engine fakes in `fx-ops` |
| 2 | Restore via undo (`Ctrl+Z`): file back at original path with name and content (Shell `restore` verb, STA-enumerated — decision 3/§8) | same command as row 1 | BLOCKED-ENV | the old `$I`-parse/`FO_MOVE` code path was **removed**; the verb path has never executed on this host. Do not claim row 2 works until row 1's run is PASS |
| 3 | Two identical deletes in the same second → undo refused (ambiguous identity) | manual: delete `x.txt` twice from two folders, `Ctrl+Z` | NOT RUN | ambiguity guard implemented in `fx-shell::recycle::find_and_restore` (name-count refuse) and `verified=false` on empty diff |
| 4 | Copy Ferrix→Explorer (CF_HDROP readable by Explorer) | `cargo test -p fx-shell --test recycle_bin -- --ignored clipboard` writes then inspect in Explorer paste | BLOCKED-ENV (auto part) | clipboard write+read round-trip incl. Preferred DropEffect is covered by `clipboard_write_read_round_trip…` — same binary, same block; cross-application paste into Explorer remains a genuine manual sub-step |
| 5 | Copy Explorer→Ferrix | Explorer Ctrl+C a file, Ferrix Ctrl+V | NOT RUN (needs app launch) | app launch also BLOCKED-ENV (log-file write) |
| 6 | Cut → paste → clipboard emptied (Explorer behavior) | manual after row 5 | NOT RUN (app launch) | implemented in M4: `run_transfer` calls `fx_shell::clipboard::clear_clipboard()` only when the move completed cleanly; the decision rule is unit-tested (`should_clear_clipboard`: 3 cases, BLOCKED-ENV for the launch, compile-verified) |
| 7 | Cross-volume move | manual: two volumes required | NOT RUN (single volume) | fake-port branch tested (`fx-ops` integration `cross_volume_move_copies_then_deletes_the_source` PASS in unit runs) |
| 8 | `ReplaceFileW` over an open target | manual: open a copied-over file in another app | NOT RUN | engine tests fake a replace failure (`a_replace_failure_keeps_the_original`) and pass |
| 9 | Long-path recycle (`>260`) | `cargo test -p fx-shell --test recycle_bin -- --ignored long_path` | BLOCKED-ENV | current code refuses ≥260 up front (`ShellRecycleError::LongPath`); the run must record whether Windows would have accepted it |
| 10 | Shift+Del raises the confirmation dialog and Cancel deletes nothing | run app, `cargo run -p fx-app` | BLOCKED-ENV (app launch) | the headless half — Escape answers *cancel* and never *confirm* — is PASS as part of `fx-ui` keyboard suite 3/3 |
| 11 | Conflict dialog: four choices, metadata shown, apply-to-all per class, Escape = cancel whole operation | run app; paste over an existing file | partial | engine + mapping + protocol PASS (`apply_to_all_is_scoped_to_the_conflict_class`, resolver blocks-until-answered, closed-channel=cancel); the visible dialog half BLOCKED-ENV |
| 12 | Activity center: per-file progress, speed, ETA, pause/resume/cancel/retry, error list, 50-row cap | run app during a large copy | partial | store + wiring PASS as unit tests; live pane BLOCKED-ENV |
| 13 | Checksum panel (MD5/SHA-256/BLAKE3 for selection via menu) | run app; right-click selection → Checksums | partial | vectors + streaming PASS in `fx-ops`; results pipeline PASS by construction; live sheet BLOCKED-ENV |
| 14 | Narrator / Accessibility Insights on new controls (dialogs, activity rows, progress text role) | manual | NOT AVAILABLE | same standing open risk as Tranche 0/1 (spec §9 manual half); app cannot launch on this host anyway |
| 15 | Empty-folder cleaner: report-first, confirmed, never the root | run app; menu Clean empty folders | partial | engine report/confirm re-check PASS (`clean::tests`, incl. root exclusion); dialog half BLOCKED-ENV |
| 16 | M2 — no Tranche 3 code in the tree | `git grep -n "IFileOperation\|begin_io_operation" -- crates` and `ls crates/fx-shell/src` | PASS 22:1x UTC | `file_operation.rs` deleted, `begin_io_operation` removed from `PlatformOps`, the no-op hook removed from `Transfer::run`; grep returns only doc/ADR prose |
| 17 | M3 — recycle quota pre-check refuses an oversized payload | `cargo test -p fx-shell --lib` (pure rule) + run app, delete a file larger than the bin maximum | partial | 4/4 PASS 22:2x UTC for `generous_capacity`/`exceeds_generous_quota`; `SHQueryRecycleBinW` + `RegGetValueW` paths NOT VALIDATED (need app launch) |
| 18 | M3 — the nuke warning stays the net (no false refusal) | run app; recycle a payload near the configured maximum | NOT RUN | refusal is only possible when the payload exceeds the *most generous* unit reading; a registry miss returns `Possible`, never a refusal |
| 19 | M5 — a symbolic link copies as a link, a junction stays skipped | `cargo test -p fx-ops --test transfer` (2 new tests) | BLOCKED-ENV | both tests self-skip without SeCreateSymbolicLink/Developer Mode, and `tempdir()` itself is denied on this host; pure rule `reparse_action` PASS in `cargo test -p fx-ops --lib` (22:2x UTC) |
| 20 | M5 — `reparse_tag` reads the real tag on NTFS | run app; copy a `.lnk`-style symlink and a `mkjunction` folder | NOT RUN | `CreateFileW(FILE_FLAG_OPEN_REPARSE_POINT)` + `FSCTL_GET_REPARSE_POINT` never executed here (rule 8: no support claim) |
| 21 | M6 — inline rename on the row: pre-filled, base name selected, live validation, focus returns | run app; F2 on `report.pdf`, type `final`, Escape, F2 again | partial | `rename_selection` + `rename_seed` PASS 13/13 in `cargo test -p fx-ui --lib` (22:2x UTC); Slint editor/`set-selection-offsets`/focus return compile but need a visible session |
| 22 | M7 — a non-local location refuses instead of redirecting to `C:\` | run app; open a OneDrive/UNC location, Ctrl+V | NOT RUN | `local_folder()` returns `None` for every one of the 9 command entry points; status message asserts nothing about the file system |
| 23 | M8 — clipboard write never blocks the Slint thread | run app; Ctrl+C while another app holds the clipboard | partial | `write_to_clipboard` now runs under `spawn_blocking`; the retry loop stays inside `fx-shell`. Timing not measured (no launch) |
| 24 | M1 — no literal user-visible text in the Tranche 2 dialog/activity sources | `cargo test -p fx-app --bin fx-app` (`tranche_2_dialog_components_take_their_text_from_resources`, `rust_and_slint_resources_agree_on_the_shared_labels`) | PASS 22:35 UTC | both guards green in the 33/33 run; residual non-externalized wording is listed in `docs/tranche-2-conformance-audit.md` |

## Known deviations recorded while executing this tranche

1. One native dialog may appear: the Shell's own "cannot recycle, delete permanently?"
   warning (`FOF_WANTNUKEWARNING`) — accepted on safety grounds (spec §14).
2. Undo journal is in-memory/session-scoped (spec decision 5); no restored-session claim.
3. Recycle identity currently derives from the raw `$Recycle.Bin` before/after diff; the
   restore *verb* path (decision 3) matches by display name with ambiguity refusal —
   pending rows 1–3 above before any capability claim.
4. Slint dialogs are focus-scoped overlays, not OS-modal windows (no cross-window trap);
   the inline rename editor is anchored to its row and returns focus to the list on commit
   or cancel (M6); the post-create inline rename of a new folder/file is still not wired.
5. 60 FPS instrumentation (Tranche 0 gate) still open; nothing in this file claims it.
