# Tranche 5 — Visual tabs and dual pane

- Date: 2026-11-22
- Status: Approved for implementation after T4 visual slice
- Scope: tab strip, dual-pane layout, workspace shell and keyboard behavior
- Framework: Slint 1.18.1

## Goals

- Browser-like tab strip with add, close and active state
- Native keyboard actions for Ctrl+T, Ctrl+W and Ctrl+Tab
- Independent two-pane workspace with a stable split handle
- Copy/move to the opposite pane
- Optional synchronized navigation
- Workspace restore shell for tabs, pane paths and split ratio

## Components

- `TabStrip`
- `TabItem`
- `DualPaneWorkspace`
- `PaneDivider`
- `WorkspaceState`

## Behavior

- One active tab per pane
- Closing the final tab restores a new Home tab
- Tabs are draggable/reorderable when pointer support is stable
- Split ratio is clamped and persisted in settings
- Tab, pane and divider controls expose names and roles
- Ctrl+T opens a tab, Ctrl+W closes it, Ctrl+Tab cycles tabs

## Validation

- Domain tests for tab add/close/reorder/restore
- UI keyboard tests for Ctrl+T/Ctrl+W/Ctrl+Tab
- Desktop screenshots for normal, split and focused-pane states
- Narrator/Accessibility Insights over tab/pane controls
