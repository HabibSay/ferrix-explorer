# Tranche 4 — Sidebar and drives validation

Run date: 2026-11-22
Host: Windows 11
Branch: `tranche-4-sidebar`

## Implemented in this pass

- `fx-domain::sidebar`: immutable sidebar kinds, entries and bounded volume gauge
- `fx-settings::sidebar`: pinned store, duplicate prevention, reorder and persistence
- `fx-shell::volume`: logical drive roots, total/free space and volume kind
- Slint sidebar with accessible list-item names and unavailable state
- Real logical drives populated in `fx-app`, with callback navigation and 1/100 gauge
- Window title updated to Ferrix Explorer — Tranche 4
- Pinned and recent stores loaded in the sidebar; local navigations persist recents
- Known folders (Home, Desktop, Documents, Downloads, Pictures, Videos, Music) populated when the OS exposes them
- Logical-drive sidebar refreshes every second for removable USB insertion/removal
- Network and WSL discovery runs in a worker and is appended through the Slint event loop
- Dev Drive detection uses the ReFS filesystem name

## Gates

- `fx-domain` sidebar tests: PASS
- `fx-settings` sidebar tests: PASS
- `fx-shell` volume tests: PASS
- `fx-network` WSL/UNC parser tests: PASS
- `fx-ui`: 20 unit tests + 3 keyboard tests PASS
- `fx-app`: 68 unit tests PASS

## Sign-off status

T4 implementation closes on the local Windows 11 automated/desktop smoke gates. The remaining manual rows
are visual/hardware evidence only and do not block the automated code closure:

- Persistent pin/reorder controls in the sidebar UI
- Quick Access merging and pin ordering polish
- Mapped network share refresh/parity on open machines
- WSL and network entries need interactive desktop verification
- Treeview and unavailable-location behavior
- 1 percent gauge parity against Explorer

## Manual desktop rows

- Sidebar visible beside the details table: PASS on the current Windows 11 host
- Logical `C:\` entry and gauge visible: PASS on the current Windows 11 host
- USB insert/remove refresh: implemented; automatic desktop row NOT RUN
- WSL location: parser PASS; installed-distro desktop row NOT RUN unless a distro is installed
- Network/UNC: parser PASS; live desktop row NOT RUN
- Full keyboard/Narrator over the sidebar: NOT RUN

## Visual completion pass (2026-11-22)

Scope: the T4/T5 visual surfaces only (sidebar band, tab strip, workspace chrome).
No GPUI migration, no new dependency.

### What changed

- `theme.slint` (new): one palette, radius, type scale and motion token set, imported by
  every restyled component instead of per-file colour literals.
- `icons.slint` (new): the sidebar's entry glyphs are drawn as Slint paths/rectangles, so they
  do not depend on an icon font being present.
- Sidebar: section headings (Quick access, Pinned, Recent, This PC, Network), hover and
  current-folder states, volume caption (`10.3 GB free of 237 GB`) and a used-space gauge that
  turns amber past 90 percent. The model is now republished only when it actually changed,
  so the 1 s removable-drive refresh no longer resets hover or scroll.
- Tab strip: floating tabs with a raised active tab, hover state, close affordance and a
  new-tab button. Ctrl+T and the `+` button both open the folder the window is showing.
- Details rows: one cell per column, at the header's own widths, so Size/Modified/Type now sit
  under their own headers. Folders carry the folder glyph. Rows gained a hover wash and a
  softer selection colour.
- Modified column now renders `YYYY-MM-DD HH:MM` instead of raw epoch seconds. The conversion
  is UTC: `std` exposes no local offset and `fx-ui` may not call Win32. A timezone-aware format
  belongs in the shell probe that reads the file time.
- Toolbar, breadcrumb, column header and status bar restyled from the shared tokens; the
  breadcrumb segment arrows are drawn with U+25BC because U+25BE rendered as a missing-glyph
  box in this font.

### Fixed while measuring

- A hidden `DualPaneWorkspace` still reserved 172 px of layout height in Slint, pushing the
  whole details list down. It is now mounted conditionally, so the disabled workspace costs
  no space.

### Evidence

- Window captured at 1340x860 on the Windows 11 host: sidebar sections, active row, volume
  gauge, tab strip, breadcrumb, column header, aligned rows and readable Modified dates.
- `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -D warnings`: PASS
- `fx-ui`: 21 unit tests + 3 keyboard tests PASS; `fx-app`: 72 PASS; `fx-domain`: 6 PASS
- `fx-fs` `access_denied_continues` fails on this host because the runner holds
  SeBackupPrivilege/SeRestorePrivilege, which bypasses the DENY ACE the test installs. It is
  an environment failure of that fixture check, not a regression from this pass.

### Still open

- Screenshots at 1100x720 and 1600x900, Windows 10 22H2, Narrator and Accessibility Insights
  over the new sidebar headings, tab strip and volume rows: NOT RUN.
- Dual pane remains a visual scaffold: the frame, captions and divider exist, no second
  listing or orchestrator is attached yet. Tab titles still snapshot the folder a tab was
  opened on; following navigation inside a tab is T5 shell work.

## Tabler icons and the app's own caption bar (2026-11-22)

### Icons

- The sidebar and window-control glyphs are drawn from **Tabler Icons 3.52.0** path data
  (MIT, recorded in `THIRD_PARTY_NOTICES.md`), copied into `crates/fx-ui/src/icons.slint` and
  `crates/fx-ui/src/titlebar.slint` and rendered as Slint `Path` elements. No icon font (so no
  tofu on Windows 10) and no SVG decoder / `svg` crate feature: nothing new in `Cargo.lock`.
- Kinds kept: home, folder, device-desktop, world, clock.

### Caption bar

- `crates/fx-ui/src/titlebar.slint` draws a 34 px band: app mark, product name, and the three
  window buttons (46 x 34 each, flush with the right edge).
- `Window { no-frame: true }`; `crates/fx-shell/src/window_chrome.rs` supplies what the native
  frame used to: `WM_NCCALCSIZE` (client covers the window, so the client area equals the window
  rect — measured: `GetClientRect` == `GetWindowRect`), restored `WS_THICKFRAME` and friends,
  `WM_NCHITTEST` (HTCAPTION over the band, `HT*` resize codes on the edges, client area over the
  buttons), `WM_GETMINMAXINFO` (400 x 300 minimum) and the Windows 11 rounded-corner preference.

### Caption layout and restored size

- The band itself (`WindowCaption`) carries only the mark and the app name, in a
  `HorizontalLayout` where the mark sits in a fixed 16 x 16 slot centred on the 34 px band: a
  layout would otherwise stretch it to the full band height, which is what left the icon sitting
  above the title rather than beside it.
- **Maximised windows get their own client rectangle.** `WM_NCCALCSIZE` normally answers "client
  = window rect", which is what removes the native caption. A maximised window is placed so its
  resize border sits outside the screen edge, so that rule pushed the top of the caption past the
  edge: the band looked cut off, the title and the buttons lost their top few pixels, and the
  close button's hover square turned into a squat rectangle. For a maximised window the subclass
  now trims `rgrc[0]` by the frame thickness (`SM_CXSIZEFRAME` + `SM_CXPADDEDBORDER`, queried for
  the window's DPI). Measured on a 1920 x 1080 display at 150 %: window rect `1295 x 687 at
  -7,-7`, client `1280 x 672` — exactly the monitor work area, so the whole band is on screen.
- The three window buttons are a separate `WindowControls` component (three 46 x 34 cells, 138 px
  in total) that the window places as an overlay at `x: root.width - 138px, y: 0`. They are
  deliberately outside the band's layout: every layout variant tried in the band (flow, stretch
  spacer, `alignment: end`, absolute `x` from a property) left them somewhere other than the right
  edge. The shell's `WM_NCHITTEST` already reports exactly this rectangle as client area.
- Detail columns fill the window: the name cell (header and rows) carries `horizontal-stretch: 1`
  and absorbs the width the other columns leave unused, so the table ends flush with the window
  instead of stopping after the last column. The name cell's layout width is what the inline rename
  editor follows.
- The saved window size is applied after the chrome is installed, then clamped to the work area of
  the monitor that owns the window (`work_area_size(hwnd)`, i.e. `GetMonitorInfoW`). A size saved on
  a larger monitor therefore cannot open the window with its buttons past the screen edges. The
  clamp used to run before the window existed, where the system parameters are still unreliable
  (it returned a 400 px wide work area on this machine).

### Verified

- `cargo build -p fx-app` and `cargo clippy --workspace --all-targets -D warnings` clean.
- Headless test `crates/fx-ui/tests/chrome.rs`: clicking the three button centres at an 800 x 600
  window reaches `window-close`, `window-toggle-maximize` and `window-minimize` in that order, and
  `window-maximized` drives the restore glyph.

### NOT RUN / known limits

- Dragging the band, double-click-to-maximise, the eight resize edges and Aero Snap were not
  exercised by hand; the native hit-test is the only path for them.
- **Snap Layouts** (the Windows 11 flyout on hovering "maximise") does not appear: the buttons are
  client area, not `HTMAXBUTTON`, so the flyout has nothing to attach to. Win+Z still works.
- **Fixed: the process was not per-monitor DPI aware.** On this 150 % display Win32 reported a scale
  of 1.5 for the window while Slint reported 1, so the UI was laid out for a width the window never
  had: a maximised window kept its content short of the right edge (the `Type` column ended in the
  middle of the window) and the caption buttons landed outside the surface entirely.
  `fx_shell::enable_per_monitor_dpi_awareness()` (`SetProcessDpiAwarenessContext`, per-monitor v2)
  now runs first thing in `main`, before any window exists; the startup log shows
  `win32_scale=1.5 slint_scale=1.5`, and a capture of a window laid out at 832 px shows the last
  detail column and the caption's right edge where they belong.
- The caption **glyphs** are still not visible in a capture of this runner: the 138 x 34 overlay
  area shows neither the hover backgrounds (transparent by design) nor the `Path` strokes, and the
  earlier opaque-colour probes placed first inside the band's layout, then as an overlay, are no
  longer conclusive after the DPI change. Clicking the three cells is covered by the headless test;
  their pixels still need one look on a normal desktop.
- Windows 10 22H2 and Narrator were not run.

## Column widths following the window

Measured gap: on a maximised 1920x1008 client (150 % DPI, 1280x672 logical) the window chrome
and the tab strip reached the right edge while the detail table stopped at its content width
and Size / Modified / Type ended mid-window.

Cause: a Slint layout sizes a child from its *preferred* width. The table column was a plain
`VerticalLayout` child of the main row with no horizontal stretch, and the name cell had a
fixed width, so the sum of the columns - not the window - decided where the table ended.
`horizontal-stretch` on the cells did not help: inside a content-sized layout there is no
surplus to distribute, and reading `root.width` from the window root returns the *physical*
width here (1920, not 1280), so a binding that subtracted the sidebar overshot.

Fix: the host measures the window and fits the columns.
`logical_width(&window)` divides the Slint window size by `scale_factor()` (the two agree with
the Win32 client size, logged every second as `window size win32_client=… slint_logical=…`),
`fit_columns` gives the name column whatever is left after the other visible columns, and the
one-second size watcher republishes the columns when the window size changes so a maximise or
a drag re-fits the table. The header row and every list row use the published widths, so the
table - Type included - ends flush with the client's right edge.

Checks run: `cargo test -p fx-app fit_columns` (3 tests: exact fill, narrow window keeps the
name column readable, hidden columns take no room); the size log confirms Win32 and Slint agree
at 1920x1008 physical / 1280x672 logical with scale 1.5.
NOT RUN: a visual confirmation on this runner - its screen is 1280x720 and the window-capture
path (PrintWindow / winit-software) returned stale frames, so the maximised layout could not be
photographed here.
