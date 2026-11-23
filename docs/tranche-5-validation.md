# Tranche 5 — Visual tabs and dual pane validation

Run date: (not run)
Host: Windows 11
Branch: `tranche-5-visual-tabs`
Spec: `docs/superpowers/specs/2026-11-22-tranche-5-visual-tabs-design.md`

## Scope

Tab strip and dual-pane workspace, as specified: `TabStrip`, `TabItem`,
`DualPaneWorkspace`, `PaneDivider`, `WorkspaceState`.

## Automated gates

| Row | Command | Status |
| --- | --- | --- |
| Formatting | `cargo fmt --all -- --check` | NOT RUN |
| Compile, all targets | `cargo check --workspace --all-targets` | NOT RUN |
| Tests, all targets | `cargo test --workspace --all-targets` | NOT RUN |
| Lints | `cargo clippy --workspace --all-targets -- -D warnings` | NOT RUN |
| Licences | `cargo deny check` | NOT RUN |
| Advisories | `cargo audit` | NOT RUN |

## Spec validation rows

| Row | Evidence | Status |
| --- | --- | --- |
| Domain tests for tab add/close/reorder/restore | `cargo test -p fx-domain` | NOT RUN |
| UI keyboard tests for Ctrl+T / Ctrl+W / Ctrl+Tab | `cargo test -p fx-ui` | NOT RUN |
| Desktop screenshots: normal, split, focused pane | capture on Windows 11 | NOT RUN |
| Narrator announces each tab and pane control | Narrator session | NOT RUN |
| Accessibility Insights shows name, role and split handle | Accessibility Insights | NOT RUN |
| Closing the last tab restores a Home tab | desktop session | NOT RUN |
| Split ratio clamped and restored from settings | desktop session + `%APPDATA%\FerrixExplorer` | NOT RUN |
| Copy/move to the opposite pane | desktop session on a scratch folder | NOT RUN |
| Synchronised navigation (optional) | desktop session | NOT RUN |

## Rules carried into the tranche

- No user-visible string in UI code: text stays in `resources/en-US.rs` and
  `resources/en-US.slint`.
- `unsafe` and its `// SAFETY:` note stay inside `fx-shell` and the other interop crates.
- No new dependency without a `docs/LEGAL.md` and `THIRD_PARTY_NOTICES.md` update.
- Nothing is committed on this branch without an explicit request.
