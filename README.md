# Ferrix Explorer

A fast, keyboard-first file explorer for Windows 10 and 11, written in Rust.

> **Status:** Active development — Tranche 1 (basic navigation) is complete and Tranche 2 (file operations) is implemented on branch `tranche-2-file-operations`; the Shell-validation checklist in `docs/tranche-2-validation.md` still needs its Windows sign-off. The project is not yet ready for end-user distribution; see [License and distribution](#license-and-distribution) for the blockers.

---

## What it does

Ferrix Explorer aims to replace Windows Explorer for power users who want speed, accessibility, and extensibility. The first working tranche delivers read-only local filesystem browsing:

- Asynchronous, streaming directory listing (no UI freeze on large folders)
- List and Details views with sortable, resizable, reorderable columns
- Breadcrumb with per-segment subfolder dropdown, and an editable address bar with autocomplete
- Navigation history (Back, Forward, Up, Refresh) with scroll and selection restored
- Instant filtering and multi-criteria sorting
- System icons via `SHGetFileInfo` with timeout protection
- Full keyboard operation and accessible names/roles/states on every list row
- Long paths (> 260 characters) and Unicode file names

---

## Non-goals (current)

These are explicitly out of scope until later tranches:

- Native Shell context menus via `IContextMenu` (Tranche 3)
- Sidebar/tree, tabs, multi-pane (Tranches 4–5)
- Search and indexing (Tranche 6)
- Network, UNC, WSL, archive, and cloud locations (later tranches)

---

## Architecture

The project is a Cargo workspace. Each crate has a strict dependency boundary:

```
crates/
  fx-domain/        Pure domain types: FileEntry, Location, NavigationHistory,
                    SortKey, FilterQuery, keyboard shortcut map — no UI, no I/O
  fx-fs/            Filesystem layer: streaming jwalk listing, notify watcher,
                    long-path helpers — no UI
  fx-ops/           File-operations engine (Tranche 2): plan, transfer with
                    conflict policy, rename/create/delete, checksums, undo,
                    cleaner — no Win32
  fx-ui/            Slint UI: ApplicationWindow, FileList, DetailsTable,
                    AddressBar, Breadcrumb, SubfolderMenu, view model
  fx-shell/         Win32/COM interop: SHGetFileInfo icons, ShellExecute,
                    IDropTarget, IContextMenu — the only crates with unsafe
  fx-app/           Entry point: wires Slint event loop, orchestrates navigation,
                    manages listing tasks and cancellation
  fx-settings/      Persisted preferences: window state, per-folder view mode
                    and column layout
  fx-a11y/          Accessibility helpers (stub, grows with each UI tranche)
  fx-git/           Git status integration (future tranche)
  fx-index/         Search index (future tranche)
  ...
prototypes/
  slint/            Evaluation spike — not product code
  iced/             Evaluation spike — not product code
resources/
  en-US.rs          User-visible strings for Rust code
  en-US.slint       User-visible strings for Slint UI
```

**Key rule:** `fx-domain`, `fx-fs`, and `fx-ops` never depend on `fx-ui` or Slint. All `unsafe` stays inside `fx-shell` and other explicitly listed interop crates.

The UI framework is **Slint 1.18.1** (selected in [ADR 0001](docs/adr/0001-ui-framework.md) on accessibility grounds; the iced spike is kept as a benchmark baseline).

---

## Getting started

### Prerequisites

- **Windows 10 or 11** — the project uses Win32/COM APIs and does not compile on other platforms
- **Rust 1.92.0** — the toolchain is pinned in `rust-toolchain.toml`; `rustup` picks it up automatically
- [cargo-deny](https://github.com/EmbarkStudios/cargo-deny) and [cargo-audit](https://github.com/rustsec/rustsec/tree/main/cargo-audit) for the license and security gates (CI installs them automatically)

### Build and run

```powershell
# Clone
git clone https://github.com/HabibSay/ferrix-explorer.git
cd ferrix-explorer

# Run the app (debug build)
cargo run -p fx-app

# Run the app (release build)
cargo run -p fx-app --release
```

The window opens at the user's home directory. On first launch it creates:
- `%APPDATA%\FerrixExplorer\window.ini` — saved window position and size
- `%LOCALAPPDATA%\FerrixExplorer\logs\` — rotating log files

### Run the full gate suite

These commands must all pass before every commit:

```powershell
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo deny check
cargo audit
```

CI runs these automatically on every push and pull request (see [`.github/workflows/ci.yml`](.github/workflows/ci.yml)).

---

## Keyboard shortcuts (Tranche 1 + 2)

| Action | Shortcut |
|---|---|
| Focus address bar | `Ctrl+L` or `Alt+D` |
| Navigate back | `Alt+←` |
| Navigate forward | `Alt+→` |
| Go to parent folder | `Alt+↑` |
| Refresh | `F5` |
| Select all | `Ctrl+A` |
| Open file / enter folder | `Enter` or double-click |
| Copy / cut / paste | `Ctrl+C` / `Ctrl+X` / `Ctrl+V` |
| Rename inline | `F2` |
| Delete to Recycle Bin | `Del` |
| Permanent delete (confirmed) | `Shift+Del` |
| New folder | `Ctrl+Shift+N` |
| Undo / redo | `Ctrl+Z` / `Ctrl+Y` |

Mouse back/forward buttons are also wired to history navigation. Right-click (or
`Shift+F10`) opens the app-local menu: checksums (when a selection exists), select by
pattern, invert selection, new file, clean empty folders. Internal drag-and-drop:
`Ctrl` copies, `Shift` moves, no modifier follows Explorer's same-volume move /
cross-volume copy rule.

---

## Project conventions

- **Conventional Commits** for all commit messages (`feat:`, `fix:`, `docs:`, etc.)
- **One tranche = one branch** — never implement a feature from a later tranche
- Every `unsafe` block carries a `// SAFETY:` comment naming its invariant
- No blocking I/O on the UI thread; all long tasks are cancellable via `CancellationToken`
- Every UI control has an accessible name and role (verified with UIA assertions in tests)
- No new dependency added without a license, maintenance, and vulnerability check; then `docs/LEGAL.md` and `THIRD_PARTY_NOTICES.md` are updated
- All user-visible strings live in `resources/en-US.rs` and `resources/en-US.slint` — no hardcoded text in UI code

---

## Contributing

Contributions are welcome. Here is how to get oriented:

1. **Read the PRD** (`PRD.en.md`) — it is the authoritative source for all feature requirements and acceptance criteria. Conflicts are resolved in favour of section 13 over Annex A.
2. **Read the current tranche design doc** — the latest one is in `docs/superpowers/specs/`. It lists what is in scope, what is out, the data model, threading model, accessibility requirements, and delivery slices.
3. **Read `CLAUDE.md`** — a short list of working rules that apply to all contributors (human and AI).
4. **Check the ADRs** in `docs/adr/` before proposing a different framework, runtime, or architecture. If you have evidence that changes a decision, open an issue first.
5. **Open an issue or draft PR** before large structural changes. Small bug fixes and test additions can go straight to a PR.

### Good first contributions

- Close the [Tranche 0 open gates](docs/adr/0001-ui-framework.md#evaluation-gates): run the manual checklist on Windows 10 22H2, run Narrator and Accessibility Insights, measure 60 FPS scroll
- Add the missing French string catalogue in `resources/fr-FR.rs` and `resources/fr-FR.slint` (English is the only language shipped so far; the resource layout is ready)
- Add frame-level instrumentation for the 60 FPS scroll metric (see `docs/benchmarks/REFERENCE.md`)
- Write the missing manual acceptance test for mouse back/forward buttons (WM_XBUTTONDOWN path)

### What to avoid

- Do not implement features from a tranche that has not been started yet — check `CLAUDE.md` for the current tranche
- Do not add a dependency without going through the rule 6 review (license, maintenance, native code, vulnerabilities)
- Do not weaken lint rules, skip tests, or add `#[allow(...)]` to make CI pass

---

## Performance targets

From PRD section 947:

| Metric | Target | Status |
|---|---|---|
| First items visible (50,000-file folder) | < 100 ms | Scripted, not yet run on reference hardware |
| Scroll frame rate | 60 FPS | Gap — frame-level instrumentation not yet in place |

See `docs/benchmarks/REFERENCE.md` for the full picture.

---

## License and distribution

Source code is dual-licensed under **MIT OR Apache-2.0**.

> **Distribution blocker:** the UI framework (Slint) is used under the Royalty-Free option, which carries attribution requirements that must be satisfied before any binary is distributed. Additionally, several [evaluation gates](docs/adr/0001-ui-framework.md#evaluation-gates) (Windows 10 testing, Narrator/Accessibility Insights, scroll smoothness) remain open. No release binary should be distributed until these are resolved.

See `docs/LEGAL.md` for the full dependency register and `THIRD_PARTY_NOTICES.md` for third-party notices.
