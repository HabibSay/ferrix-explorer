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
