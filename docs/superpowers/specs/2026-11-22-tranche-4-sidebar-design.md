# Tranche 4 — Sidebar and drives: design

- Date: 2026-11-22
- Status: Implementation in progress
- Source: PRD.md section 16.4 Tranche 4, section 13.6, and ADR 0001
- Depends on: Tranche 3 native shell on branch `tranche-4-sidebar`, with T3 sign-off recorded by the maintainer

## Scope

Tranche 4 adds the complete navigation sidebar:

- persistent pinned locations with drag reorder
- Quick Access, Recent, Libraries/known folders
- This PC with volume space gauges
- mapped network shares and UNC entries
- USB/removable hotplug
- tree navigation
- Dev Drive detection
- WSL distributions when installed
- unavailable locations shown without blocking the UI

Out of scope: tabs, windows, dual-pane, workspace persistence, sidebar filter commands, cloud and virtual providers. They belong to later tranches.

## Module layout

- `fx-domain/src/location.rs`: sidebar item and location types remain domain-only
- `fx-settings/src/view_state.rs`: persisted pinned and recent entries
- `fx-network/src/lib.rs`: network and WSL discovery port; no UI dependency
- `fx-shell/src/volume.rs`: fixed/removable/network/dev-drive/WSL shell probes
- `fx-app/src/sidebar.rs`: discovery orchestration and sidebar state
- `fx-ui/src/ui.slint`: sidebar rows, gauges, unavailable state, keyboard/focus semantics
- `resources/en-US.rs` and `resources/en-US.slint`: all sidebar labels

## Data model

```rust
pub enum SidebarKind {
    Home,
    Pinned,
    Recent,
    KnownFolder,
    ThisPc,
    Volume,
    Network,
    Wsl,
    DevDrive,
}

pub struct SidebarEntry {
    pub id: String,
    pub kind: SidebarKind,
    pub label: String,
    pub location: Option<Location>,
    pub unavailable: bool,
    pub used_bytes: Option<u64>,
    pub total_bytes: Option<u64>,
}
```

Persistence is an application settings file. Pinned entries are ordered by `u32`, with drag reorder producing an immutable updated vector.

## Threading and failure

- Discovery that can block runs outside the Slint UI thread.
- USB and network changes invalidate discovery asynchronously.
- WSL or network outages produce unavailable entries, never a blocking error dialog.
- Volume gauges use filesystem capacity APIs and report bytes plus a normalized percentage.

## Slices

1. Sidebar model, persistence, pinned reorder and tests
2. Known folders and recent locations
3. Volume list, gauges and removable hotplug
4. Mapped/UNC network locations
5. WSL and Dev Drive discovery
6. Treeview, keyboard navigation, announcements and manual validation

## Acceptance

- Pinned entries persist and reorder
- A USB key appears/disappears without app restart
- Volume gauges match Explorer within 1 percent
- WSL locations open when installed
- Unavailable locations do not block the UI
- Every control has an accessible name and role
