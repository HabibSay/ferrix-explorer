# Tranche 0 — implementation status and validation

Status: **implementation complete, Windows validation pending.** Nothing in this file is a
substitute for running the checks on Windows.

## Delivered

| PRD Tranche 0 requirement | Artifact |
|---|---|
| Multi-crate workspace (PRD 10.1) | `Cargo.toml` + 19 crates under `crates/` (empty boundaries for later tranches) |
| Pinned toolchain | `rust-toolchain.toml` (1.92.0, `x86_64-pc-windows-msvc`) |
| Agent rules | `CLAUDE.md` |
| Slint and iced prototypes with 100,000 rows | `prototypes/slint`, `prototypes/iced` |
| COM / OLE drag-and-drop / `IContextMenu` spikes | `crates/fx-shell` |
| UI Automation root skeleton | `crates/fx-a11y` (name + `Window` role contract, default HWND provider) |
| Main window: title, icon, remembered size | `crates/fx-app`, `crates/fx-settings` |
| Rotating `tracing` logs | `crates/fx-app` (`%LOCALAPPDATA%\FerrixExplorer\logs`) |
| CI: fmt, check, build, test, clippy, cargo-deny, cargo-audit | `.github/workflows/ci.yml` |
| Legal / benchmark / UI ADR documents | `docs/LEGAL.md`, `docs/benchmarks/REFERENCE.md`, `docs/adr/0001-ui-framework.md` |
| Externalized texts | `resources/en-US.rs`, `resources/en-US.slint` |

## Verification performed here

- Parsed every `Cargo.toml` and `deny.toml` with Python `tomllib`: workspace membership, package
  name uniqueness, workspace inheritance, dependency references, build-script paths.
- Reviewed the Windows interop against the windows-rs 0.62 API surface. `Error::new(HRESULT, &str)`
  and `BOOL`'s representation were confirmed from docs.rs; the exact `_Impl` trait signatures could
  not be fetched (docs.rs does not publish the `implement` traits and the generated sources exceed
  the fetch limit).

## Not verified — blocking acceptance

1. **No Rust toolchain and no network access in this workspace.** `cargo fmt`, `cargo check`,
   `cargo build`, `cargo clippy`, `cargo test`, `cargo deny`, and `cargo audit` were **not run**.
   A compile error in the `windows-rs` calls is therefore possible; the first Windows run must fix
   any signature mismatch before the tranche is accepted.
2. **`Cargo.lock` is absent** because it cannot be generated without Cargo. Commit it after the
   first successful build: reproducibility and `cargo audit` both depend on it.
3. **Runtime behaviour is unverified**: window creation, OLE registration, popup menu, log
   creation, settings persistence, Narrator announcement, and prototype interactivity have not
   been observed on Windows.
4. **UI Automation spike is the default provider only.** Windows exposes the HWND caption and a
   `Window` control; no custom `WM_GETOBJECT`/provider is registered. Slint and iced accessibility
   trees are unverified.

## Windows acceptance checklist

Run on both Windows 10 22H2 and Windows 11, Release and Debug.

1. `cargo fmt --all -- --check` → clean.
2. `cargo clippy --workspace --all-targets -- -D warnings` → clean.
3. `cargo test --workspace --all-targets` → all tests pass.
4. `cargo deny check` and `cargo audit` → no denied licenses or known vulnerabilities.
5. `cargo run -p fx-app` → window titled “Ferrix Explorer — Tranche 0”, default icon, 1100×720 on
   first launch.
6. Resize/move, close, relaunch → size restored from `%APPDATA%\FerrixExplorer\window.ini`.
7. Logs in `%LOCALAPPDATA%\FerrixExplorer\logs\ferrix-explorer.*.log` contain start/stop entries;
   rotate by setting `RUST_LOG=debug` and confirming filtering works.
8. Narrator announces the window name; Accessibility Insights reports a Window control.
9. Right-click and Shift+F10 → native “About Ferrix” popup; Enter invokes it and logs
   “About Ferrix command invoked”.
10. Drag a file over the window → no crash; the drop cursor reports “no drop” (intentional).
11. `cargo run -p slint-spike` and `cargo run -p iced-spike --release` → type in the filter, scroll
    to the end, and confirm the UI stays responsive with 100,000 rows.
12. Record measurements and the UIA observations in `docs/benchmarks/REFERENCE.md`, then complete
    ADR 0001.

## Known limitations

- Drop target is intentionally a no-op: no `CF_HDROP` extraction, no copy/move, no effect feedback.
- `IContextMenu::GetCommandString` returns `E_NOTIMPL`; the menu is app-local, not a Shell
  extension, and only one command exists.
- Saved size is stored in physical pixels; mixed-DPI restore is not adjusted yet (Tranche 9).
- One window, fixed `IDI_APPLICATION` icon, no Snap Layouts/Mica work.
- The iced prototype keeps its two headings inline (no shared string resource); it is a spike.
