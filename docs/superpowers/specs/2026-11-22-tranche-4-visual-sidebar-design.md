# Tranche 4 — Visual sidebar completion

- Date: 2026-11-22
- Status: Approved for implementation
- Scope: visual completion of sidebar and drive surfaces; no GPUI migration
- Framework: Slint 1.18.1

## Goals

- Dense desktop sidebar with clear section grouping
- Pinned, recent, known-folder and volume sections
- Stable 180-220 px desktop width with compact mode support
- Persistent pin/unpin and reorder affordances
- USB, network and unavailable rows remain visually distinct
- Full keyboard focus and Narrator names

## Components

- `SidebarSectionHeader`
- `SidebarEntryRow`
- `SidebarVolumeGauge`
- `Sidebar`

## Behavior

- Visible insertion and removal of USB roots update the existing volume section
- Repeated network/WSL discovery cannot duplicate or erase sections
- Pin/reorder actions update `PinnedStore`
- Reorder is keyboard-accessible through Alt+Up/Down or dedicated callbacks
- Entries expose role `list-item`, label, unavailable state and default action

## Validation

- Unit tests for section ordering and pin/reorder state
- Keyboard tests for selecting and reordering entries
- Desktop screenshot at 1100x720 and 1600x900
- Narrator/Accessibility Insights over sidebar rows
