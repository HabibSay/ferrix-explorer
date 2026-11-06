# CLAUDE.md — Working rules for agents

## Context

- Project: **Ferrix Explorer**, a Rust file explorer for Windows 10/11.
- Source of truth: `PRD.md` and the decision records in `docs/adr/`. Read the whole PRD and the
  current tranche before changing code.
- Current tranche: **Tranche 2 — file operations** (`docs/superpowers/specs/2026-11-06-tranche-2-file-operations-design.md`),
  implemented on branch `tranche-2-file-operations`. Slices 1–5 are coded; the open work is the
  Windows validation sign-off (`docs/tranche-2-validation.md`) and the batch list it links.
  Do not implement Tranche 3 features (`IFileOperation`, the native progress dialog, OLE
  drag-and-drop, native context menus) before Tranche 2 is accepted — no Tranche 3 code exists.
- The UI framework is **Slint 1.18.1**, selected in `docs/adr/0001-ui-framework.md`; the iced spike
  stays a benchmark baseline, not product code.

## Layout and dependency boundaries

- `crates/fx-domain`, `fx-fs`, `fx-ops`: business logic; must never depend on `fx-ui` or a UI
  framework.
- `crates/fx-shell`, `fx-terminal`, `fx-git`, `fx-a11y`: the only crates allowed to contain raw
  COM/Win32 `unsafe` code.
- `crates/fx-app`: native host and orchestration; `crates/fx-cli`: the `fx` companion binary.
- `prototypes/slint`, `prototypes/iced`: throwaway evaluation spikes, not product code.
- `resources/en-US.rs` and `resources/en-US.slint`: externalized UI text. Do not hard-code
  user-visible strings in UI code.

## Rules

1. One tranche = one branch. Never implement a later tranche.
2. Propose the file/module structure first and wait for validation when the change is structural.
3. Every `unsafe` block has a `// SAFETY:` comment explaining its invariant, and stays inside the
   interop crates listed above.
4. No blocking I/O or long computation on the UI thread; long tasks are cancelable.
5. Every UI control has an accessible name and role.
6. No new dependency without checking license, maintenance, native code, and vulnerabilities; then
   update `docs/LEGAL.md` and `THIRD_PARTY_NOTICES.md`.
7. No non-permissive component enabled by default. Optional engines are opt-in and isolated.
8. Never claim that a Windows API is supported without validating it on real Windows.
9. Run the checks below before every commit. Do not weaken assertions, skip tests, or add lint
   suppressions to make them pass.
10. Conventional Commits for commit messages.

## Commands

```powershell
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo deny check
cargo audit
```

The toolchain is pinned in `rust-toolchain.toml` (1.92.0, `x86_64-pc-windows-msvc`). Build and
run on Windows; the Win32/OLE code does not compile or run on other hosts.

## Manual validation for Tranche 0 (Windows 10 22H2 and Windows 11)

1. `cargo run -p fx-app` opens a window titled “Ferrix Explorer — Tranche 0” with the default icon.
2. Resize and move the window, close it, relaunch: the outer size is restored from
   `%APPDATA%\FerrixExplorer\window.ini`; logs exist in `%LOCALAPPDATA%\FerrixExplorer\logs\`.
3. Narrator announces the window name; Accessibility Insights shows a Window control.
4. Right-click and Shift+F10 open the native “About Ferrix” menu; Enter invokes it and the command
   is logged.
5. Drag a file over the window: OLE registration succeeds and the cursor shows “no drop”.
6. Run both prototypes, compare 100,000-row scrolling/filtering, then record results in
   `docs/benchmarks/REFERENCE.md` and the ADR.
