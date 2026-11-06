# Tranche 2 conformance audit (slice 5 deliverable)

Audits Tranche 2 against `PRD.en.md:949-955` (content + acceptance criteria) and the
delivery slices of `docs/superpowers/specs/2026-11-06-tranche-2-file-operations-design.md`.
Evidence is code + automated tests observed on 2026-11-06; runtime rows refer to
`docs/tranche-2-validation.md`. Verification statuses reflect what was **actually
observed** — several automated suites exist whose binaries the workstation security
layer intermittently refuses to launch; those rows are marked *verify-pending*, never
claimed.

## PRD acceptance criteria (PRD.en.md:955)

| Criterion | Status | Evidence / gap |
|---|---|---|
| All operations work with visible progress feedback | Implemented; **verify-pending** (runtime) | Engine progress (~10 Hz, bounded channel) PASS in `fx-ops` tests; activity rows fed by dispatcher + status bar no longer masks messages (batch E7/E1); live pane seen only after app launch |
| Deleted files found in the Windows Recycle Bin | Implemented; **verify-pending** | `SHFileOperationW(FOF_ALLOWUNDO)` path compiled and unit-covered via fake port; real-bin run blocked (security layer) — checklist rows 1 |
| `Ctrl+Z` undoes the last reversible operation | Implemented; **verify-pending** (runtime) | Undo journal + capability gating PASS as unit tests (`fx-ops undo`); keyboard dispatch of `Ctrl+Z` PASS headless (`keyboard.rs` 3/3); end-to-end undo of a real recycle awaits checklist row 2 (restore verb never executed on this host) |
| Conflicts offer the four choices | Implemented; **verify-pending** (dialog visibility) | Engine four-way decisions + apply-to-all-per-class PASS (`apply_to_all_is_scoped_to_the_conflict_class`); dialog wired with metadata/Escape semantics; auto-Skip removed; the visible dialog itself awaits an app session |
| No false success confirmation | Implemented | Post-condition rule enforced by engine tests (`verifies_that_recycled_items_are_gone`, `replace_swaps…`); terminal states derived from summaries, `Completed` unreachable without `finish()`; refusal paths emit typed `Failed`, never silent returns (each submitted id gets exactly one terminal event) |

## PRD content items (PRD.en.md:953)

| Item | Status | Notes |
|---|---|---|
| copy/cut/paste (`CF_HDROP`, `Preferred DropEffect`) | Implemented | `fx-shell/clipboard.rs`; interop both directions = checklist rows 4–5; a successful move-paste now empties the clipboard (M4), a cancelled or partial one leaves the cut in place |
| rename (F2) | Implemented | F2 dispatch + engine rename PASS headless/unit; the editor now sits on the row, is pre-filled with the name, selects the base name only, validates as you type and returns focus (M6) |
| create folder and file | Implemented | `Ctrl+Shift+N` and menu "New file"; Explorer numbering in `create.rs`; post-create inline edit still deferred (deliberate: the created name is not a user choice yet) |
| delete to Recycle Bin / `Shift+Del` permanent | Implemented | permanent delete gated behind counted confirmation (B1/B3, Escape-cancels PASS headless) |
| multiple and rectangular selection | Partial | `Ctrl`/`Shift`/`Ctrl+A` PASS; pattern select + invert implemented (E8); **marquee is a row-range drag, not a pixel rubber band** — Slint 1.18 does not expose the list scroll offset needed for a pixel rectangle; keyboard alternative exists (spec §13.9) |
| internal drag-and-drop | Implemented; **verify-pending** (runtime) | `DragArea`/`DropArea` with Explorer modifier semantics + same-volume default rule + same-folder no-op (rule 15); no live session yet |
| conflict dialog | Implemented; **verify-pending** (dialog visibility) | see acceptance row above |
| activity center (progress, speed, errors) | Implemented; **verify-pending** (runtime) | rows fed, pause/resume/cancel per id, retry over failed items, 50-row cap, queue rows show pending + are cancellable; store PASS as unit tests |
| undo/redo incl. restore from Recycle Bin | Implemented; restore leg **unvalidated** | decision 3 verb implemented in `fx-shell::recycle`; refuses on ambiguity; checklist rows 2–3 |
| checksums MD5/SHA-256/BLAKE3 | Implemented; **verify-pending** (panel) | vectors PASS (`fx-ops`); results now flow to the ChecksumSheet via menu (E2/E8); automatic post-copy verification = OPS-18/T12 (out of scope) |
| empty-folder cleaner | Implemented; **verify-pending** (runtime) | report-first two-step (scan → counted confirmation → re-verified clean), never the root (engine tests PASS) |
| enriched status bar (ORG-01) | Implemented | counts, selected bytes, free space async refresh |

## Spec slices (design §13)

| Slice | State |
|---|---|
| 1 — `fx-ops` core | Done and committed (`d0b2d21`); 85 engine tests, re-run observed passing whenever the security layer allows launches |
| 2 — Shell adapters | Code done; `unsafe` moved back into `fx-shell` (E5); restore now via verb (E3); checklist (E4) awaiting execution on this host |
| 3 — commands/editing/status | Done (B1 + E7 + M1/M6/M8); strings routed through `resources/en-US.*` with a static guard test |
| 4 — conflicts/activity/undo | Done (B2 + E1/E2/E6); focus-trap perfection and announcement audit remain manual (Narrator unavailable here) |
| 5 — selection/DnD/cleaner/docs | Done in code (E8), marquee approximated, runtime verification pending; README/CLAUDE updated; this audit |

## Known open items after the M batch (not silently dropped)

Closed by the M batch: **M1** (strings), **M2** (Tranche 3 placeholders), **M3** (recycle
quota pre-check), **M4** (cut tracking), **M5** (symlink/junction classification),
**M6** (inline rename), **M7** (non-local locations), **M8** (off-thread clipboard writes).
See `docs/tranche-2-validation.md` rows 16–24 for what is verified and what is blocked.

Still open, with the reason:

- **Localisation coverage is partial by design.** All Tranche 2 dialog, activity, menu and
  status wording now comes from `resources/en-US.rs` / `resources/en-US.slint`, and a test
  guards that `dialogs.slint`/`activity.slint` carry no literal text. Not yet externalized:
  the typed error sentences rendered by `fx-ops` (`OperationError`'s `Display`), the
  `ascending`/`descending` sort codes, and byte/duration units (`MB/s`, `B/s`, `min`).
  Localizing the engine errors needs a variant→string map in the host, not a search-replace.
- **The recycle-bin quota pre-check is conservative and unvalidated here.** Windows exposes
  the bin's current size (`SHQueryRecycleBinW`) but not its configured maximum; the maximum is
  read from the `MaxCapacity` registry values, whose unit Microsoft does not document. The
  check therefore refuses only when the payload cannot fit even under the most generous
  reading, and `FOF_WANTNUKEWARNING` remains the net. Rows 17–18 of the checklist.
- **Symbolic link creation needs a privilege.** `reparse_tag` and `create_link` are unvalidated
  on this host (launches blocked); without SeCreateSymbolicLink/Developer Mode the copy reports
  the item as failed rather than following the link. Junctions stay skipped, per rule 6.
- **Pre-existing Tranche 1 gap found by the final Tranche 2 sweep (not fixed here: different
  tranche, different branch).** `ApplicationWindow` declares `filter-changed`, `sort-by-column`
  and `navigate-to`, and `crates/fx-app/src/main.rs` registers no handler for them — verified on
  `HEAD` as well (`git show HEAD:crates/fx-app/src/main.rs` has zero `on_sort_by_column`), so the
  filter box and the column headers are inert in the built app even though the view model exposes
  `set_filter` and `set_sort_key`. `pattern-cancelled` is unregistered too but harmless: the
  overlay clears `show-pattern-dialog` on the Slint side.
- Pixel-accurate marquee rectangle; Narrator/Accessibility Insights session; 60 FPS
  instrumentation; two-volume live test; post-create inline rename of the new item — all
  environment-gated or explicitly deferred.
