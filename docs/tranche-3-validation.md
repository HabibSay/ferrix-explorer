# Tranche 3 â€” Windows Shell validation checklist (PRD 15.3, spec 12)

Run date: **2026-11-11** (slice 0), **2026-11-12** (slice 1). Host: Windows 11 (build 22631),
single user session.
Scope of this pass: **slice 0 (spikes)** and **slice 1 (native menu, automated half)** â€” spec
`2026-11-11-tranche-3-native-shell-design.md` section 12. The slice-1 manual half is NOT RUN
(see the slice 1 section); slices 2-5 rows below are planned, not executed.

**Blocked-status note (read first).** The workstation's third-party security layer is
**CrowdStrike Falcon** (Defender real-time protection is disabled on this host). The local sensor
log `CrowdStrike-Falcon Sensor-CSFalconService/Operational` records paired events with no file
names:

- id=3: "Un processus a ete bloque parce qu'un comportement malveillant a ete detecte."
- id=4: "Un fichier a ete mis en quarantaine parce qu'un comportement malveillant a ete detecte."

The exact technique/verdict for a file is only visible in the Falcon console (agent UI or web
console), not locally. Observed behaviour on 2026-11-11, 17:22-18:00 local (differential probes,
not bypass attempts):

1. A trivial `println!` example (`spike_min`, since removed) built and ran fine â€” this is not a
   blanket block on new binaries.
2. `spike_menu.exe`: the build **writes the file successfully**, it survives on disk >1.5 s,
   **execution is denied** (`os error 5`), and the exe is **quarantined immediately after the
   blocked launch**. The verdict fires at process start, not at file write.
3. After the last rebuild (source fixed for the verb-offset bug, see (a) below), **all four**
   spikes are denied `could not execute process ... (never executed) / Acces refuse (os error 5)`.
4. Earlier binaries of the *same sources* executed successfully (`spike_thumbnails` built 17:22
   and 17:33; `spike_openwith` built 17:46 ran 17:54-17:56) â€” the verdict is per-built-binary and
   can flip after a rebuild.
5. Test binaries run but **cannot create files in `%TEMP%`** (`tempfile::tempdir()` ->
   `Acces refuse (os error 5)`), the same condition Tranche 2 recorded on 2026-11-06
   (`docs/tranche-2-validation.md`, blocked-status note).
6. One build of `spike_fileop` was interrupted by `rustc.exe` dying with
   `STATUS_PRIVILEGED_INSTRUCTION` while the exe was simultaneously quarantined; later in-tree
   builds succeeded. No claim below rests on that run.

Rows are marked **PASS** only when actually observed to pass on this machine after the code under
test was current; otherwise **BLOCKED-ENV** (ready, launch refused) or **NOT RUN**
(CLAUDE.md rule 8). Nothing here is claimed as supported that was not validated here.

## Automated gates observed on this host (2026-11-11)

| Gate | Result |
|---|---|
| `cargo fmt --all -- --check` | **PASS** (exit 0) |
| `cargo check --workspace --all-targets` | **PASS** (exit 0), 1m59s |
| `cargo clippy --workspace --all-targets -- -D warnings` | **PASS** (exit 0), no suppressions |
| `cargo deny check` | **PASS** (advisories ok, bans ok, licenses ok, sources ok) |
| `cargo audit` | **PASS** (exit 0; 8 pre-existing allowed warnings, incl. RUSTSEC-2026-0253 â€” unchanged) |
| `cargo test --workspace --all-targets --no-fail-fast` | **exit 101**: 151 passed / 66 failed / 3 ignored across all targets. **Every one of the 66 failures is `tempfile::tempdir()` -> `%TEMP%\.tmp*` -> `os error 5 PermissionDenied`** (verified from full log, `opencode\spikes\test-full.txt`): `fx-fs` listing 1/6 + longpath 0/3, `fx-ops` lib 30/66, `fx-ops` transfer 0/22. Identical to the Tranche 2 environment baseline; 0 logic failures observed. Green targets include `fx-domain` 53/53, `fx-app` 33/33, `fx-ui` 13/13 + keyboard 3/3, `fx-shell` recycle_bin (0 run / 3 ignored manual rows) |

### Slice 1 gates, re-run with `TEMP`/`TMP` redirected to a workspace directory (2026-11-12)

The `%TEMP%` block above is environmental (see note 5), so the slice-1 pass re-ran the suite with
`TEMP`/`TMP` pointed at `target\tmp` inside the workspace. That isolates the code from the
environment: with the redirect, **every** target passes.

| Gate | Result |
|---|---|
| `cargo fmt --all -- --check` | **PASS** (exit 0) |
| `cargo check --workspace --all-targets` | **PASS** (exit 0) |
| `cargo clippy --workspace --all-targets -- -D warnings` | **PASS** (exit 0), no suppressions, no `#[allow]` added |
| `cargo deny check` | **PASS** (advisories ok, bans ok, licenses ok, sources ok) |
| `cargo audit` | **PASS** (exit 0; 8 pre-existing allowed warnings â€” unchanged) |
| `cargo test --workspace --all-targets --no-fail-fast` | **PASS** with the redirect: `fx-ops` lib 79/79 (incl. the 13 new `routing` tests), `fx-app` 37/37 (incl. the 3 new menu tests), `fx-domain` 53/53, `fx-fs` 7/7 + listing 6/6 + longpath 3/3, `fx-ops` transfer 22/22, `fx-settings` 12/12 (5 new menu-settings tests), `fx-shell` 10/10, `fx-ui` 13/13 + keyboard 3/3. Without the redirect the same 66 `%TEMP%` failures as the slice-0 baseline reappear, plus two binaries the security layer refuses to launch (`fx_settings-f9a0f9abeb347279.exe`, `recycle_bin-1c17dc6e52fb9a6a.exe` â€” the same two hashes the slice-0 pass already recorded as blocked; each passes when run alone with a fresh hash) |

No new workspace dependency was added in slice 1: `fx-shell` still depends only on the `windows`
crate, `fx-ops` only on `fx-domain` (spec §4).

### Runner validation (2026-11-12)

Branch `tranche-3-native-shell` pushed at `376ea56`; CI run **43227624974** (`windows-latest`,
no third-party security layer), conclusion **success**, every step green: format, check, build,
tests, clippy, `cargo deny`, `cargo audit`.

| Target | Result on the runner |
|---|---|
| `fx-ops` lib | 79/79 (incl. the 13 `routing` tests) |
| `fx-app` | 37/37 (incl. the 3 menu-registry tests) |
| `fx-domain` | 53/53 |
| `fx-fs` | 7/7 + listing 6/6 + longpath 3/3 |
| `fx-ops` transfer | 22/22 |
| `fx-settings` | 12/12 (incl. the 5 menu-settings tests) |
| `fx-shell` | 10/10 + recycle_bin 3 ignored |
| `fx-ui` | 13/13 + keyboard 3/3 |
| `fx-a11y` | 1/1 |

**0 failed.** The two binaries this workstation's security layer refuses to launch
(`fx_settings-f9a0f9abeb347279.exe`, `recycle_bin-1c17dc6e52fb9a6a.exe`) both run here, and
the 66 `%TEMP%` failures of the local baseline do not reproduce. The local-only failures are
therefore environmental, not logical.

## Slice 0 â€” spike findings (spec section 12)

Gate per spec: findings recorded here; decisions 1-8 confirmed or amended in an addendum to the
spec. **No decision is amended by this pass** â€” the probes that could confirm decisions 1, 2, 3, 4
are BLOCKED-ENV (rows a, b), and the multi-select properties answer (row c) is pending a re-run
after a spike bug fix. Decisions stay as written; pending evidence is listed per row.

### (a) Menu merge on UI thread and helper STA thread â€” `spike_menu` â€” BLOCKED-ENV

Ready to run, launch refused (all modes): phase A enumeration (labels, canonical verbs,
`QueryContextMenu` timing, 7-Zip/Git/properties reality check on a `.zip`, a folder and a `.txt`),
`--show` (popup on this thread), `--helper` (popup on a dedicated STA thread, decision 4's
design minus the product timeout).

- **Spike bug found and fixed before any run** (source review against the validated product code):
  `IContextMenu::GetCommandString` takes the offset relative to `idCmdFirst`
  (`fx-shell/src/recycle.rs:486` â€” `(menu id - idCmdFirst)`, comment at :478); the spike passed
  the absolute menu id, which fails silently for every entry and would have produced a false
  "verb not found" for all rows. Same fix applied to `spike_openwith` (row c) including
  `InvokeCommand`'s `MAKEINTRESOURCE(offset)`. Re-run pending.
- Decision 4 (helper STA thread + 2 s watchdog): **evidence pending** â€” the popup probes have not
  executed on this host.

### (b) IFileOperation + sink + cancel + metadata + long path â€” `spike_fileop` â€” BLOCKED-ENV

Ready to run, launch refused. Covers: copy with metadata parity (`set_times` post-pass), unicode
move tree (`file-alpha`, `zi mu lu`, `file-cyr`), >260-character long path display/verbatim,
cancellation via `E_ABORT` returned from the sink, all on fixtures in `%TEMP%`.
`CLSID_FileOperation = 3AD05575-8857-4850-9277-11B85BDB8E09` (from the SDK header; the windows
crate does not export it). Decisions 1/2/3 and spec section 10's cancellation mechanism:
**evidence pending**.

### (c) SHOpenWithDialog and properties â€” `spike_openwith` â€” PARTIAL PASS

Run 2026-11-11 17:54-17:56 local (log `%TEMP%\opencode\spikes\run-log.txt`), binary built 17:46,
two paths (`sample.zip`, `a.txt`). Verbatim:

```
FINDING | openwith: hr=Ok(()) blocked_ms=11292 (modeless if ~0ms)
FINDING | properties_see: hr=Ok(()) returned_after_ms=102 (modeless if <500ms)
FINDING | properties_multi: invoke_after_ms=0 outcome=no properties verb among 68 entries paths=2
```

- **`SHOpenWithDialog` is modal**: it returned only after the dialog was closed (11 292 ms),
  `hr=Ok(())`. OPS-15 (`PRD.md:1322`) validated: the call works, takes an owner `HWND`, and
  blocks until dismissal.
- **`ShellExecuteExW` with `SEE_MASK_INVOKEIDLIST` + `properties` is modeless**: returned
  `Ok(())` after 102 ms while the sheet stayed open (kept alive by the spike's 45 s message
  pump). OPS-14 (`PRD.md:1321`) validated. Product note: Ferrix's UI thread already runs a
  message loop, so the sheet keeps working without any extra pump.
- **`properties_multi` row: INVALID â€” spike bug.** The "no properties verb among 68 entries"
  outcome came from passing absolute ids to `GetCommandString` (fixed in the same pass as row a).
  The multi-select properties question (spec sections 7 and 15) stays **open**, not answered.
  Re-run pending: rebuilds of this spike are currently BLOCKED-ENV.

### (d) Thumbnail fetch API comparison â€” `spike_thumbnails` â€” PARTIAL PASS

Runs 2026-11-11 17:2x and 17:33 local, image `C:\Windows\Web\Wallpaper\Spotlight\img14.jpg`
(96x96 requested). Verbatim (fold values elided):

```
factory cold: (96, 96, <fold>)  warm: (96, 96, <fold identical>) (2-3ms)
factory icon-only: (96, 96, <fold identical>) 
thumbnail-cache cold: size=Some(SIZE { cx: 96, cy: 60 })
thumbnail-cache warm: Ok(()) size=Some(SIZE { cx: 96, cy: 60 }) (59-83ms)
```

- **CLSID correction (rule 8 evidence):** the first build used
  `e357fccd-a995-4576-b01f-234630154e96` as a CLSID and got
  `HRESULT(0x80040154) Class not registered`. That value is the **IID of `IThumbnailCache`**
  (`thumbcache.h:488`). The coclass is **`CLSID_LocalThumbnailCache =
  50EF4544-AC9F-4A8E-B21B-8A26180DB13F`** (`thumbcache.h:751`), confirmed present in
  `HKLM\SOFTWARE\Classes\CLSID` and pointed at `thumbcache.dll`; with it, `CoCreateInstance`
  succeeds and both cache calls return `Ok`.
- `IShellItemImageFactory::GetImage(SIIGBF_RESIZETOFIT)` returns the exact requested 96x96;
  the cache serves aspect-correct 96x60 for the same item. Warm factory call: 2-3 ms
  (in-process); warm cache call: 59-83 ms.
- For a `.jpg`, `SIIGBF_ICONONLY` folds identically to the plain request â€” the file-type "icon"
  *is* the thumbnail, so icon-vs-thumbnail discrimination must use the non-image control instead:
- **Pending (BLOCKED-ENV):** the `.txt` control row never produced output (a debug print added to
  `control_path()` forced a rebuild, and the new binary was blocked). Decision 6's
  "spike-verified" comparison is therefore **recorded but not final**: preliminary lean stays
  `IShellItemImageFactory` (faster warm, same Shell pipeline), final record after the control row
  runs.

### (e) Out-of-process surrogate feasibility memo (decision 5) â€” recorded

Timeboxed design study, no code â€” spec decision 5 asks for a credible implementation estimate or
evidence it does not fit.

- **Option A, registry surrogate** (`AppID\DllSurrogate` per third-party CLSID): rejected â€” it
  requires writing registry entries under HKCR for extensions we do not own; invasive, per-vendor,
  and silently broken by vendor updates.
- **Option B, bridge helper** (the design to estimate): our own helper process
  (`fx-shell-helper.exe`, COM stays inside the interop crate) builds the merged menu itself on a
  dedicated STA (`SHGetDesktopFolder` / `GetUIObjectOf` / `QueryContextMenu`), serializes the
  menu tree (labels, icons via `HBITMAP` -> PNG bytes, enabled/checked state, submenus, id
  offsets) over a named pipe, and forwards the chosen offset back to the original
  `IContextMenu::InvokeCommand`. A hung or crashed third-party extension affects only the
  helper; Ferrix renders its fallback after the decision 4 timeout. Two rendering choices:
  (i) the helper calls `TrackPopupMenuEx` itself â€” native look preserved, but cross-process
  popup activation is exactly the unvalidated part of probe (a); (ii) Ferrix renders the
  serialized menu in Slint â€” fully controlled, but changes the menu's look from native.
- **Estimate (credible per decision 5):** serialization protocol + pipe transport + helper
  lifecycle (on-demand start ~50-150 ms cold, reused warm) is a few days of work inside
  `fx-shell` with **no new dependencies** (std pipe/process + existing windows crate features).
  Main risks: icon fidelity, large menus (>100 entries), and the popup-activation question that
  probe (a) was meant to answer.
- **Verdict:** credible and estimable â€” decision 5's stretch slice-5 add-on, not a blocker for
  slices 1-4. What ships by default remains in-process isolation, which already satisfies
  `PRD.md:963` ("autant que techniquement possible") because undo and activity state are
  memory-only: an extension crash cannot corrupt the session, only end it. The measured half
  (is a real extension's `QueryContextMenu` worth isolating?) waits on probe (a), BLOCKED-ENV.

## Slice 1 - native menu (spec section 12, slice 1)

Gate per spec: "`fx-ops` route matrix green; manual menu checklist half (entries, keyboard,
fallback) signed off on Windows 11".

**Automated half: PASS.** The route matrix is green (13/13 `fx-ops::routing` tests), the
workspace suite is green apart from the two binaries the security layer refuses to launch
(same class as the slice-0 note above, on untouched crates), and the menu code is
compile- and lint-clean with no new dependency.

**Manual half: NOT RUN.** Every row of the checklist that needs a real right-click
(7-Zip and Git entries, keyboard reachability, the blocklist, the timeout path) is
NOT RUN on this host: the workstation's security layer refuses to launch freshly built
binaries, and no interactive right-click session was available. Nothing about the
native menu's on-screen behaviour is claimed here (CLAUDE.md rule 8).

### What slice 1 implemented

| Piece | Where | Note |
|---|---|---|
| Route gate: destination probe + `choose_route` | `crates/fx-ops/src/routing.rs` | Pure `std`, no COM. One stat per planned entry, a recursive reparse probe when the plan holds a directory. Native only for a clean copy/move with the native API probed available (decision 1). |
| Native menu merge | `crates/fx-shell/src/context_menu.rs` | `IContextMenu::QueryContextMenu` + the Ferrix section appended with command ids outside the Shell's range, `TrackPopupMenuEx` with `TPM_RETURNCMD`, canonical verb read with `GetCommandString` for logging and for the blocklist. |
| Command registry + fallback | `crates/fx-app/src/context_menu.rs` | Labels come from `resources/en-US.rs`; the fallback menu carries the same commands, so the user is never left without a menu (spec §8.7). |
| Menu settings | `crates/fx-settings/src/view_state.rs` | `MenuExtensionState`: the global native-menu switch (§8.7) and the per-identity blocklist (§8.4/§8.5), persisted in `%APPDATA%\FerrixExplorer\menu_extensions.ini`. |
| Explorer targeting | `crates/fx-app/src/orchestration.rs` | A right-click on a row outside the selection selects that row first, so the Shell's verbs and the Ferrix commands apply to the same set (the Explorer rule). `Shift+F10` / `Menu` keep the selection as it is. |
| Measurement + logging | `crates/fx-app/src/context_menu.rs` | `tracing::info!(target: "menu", query_ms, visible_ms, items, source = "shell" \| "ferrix_fallback")` (spec §8.1). |

### Decision 4 - the helper thread is implemented but not enabled

The spec's decision 4 puts the menu on a dedicated STA helper thread with a watchdog, and
names the UI-thread variant as the fallback "if the slice 0 spike proves a cross-thread
popup unreliable (activation, foreground lock)".

That spike (row (a)) is **BLOCKED-ENV**, so there is no evidence either way. The code
therefore ships **both** variants behind `MenuHost` (`crates/fx-shell/src/context_menu.rs`),
with `MENU_HOST = MenuHost::CallerThread` as the default in
`crates/fx-app/src/context_menu.rs`:

- `CallerThread` - the whole Shell interaction runs on the UI thread, the same way the
  app-local menu already does. Activation and keyboard navigation behave the way the user
  expects; a hung extension hangs the UI thread, which is the documented gap decision 4
  allows for this variant.
- `HelperThread` - the interaction runs on a dedicated STA thread; the UI thread watches
  the budget and, on expiry, logs a warning and shows the minimal Ferrix menu while the
  helper is abandoned (never killed, spec §6). The abandoned helper checks the flag before
  showing the popup, so no stray popup appears after the fallback.

Switching the default is one constant. **The choice stays open until row (a) runs**: flip
`MENU_HOST` to `MenuHost::HelperThread` once the cross-thread popup is validated on real
Windows, and keep `CallerThread` otherwise. This is a slice-1 decision the maintainer
should confirm; it is not settled by this pass.

### Rows still to observe

| # | Item | Status | Evidence / note |
|---|---|---|---|
| 1 | 7-Zip and Git entries present on right-click (file, multi, background) | NOT RUN | needs an interactive session; row (a) BLOCKED-ENV |
| 2 | `Menu` and `Shift+F10` open the same menu; every entry reachable by keyboard | NOT RUN | needs an interactive session |
| 3 | A disabled extension does not run (settings blocklist) | NOT RUN | the blocklist is persisted and honoured at invoke time; the on-screen effect is unobserved |
| 4 | Timeout path shows the minimal Ferrix menu (forced zero budget in a debug build) | NOT RUN | needs an interactive session |
| 12 | Narrator / Accessibility Insights over the menu | NOT RUN | a11y is required in every UI tranche (rule 5) |

## Manual / integration checklist (slices 1-5, spec section 12)

Reproduce any BLOCKED-ENV row with the given command once the workstation security layer allows
freshly built binaries (or run it on a machine with a developer exclusion for `target\`); tick
and date on success. Statuses start as NOT RUN unless noted.

| # | Item | Command / procedure | Status | Evidence / note |
|---|---|---|---|---|
| 1 | 7-Zip and Git entries present on right-click (file, multi, background) | run app, right-click | NOT RUN | slice 1 code in place; spike (a) BLOCKED-ENV |
| 2 | `Menu` and `Shift+F10` open the same menu; every entry reachable by keyboard | manual, Narrator | NOT RUN | slice 1 code in place |
| 3 | A disabled extension disappears (settings blocklist/disable) | settings, then menu | NOT RUN | slice 1 code in place; the blocklist is persisted and honoured at invoke time |
| 4 | Timeout path shows the minimal Ferrix menu (forced zero budget in a debug build) | debug build | NOT RUN | slice 1 code in place; decision 4 |
| 5 | Drag Ferrix->Desktop, Ferrix->Explorer, Explorer->Ferrix, multi-select, Ctrl/Shift/Ctrl+Shift, default rule | manual | NOT RUN | slice 2; decision 8 |
| 6 | 2 GB copy shows the native progress dialog; its cancel works (`PRD.md:963` acceptance) | manual, sized volume | NOT RUN | slice 5 |
| 7 | Properties sheet single- and multi-select | manual | NOT RUN | (c) single-select API PASS, sheet itself not observed; multi open |
| 8 | Open with dialog | manual | NOT RUN | (c) API PASS (modal, 11 292 ms); UI not observed |
| 9 | Thumbnails appear in the list; second visit served from cache (log proof) | run app | NOT RUN | (d) API PARTIAL |
| 10 | Native long-path copy (>260) | spike (b) / conformance test | BLOCKED-ENV | spike (b) ready |
| 11 | Metadata spot-check (mtime, creation, attributes) on a native copy | conformance test | NOT RUN | slice 5 |
| 12 | Narrator / Accessibility Insights over the menu and activity rows | manual | NOT RUN | slice 1/5, a11y every UI tranche |
| 13 | Robustness: hung extension, disk full, access denied, cancel mid-copy, race past gate | spec section 15 robustness | NOT RUN | slice 5; spike (b) covers cancel primitive |
| 14 | Windows 10 22H2 rows | manual | NOT RUN | host is Windows 11 only |

## Pending spike rows (re-run when the security layer allows)

| Row | Command | Blocked since |
|---|---|---|
| (a) menu enumeration + `--show` + `--helper` | `cargo run -p fx-shell --example spike_menu -- <zip> <folder> <txt>` (then `--show`, `--helper`) | 2026-11-11, verb-offset fix build |
| (b) file operations | `cargo run -p fx-shell --example spike_fileop -- all` | 2026-11-11, never executed on this host |
| (c) multi-select properties re-run (verb fix) | `cargo run -p fx-shell --example spike_openwith -- --interactive <zip> <txt>` | 2026-11-11, verb-offset fix build |
| (d) non-image thumbnail control | `cargo run -p fx-shell --example spike_thumbnails` | 2026-11-11, control-path debug build |
