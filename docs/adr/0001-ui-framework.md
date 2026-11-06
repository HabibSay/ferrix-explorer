# ADR 0001: UI framework for Ferrix Explorer

- **Status:** Accepted (conditional) — Slint selected 2026-10-29 on Windows 11 evidence only; Windows 10 22H2, Narrator/Accessibility Insights, and scroll smoothness remain open
- **Date:** 2026-10-28, decided 2026-10-29
- **Decision owners:** Ferrix maintainers

## Context

Ferrix requires a virtualized file list, Windows Shell/OLE integration, multi-window behavior, mixed DPI, Snap Layouts/Mica, keyboard-first operation, and reliable UI Automation for Narrator/NVDA. The project currently targets `MIT OR Apache-2.0` licensing. A framework choice affects both product capability and distribution license.

Candidates in the initial spike are Slint 1.18.1 and iced 0.14.0. Slint provides a documented virtualized `ListView` and platform accessibility integration, but its available license choices include GPLv3, Royalty-Free, and commercial terms. iced is MIT-licensed, but its `Scrollable` widget does not provide a documented virtualized list control; the spike therefore windows the visible fixed-height rows in application code. Do not infer complete Windows UIA support for either framework from these facts alone.

## Decision

**Slint 1.18.1 is selected as the product UI framework**, on the measurements in `docs/benchmarks/REFERENCE.md`. The maintainer chose it on 2026-10-29 after both spikes were built and run on Windows 11.

The discriminator was accessibility, not speed. Slint published a virtualized tree with named rows and focusable items for a 100,000-row model; iced published a window with zero child elements, and the resolved lockfile shows no `iced*` crate depends on `accesskit` at the pinned version, so the gap is upstream rather than a spike configuration error. Startup was a near tie (691 ms against 733 ms, inside run-to-run spread) and iced shipped the smaller executable, but iced held roughly 50 MB more private memory at rest.

This decision is conditional on closing the open gates before any release:

- Windows 10 22H2 has not been tested at all; no host was available.
- Narrator and Accessibility Insights have not been run against either spike; the accessible tree was read programmatically only.
- Scroll smoothness and scrollbar thumb jumps were never measured, because neither list exposes a UIA `ScrollPattern`.
- The `ListItem` name is carried by its inner `Text` child rather than by the item itself, so Ferrix rows must set an explicit accessible name on the item.

The previous position, kept for the record: "**No framework is selected yet.** Keep `fx-app` as a small Win32 host prototype and build independent Slint and iced spike binaries. Choose only after both have been built and tested on Windows 10 22H2 and Windows 11."

## Evaluation gates

1. Build and launch reproducibly on Windows 10 and 11.
2. Render and interact with 100,000 rows; record startup, memory, filtering, scrolling, scrollbar jumps, and selection behavior.
3. Validate the accessible tree with Narrator and Accessibility Insights: window name/role, list semantics, row names, focus, selection, and off-screen items.
4. Exercise OLE drop-target registration, keyboard navigation, mixed DPI, IME, and multi-window basics.
5. Assess effort and limitations for native menus, Shell integration, Mica, and Snap Layouts.
6. Review current license terms, attribution requirements, downstream distribution, and compatibility with the intended project license.
7. Capture results and the final choice in this ADR before production UI work.

## Evidence collected on Windows (2026-10-29)

Measurements and their limits are recorded in `docs/benchmarks/REFERENCE.md`. Only Windows 11 Enterprise 23H2 was available; the Windows 10 22H2 leg of every gate below is still open.

| Gate | Status on the evidence so far |
|---|---|
| 1. Build and launch reproducibly | Partial: both spikes build in release and open a window on Windows 11; Windows 10 untested. |
| 2. 100,000 rows | Partial: startup 691 ms (Slint) against 733 ms (iced), a difference inside run-to-run spread; private bytes at rest 50.2 MB against 103.4 MB; Slint narrow-filter latency 320 ms verified against 11 expected rows. Scroll smoothness and scrollbar thumb jumps were not measured, because neither list exposes a UIA `ScrollPattern`. |
| 3. Accessible tree | The discriminator. Slint publishes `Window` → `Group` → `Edit` + `Button`, and `List` → 21 `ListItem` with named `Text` children and focusable items, for a 100,000-row model. iced publishes a `Window` with zero children and zero descendants, and no `iced*` crate depends on `accesskit` in the resolved lockfile, so this is a framework property at the pinned version rather than a spike misconfiguration. Narrator and Accessibility Insights have not been run against either. |
| 4. OLE drop target, keyboard, mixed DPI, IME, multi-window | Not covered by the spikes. Only `fx-app` exercises OLE drop registration; the machine runs at 150 percent scaling; IME and multi-window are untested. |
| 5. Native menus, Shell integration, Mica, Snap Layouts | Not assessed. |
| 6. License terms | Unchanged: Slint is carried under the Royalty-free option through per-crate exceptions in `deny.toml`, iced under MIT. See `docs/LEGAL.md`. |
| 7. Final choice | Decided: Slint, recorded in the Decision section above. Gates 1 to 5 stay open and gate release. |

## Consequences

- Slint becomes a production dependency of the UI crates. The `deny.toml` exceptions were written as "spike only" and must be re-labelled as a product dependency; the GPLv3 option stays excluded and the attribution requirements of the Royalty-free option must be satisfied before any binary is distributed.
- The iced spike stays an experiment and is not a product dependency. It remains useful as the comparison baseline recorded in `docs/benchmarks/REFERENCE.md`.
- Until the open gates above are closed, no release binary may be distributed under the assumed UI-framework license.
- The Win32 host window uses the default Windows UIA provider for the top-level HWND caption and Window control type. Custom UI elements will need their own accessibility semantics/provider and must be tested.
