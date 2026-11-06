# PRD — Ferrix Explorer

> An open-source, native Windows 10/11 file explorer written in Rust.

> **Merged document (v3.0).** This PRD merges three concurrent PRDs into a single source of truth:
> - `PRD_claude-haiku-5.5-high.txt` — **backbone**: identified backlog (IDs), acceptance criteria, legal decisions, threading, personas, process.
> - `PRD_gpt-5.6-sol-medium.txt` — **requirements and engineering**: detailed functional specification, security, tests, KPIs.
> - `PRD_claude-sonnet-5-high.txt` — **synthesis**: "killer features" framing, thematic matrix, roadmap arbitrations.
>
> Divergences between the three have been arbitrated and are recorded in **section 6**. Each document's contributions are traced in **Annex C**.

---

## 0. Document information

| Field | Value |
|---|---|
| Codename | Ferrix Explorer (provisional, to be validated — see 9.4) |
| Document version | 3.0 (merge) |
| Status | Draft — to be validated before Tranche 0 |
| Product type | Native Windows desktop application |
| Target license | MIT OR Apache-2.0 (subject to the UI decision, section 9.1) |
| Platforms | Windows 10 (22H2+) and Windows 11 (x64 first, ARM64 as stretch) |
| Primary language | Rust — **Rust-first** positioning (see 8.1) |
| Distribution | MSI, MSIX, and portable archive |
| Repository | Cargo multi-crate monorepo |
| Project language | French (documentation), English (code, technical comments) |
| Semantic / AI search | **Out of scope, permanently** (5.3) |
| Replacing the `explorer.exe` shell | **Out of scope for 1.0** (5.3) |

---

## 1. Executive summary

Ferrix Explorer is an open-source alternative to the Windows File Explorer, written in Rust: fast to start, fluid on massive folders, reactive during long operations, compatible with usual Windows behaviors, integrated with the Shell, equipped with instant global search, keyboard- and screen-reader-accessible, extensible without compromising stability or security, and privacy-respecting.

The application must cover the main usages of Windows Explorer:

- folder navigation (local, UNC, WSL, network, cloud);
- list, details, grid, tiles, and large icons views;
- copy, move, delete, and rename operations;
- complete Windows context menu with third-party extensions;
- thumbnails and previews (Quick Look via the spacebar);
- tabs, multiple windows, and dual-pane mode;
- complete sidebar (pinned, quick access, recent, drives, cloud);
- instant Everything-style global search;
- keyboard shortcuts with Explorer parity;
- internal and OLE drag-and-drop;
- properties and metadata;
- recycle bin, archives, integrated terminal;
- advanced file management features.

**Ten concrete reasons to leave Windows Explorer:** very fluid navigation; instant global search; advanced tabs and panes; fast spacebar preview; a command palette; an integrated terminal; robust operation management; dual-pane mode; analysis and organization tools; an architecture open to contributions.

**Rust-first positioning:** all of the project's business logic is written in Rust. Some features will require Windows APIs, COM, system DLLs, or optional third-party libraries. The goal is not to guarantee that no external native code is ever loaded, but that the project's own code is Rust and that dependencies are isolated, audited, and documented (section 10.6).

The "1000 times faster" claim is a marketing ambition, not a literal technical criterion: **every advertised performance gain must be proven by a reproducible benchmark** against `explorer.exe` (sections 5.2 and 14.1).

---

## 2. Product vision

> Build the fastest, most fluid, and most productive open-source Windows file explorer, without sacrificing compatibility with the Windows Shell, accessibility, or data security.

Ferrix Explorer targets feature parity with Windows Explorer, enriches it with power-user features (instant Quick Look-style preview, command palette, integrated terminal, Everything-style instant search), and stays significantly more responsive on large folders.

The ambition is expressed as **measurable objectives (section 5.2)** rather than a marketing multiplier.

---

## 3. Problem statement

**Windows Explorer today:**

- Visible latency on large folders (tens of thousands of entries) and on network shares.
- Slow built-in search, dependent on the Windows Search indexer.
- No native quick preview (macOS Quick Look equivalent).
- Limited extensibility, slowly evolving UI, closed source.

**Existing alternatives and their limits:**

- **Total Commander**: paid, dated interface.
- **Files** (C#/WinUI, MIT): good functional reference, but not Rust and variable performance.
- **Double Commander**: cross-platform, not native Windows 11 look.
- **Everything**: excellent for search, but not a full-featured explorer.

**Opportunity:** a native Windows 11 explorer (Mica, Snap Layouts), fast, open-source, gathering power-user features in a single application.

---

## 4. Guiding principles

### 4.1 Measurable performance

- Never block the UI thread with disk, network, or Shell operations.
- Virtualize all long lists.
- Load thumbnails only when they are visible or about to be visible.
- Cancel tasks that became obsolete after a folder change.
- Use LRU-bounded caches with a memory budget.
- Measure before optimizing; publish benchmark scenarios and results.

### 4.2 Data safety

- Never consider an operation successful before the system confirms it.
- Prefer the Recycle Bin over permanent deletion; require explicit confirmation for any destructive action.
- Never silently follow dangerous links or reparse points during a recursive operation.
- Provide recovery, cancellation, and detailed error reporting; never claim an operation is reversible when it is not.
- Never log file contents, tokens, or sensitive paths without consent.

### 4.3 Windows integration

Use official Windows APIs when they provide better compatibility: Windows Shell, COM, `IFileOperation`, `IContextMenu`, OLE Drag-and-Drop, Known Folders, Media Foundation, DirectWrite, UI Automation, Cloud Files API, USN Journal, Windows Search as a possible fallback. Provide a fallback when the native API is unavailable or unstable.

### 4.4 Accessibility from the design stage

- Full keyboard navigation, visible focus, accessible names/roles/states.
- Narrator and UI Automation compatibility, high contrast.
- Respect Windows animation settings.
- **Do not defer accessibility to the end of the project** (see 6, decision D-3).

### 4.5 Open source with controlled licenses

- The default build only uses components whose redistribution is compatible with the project's license.
- Any restrictive dependency is: optional, disabled by default, isolated, documented, and explicitly accepted by the user.
- An SBOM inventory is produced for every release (section 19.2).

---

## 5. Objectives and non-objectives

### 5.1 Product objectives

- Replace Windows Explorer for daily operations without breaking Windows conventions.
- Fluid navigation: virtualized lists, zero lag on massive folders (target fluidity up to 100,000 entries).
- Complete **native Windows context menu**, including third-party application entries (7-Zip, Git, antivirus).
- Instant global search via USN Journal / MFT NTFS reading, across millions of files.
- Multi-tab, multi-window, split view, and Total Commander-style dual-pane mode.
- Quick Look: instant preview of most common formats.
- Complete sidebar: pinned, quick access, recent, libraries, drives, network, cloud accounts.
- Cloud integration (OneDrive, Google Drive, Dropbox) via Windows synchronization APIs, without reimplementation.
- Command palette, integrated terminal, companion CLI, WSL support.
- Complete accessibility from the start (UI Automation, full keyboard navigation, high contrast).
- Architecture extensible by sandboxed (WASM) plugins in v2.
- Preserve sessions, tabs, and operations after a crash; local usage without an account or mandatory telemetry.

**Technical objectives:** modular multi-crate architecture; strict separation of UI / domain / system access / infrastructure; asynchronous model with cancellation; high test coverage on critical logic; Windows interop confined to dedicated crates; reproducible benchmarks; installable and portable packaging; x64 first, ARM64 afterward.

### 5.2 Performance objectives (measurable)

All measurements are taken on a reference machine defined in `docs/benchmarks/REFERENCE.md` (created in Tranche 0), systematically compared against `explorer.exe` under the same conditions.

**Arbitration (decision D-2, section 6):** the v1.0 targets take the **demanding** values from the haiku PRD; the more realistic values from the gpt-5.6 PRD are the **acceptable floor** — a documented and accepted gap may be recorded, never silently exceeded.

| Metric | v1.0 target | Stretch target | Acceptable floor |
|---|---|---|---|
| Cold start (window shown and interactive) | < 300 ms | < 200 ms | < 500 ms |
| Warm start (window shown) | < 200 ms | < 150 ms | < 200 ms |
| Visual feedback after navigation | < 50 ms | < 30 ms | < 50 ms |
| First view of a 50,000-entry folder | < 100 ms | < 50 ms | < 150 ms |
| 100,000-entry folder | UI interactive while loading | idem | idem |
| Scrolling a 50,000-entry list | constant 60 FPS | 120 FPS on compatible screens | 60 FPS |
| Navigation to a previously visited folder (warm cache) | < 50 ms | < 20 ms | < 50 ms |
| Tab switch | < 50 ms | < 30 ms | < 50 ms |
| Name search on a 1 M-file index (p95) | < 50 ms | < 20 ms | < 100 ms |
| Initial indexing of 1 M files (NTFS, USN/MFT) | < 60 s | < 30 s | no UI blocking |
| Idle memory, 1 tab, empty folder | < 120 MB | < 80 MB | < 150 MB |
| Installer size without optional engines | < 30 MB | < 20 MB | — |
| Quick Look opening (standard image) | < 100 ms | < 50 ms | < 150 ms |
| First PDF page displayed | < 300 ms | < 200 ms | — |

These values are measurable objectives, not contractual guarantees. Any regression greater than **10%** on a benchmark scenario blocks the merge (CI, documented scenarios — see 14.1).

### 5.3 Non-objectives (v1.0)

- Replacing the `explorer.exe` process (the Windows shell remains intact; Ferrix is standalone, with a "default explorer" option in v1.5, without fragile registry hacks).
- Replacing the taskbar or the desktop.
- Shipping Linux/macOS support (the architecture stays modular for a future port, no porting work before 1.0).
- Semantic or AI search. **Out of scope, permanently.**
- Text, image, video, PDF editors or an office suite. We rely on existing applications and Quick Look.
- A proprietary cloud synchronization client: we integrate with official clients, we do not reimplement synchronization.
- Antivirus or security tooling: we rely on the native Shell scan.
- Guaranteed compatibility with all third-party Shell extensions (exact parity with Windows 10/11 menus is not guaranteed — see 13.8).
- Default inclusion of codecs or engines whose license is incompatible.
- Absolute performance promises such as "1000x" without measurement.

---

## 6. Decisions recorded (arbitrations between the source PRDs)

This chapter settles the divergences between the three merged PRDs. Any later change goes through an ADR.

| # | Subject | Decision | Rationale |
|---|---|---|---|
| **D-1** | Quick Look priority | **MVP (Tranche 8)** — the sonnet PRD deferred it to v1.5 | 2 votes to 1; it is a killer feature (impact 5) and the dedicated tranche already exists |
| **D-2** | Performance budgets | **Table in 5.2**: demanding targets (haiku) + acceptable floor (gpt-5.6) | Avoids contradictory targets (< 300 ms vs < 500 ms startup) |
| **D-3** | Accessibility positioning | **Continued from Tranche 0**, full audit in Tranche 10 | The sonnet PRD is right: added after the fact on a frozen UI, it is far more costly; the haiku PRD already made it a cross-cutting objective |
| **D-4** | Roadmap | **19 tranches (table 16.1)**: Quick Look + compression merged into Tranche 8 (haiku), full a11y in Tranche 10 with checkpoints at every tranche (gpt) | Compromise between the gpt and haiku splits; the gpt PRD remains the reference for archive security (12.5) |
| **D-5** | Crate layout | **Fine-grained split**: distinct `fx-domain` / `fx-fs` / `fx-ops` / `fx-storage` (gpt) instead of a monolithic `fx-core` (haiku) | Reduces coupling and build times; `fx-core` was becoming a dependency of everything |
| **D-6** | Command palette shortcut | `Ctrl+Shift+P` (`Ctrl+Maj+P`) | Cosmetic: the same key depending on the input language |
| **D-7** | "Pure Rust" positioning | **Transparent Rust-first**: the project's code is Rust, Windows interop is confined and documented, no promise of zero native code | Honest toward contributors and toward `cargo deny` |
| **D-8** | AI / semantic search | Excluded from scope — **permanently**, not "later" | Recorded by all three PRDs |
| **D-9** | Project name | "Ferrix" kept as provisional codename, verified before first publication | See 9.4 |

**Decisions from the source PRDs that are kept:** anticipated accessibility; Quick Look as a dedicated tranche; `fx-preview` as a dedicated crate with pluggable providers; LZTurbo never included by default; UnRAR read-only behind a feature flag; strictly opt-in telemetry.

---

## 7. Personas and target users

### 7.1 Persona matrix

| Persona | Main needs | Key features |
|---|---|---|
| **Developer** | Terminal, Git, WSL, long paths, keyboard navigation | Command palette, ConPTY terminal, Git badges, WSL support, `fx` CLI |
| **Power user / archivist** | Mass processing, comparison, integrity | Dual-pane, batch regex rename, checksums, duplicate finder, operation queue |
| **Consumer** | Simplicity, modern look, fast preview | Themes, Mica, Quick Look, mouse navigation |
| **Screen reader user** | Full access without mouse or sight | UI Automation, coherent announcements, full keyboard navigation |
| **Administrator / support** | Deployment, diagnostics, installation-free usage | Portable mode, `tracing` logs, CLI, configuration export |

### 7.2 Detailed needs

**Standard user** — find documents; copy, move, delete, rename; navigate Downloads, Documents, Pictures; access OneDrive; a familiar interface.

**Power user** — tabs; dual-pane; batch rename; advanced search; archives; checksums; folder comparison; operation queue; custom shortcuts.

**Developer** — integrated terminal; Git; WSL; CLI; long paths; hidden files; quick access to PowerShell, CMD, and Windows Terminal; fast navigation in large repositories.

**System administrator** — SMB and network drives; NTFS permissions; occasional elevation; checksums; removable drives; technical logs; robust operations.

**Accessibility** — screen reader; 100% keyboard navigation; high contrast; predictable focus; operation announcements; an interface usable without drag-and-drop.

---

## 8. Technology stack

### 8.1 Technology choices

| Component | Choice | License | Notes |
|---|---|---|---|
| Language | Rust (edition 2024 if stable at Tranche 0, otherwise 2021) | — | MSRV pinned in `rust-toolchain.toml` |
| UI | Slint (plan B: iced, or Win32 + Direct2D) | GPLv3 / Royalty-Free / commercial | **Blocking decision, see 9.1** |
| Windows interop | `windows-rs` (IShellFolder, IContextMenu, IFileOperation, CfApi, ConPTY, UIA, DirectWrite, Media Foundation) | MIT/Apache-2.0 | Official Microsoft API |
| FS traversal | `jwalk` + `rayon` | MIT / MIT-Apache | Parallel traversal |
| FS watching | `notify` | Permissive | Events for open folders |
| Index and search | `tantivy` + USN Journal / MFT reading | MIT | Everything-style search |
| Local database | `rusqlite` (embedded SQLite) | MIT | Metadata, recents, tags, config (see 10.9) |
| Async | `tokio` | MIT | Network I/O, cloud, orchestration |
| Cloud | `reqwest`, `oauth2`, Microsoft Graph, Google Drive API v3, Dropbox API | MIT/Apache-2.0 | OAuth2 + PKCE via system browser |
| Compression (default) | `libdeflater`, `zstd`, `sevenz-rust` | MIT / BSD / Apache-2.0 | See 12 |
| Compression (RAR, read-only) | `unrar` via `unrar.dll` | **Restrictive** | Optional, disableable, see 9.3 |
| Images | `zune-image`, `fast_image_resize` | MIT/Apache-2.0 | SIMD decoding and resizing |
| RAW | `rawloader` or maintained successor | MIT/Apache-2.0 | To confirm at Tranche 8 (maintenance) |
| PDF | `pdfium-render` (Pdfium engine) | BSD / MIT-Apache | **No MuPDF** (AGPL) |
| Audio | `symphonia` | MPL-2.0 | Waveform, metadata |
| Video | Windows Media Foundation (native) | System | `ffmpeg` optional (plugin) |
| Syntax highlighting | `tree-sitter` (fallback `syntect`) | MIT | Incremental |
| Fonts | DirectWrite (native) | System | Font preview |
| Terminal | ConPTY via `windows-rs` | MIT/Apache-2.0 | Integrated terminal |
| Config | `serde` + TOML | MIT/Apache-2.0 | User files |
| Logs | `tracing` + `tracing-subscriber` | MIT | Rotation, redaction (see 14.5) |
| Plugins (v2) | `wasmtime` | Apache-2.0 with LLVM exception | WASM sandbox |
| Packaging | `cargo-wix` (MSI) and MSIX | MIT | Installer and Store-ready |
| CI/CD | GitHub Actions (`windows-latest`) | — | Build, tests, lint, licenses, release |

### 8.2 Dependency selection requirements

Each dependency is examined for: license; project activity; vulnerabilities; native code presence; Windows ARM64 compatibility; size; update policy; actual necessity. Any sensitive component (LZTurbo, RAR, FFmpeg, PDFium, various SDKs) requires an ADR and legal review before distribution (see 9 and 19.2).

---

## 9. Open decisions and legal matters

### 9.1 UI license (blocking decision before Tranche 0)

Slint is available under GPLv3, under a Royalty-Free license (mandatory "Made with Slint" badge, non-OSI license), or under a commercial license. **A MIT/Apache-2.0 license for Ferrix is incompatible with a GPLv3 dependency distributed in the binary.**

**Options:**

1. Project licensed under **GPLv3** (simple, OSI, compatible with Slint GPL). Gives up the MIT/Apache-2.0 dual license.
2. Keep **Slint Royalty-Free** and accept the badge and a non-OSI license for the UI part.
3. Switch to **iced** (MIT) or **Win32 + Direct2D** (full control, longer development).

**Recommendation:** decide in Tranche 0 using the prototypes (section 11), and only retain Slint **if it meets the accessibility and Windows integration criteria**. Spike criteria: virtualization of 100,000 elements, UI Automation, OLE drag-and-drop, native menus, DPI rendering, IME, multi-window, Snap Layouts, Mica, latency, memory, binary size, license. Record the decision in `docs/adr/0001-ui-framework.md`.

### 9.2 LZTurbo (powturbo)

Non-OSI license (free non-commercial use, source not free). It will therefore **never be included in the default distributed build**. It is an **optional backend** that the user installs themselves and explicitly enables, with a warning displayed in the settings. Before any integration: verify the license in force at integration date, redistribution rights, commercial use, source availability, x64/ARM64 compatibility, performance on real scenarios, security audit.

This logic **"OSS default + opt-in third-party engine"** applies to any non-permissive component (MuPDF, full ffmpeg build, etc.).

### 9.3 UnRAR

The UnRAR license prohibits reconstructing the RAR compression algorithm. RAR support is limited to **reading and extraction** via the official `unrar.dll`, isolated in the `fx-codec` crate, disableable at compile time (feature flag `rar`) and documented in `docs/LEGAL.md`. No creation of RAR archives.

### 9.4 Project name

"Ferrix" must be checked (trademark conflicts, domain names, GitHub repositories, crates.io) before the first publication.

### 9.5 Telemetry and privacy

Strictly **opt-in**, transparent (data listed in the settings), anonymous, disableable at any time. No telemetry by default, no account required, no file paths sent by default, opt-in crash reporting with preview before sending, local searches and indexing. Details in section 14.4.

### 9.6 Licenses and dependencies (delivery)

The project maintains `LICENSE-MIT`, `LICENSE-APACHE`, `THIRD_PARTY_NOTICES.md`, `docs/LEGAL.md`, and an **SBOM per release**. Every non-permissive dependency is listed in `docs/LEGAL.md` with its status: **default**, **optional**, **isolated**, or **opt-in**. `cargo deny` blocks any unauthorized license in CI without a documented exception.

---

## 10. Software architecture

### 10.1 Cargo workspace (multi-crate monorepo)

Fine-grained layout retained (decision D-5):

```text
ferrix-explorer/
├── Cargo.toml                  # workspace
├── rust-toolchain.toml         # pinned MSRV
├── CLAUDE.md                   # rules for AI agents (see 20.1)
├── crates/
│   ├── fx-domain/              # Entities, business rules, commands (FsEntry, selection rules…)
│   ├── fx-fs/                  # Listing, metadata, long paths, traversal, watching
│   ├── fx-ops/                 # Operation queue, conflicts, undo/redo, checksums
│   ├── fx-shell/               # Windows Shell COM interop (IContextMenu, IFileOperation, icons, properties)
│   ├── fx-index/               # Search: USN/MFT reader + tantivy
│   ├── fx-preview/             # Quick Look: preview providers
│   ├── fx-codec/               # Pluggable compression/decompression backends
│   ├── fx-cloud/               # Cloud providers (trait CloudProvider): onedrive/, gdrive/, dropbox/
│   ├── fx-network/             # SMB, FTP/SFTP/WebDAV (v2)
│   ├── fx-terminal/            # Integrated terminal (ConPTY)
│   ├── fx-git/                 # Git states, badges (v1.5)
│   ├── fx-a11y/                # Accessibility (UIA wrapper, announcements, roles)
│   ├── fx-settings/            # TOML config, themes, shortcuts, persistence, portable mode
│   ├── fx-storage/             # SQLite, migrations, persistent caches
│   ├── fx-ui/                  # UI components (views, sidebar, tabs, palette)
│   ├── fx-plugin-api/          # Stable API for plugins (v2, empty skeleton in 1.0)
│   ├── fx-plugin-host/         # WASM runtime (v2, empty skeleton in 1.0)
│   ├── fx-cli/                 # Companion CLI binary `fx`
│   └── fx-app/                 # Main binary, orchestration
├── resources/                  # Icons, assets, translations
├── benchmarks/                 # Benchmark scenarios and results
├── test-data/                  # Test datasets
├── installer/                  # cargo-wix scripts, MSIX manifest
├── scripts/
└── docs/
    ├── LEGAL.md                # Non-permissive dependencies and their status
    ├── ARCHITECTURE.md
    ├── SECURITY.md
    ├── PRIVACY.md
    ├── BENCHMARKS.md
    ├── CONTRIBUTING.md
    ├── adr/                    # Architecture Decision Records
    ├── benchmarks/             # Reference machine, scenarios, results
    └── user/                   # User documentation
```

### 10.2 Layers (hexagonal architecture)

```text
UI
 ↓ commands / events
Application
 ↓ use cases
Domain (fx-domain)
 ↓ ports/traits
Infrastructure
 ├── Filesystem (fx-fs)
 ├── Windows Shell (fx-shell)
 ├── Index (fx-index)
 ├── SQLite (fx-storage)
 ├── Cloud (fx-cloud)
 ├── Network (fx-network)
 └── Codecs (fx-codec)
```

**Dependency rules:**

- `fx-domain`, `fx-fs` and `fx-ops` **never** depend on `fx-ui` nor on the UI framework. All business logic is testable without a window.
- The UI must not call Win32 or COM APIs directly.
- Only `fx-shell`, `fx-terminal`, `fx-git` and `fx-a11y` contain `unsafe` code or raw COM. Every `unsafe` block carries a `// SAFETY:` comment and a targeted test.
- `fx-codec` and `fx-preview` expose traits; optional implementations sit behind feature flags.
- `fx-cli` reuses `fx-domain`/`fx-fs`/`fx-index`, never `fx-ui`.

### 10.3 Threading model

| Context | Role | Rule |
|---|---|---|
| **UI thread** | Rendering and input events | **No blocking I/O**, no long computation |
| **`tokio` runtime** | Async I/O, network, cloud, orchestration | Talks to the UI via `invoke_from_event_loop` |
| **`rayon` pool** | FS traversal, hashing, initial indexing | Results streamed in batches, never blocking waits on the UI side |
| **Dedicated threads** | USN Journal reading, ConPTY pumps | `crossbeam` or `tokio::sync::mpsc` channels |

Every long task is **cancellable** (`CancellationToken`) and **reported back** to the UI with a state (pending, running, done, error).

### 10.4 Async and cancellation

- Tokio runtime for asynchronous operations; dedicated pools for blocking tasks.
- `CancellationToken` for navigation, search, and previews.
- A **generation** identifier to reject stale results (folder change during a listing).
- Backpressure on channels; priority to visible tasks.
- No unmanaged `.await` or blocking call on the UI thread.

### 10.5 COM and Windows interop

- Explicit COM initialization per thread; respect STA/MTA according to the API.
- RAII wrappers; strict limitation of `unsafe` code; every block documented.
- Tests on real Windows; handling of slow or failing Shell objects (timeouts, fallback — see 13.8).

### 10.6 Rust-first positioning

The project's code is in Rust. Windows interop (COM, system DLLs, Media Foundation, DirectWrite) is confined to the dedicated crates, documented, and tested. No promise of zero external native code (decision D-7).

### 10.7 Main data model

```rust
pub struct FileEntry {
    pub id: EntryId,
    pub path: PathBuf,            // OsString/UTF-16 for \\?\ long paths
    pub display_name: String,
    pub kind: EntryKind,          // file, folder, link, reparse, cloud placeholder…
    pub size: Option<u64>,
    pub modified: Option<SystemTime>,
    pub attributes: FileAttributes,
    pub icon_key: IconKey,
    pub thumbnail_key: Option<ThumbnailKey>,
    pub cloud_state: Option<CloudState>,   // synced, syncing, error, online-only
    pub git_state: Option<GitState>,       // v1.5
    pub tags: Option<Tags>,                // v1.5
}

pub struct TabState {
    pub current_location: Location,
    pub back_history: Vec<NavigationState>,
    pub forward_history: Vec<NavigationState>,
    pub view: ViewSettings,
    pub selection: SelectionState,
    pub scroll_anchor: Option<EntryId>,
}

pub enum Location {
    Local(PathBuf),
    Unc(String),
    Wsl { distro: String, path: String },
    Search(SearchQuery),
    Archive { archive: PathBuf, inner_path: String },
    Cloud(CloudLocation),
    Virtual(VirtualLocation),
}
```

### 10.8 Multi-level cache

| Level | Content | Policy |
|---|---|---|
| **L1 memory** | Icons, thumbnails, recent listings, previews | Bounded LRU, configurable memory budget |
| **L2 SQLite** | Recents, pins, tags, workspaces, configuration, listing metadata | Persistent, versioned migrations |
| **L3 disk** | `tantivy` index, thumbnail cache, hash cache | In `%LOCALAPPDATA%\FerrixExplorer\cache`, purgeable, rebuildable |

### 10.9 Local storage

SQLite (`fx-storage`) for: preferences, history, recents, pins, workspaces, tags, operation state, cache metadata, migrations. Tantivy (`fx-index`) for the search index.

**Requirements:** transactions; versioned migrations; safety backup before a destructive migration; detectable corruption; rebuildable caches; strict separation of critical and reconstructible data.

### 10.10 Data locations

- Standard mode: `%APPDATA%\FerrixExplorer\` (configuration) and `%LOCALAPPDATA%\FerrixExplorer\` (cache, index).
- Portable mode: everything in a `data\` folder next to the executable (specific marker required). No registry writes and no `%APPDATA%` writes.

---

## 11. UI spike (Tranche 0)

The UI framework **must not be finally chosen without a prototype**. Candidates: Slint; WinUI via interop; `windows-rs` with Direct2D/DirectComposition; any other maintained Rust framework meeting the criteria.

The spike compares, on a prototype main window and a virtualized list of 100,000 elements:

- virtualization and latency; memory usage; binary size;
- UI Automation accessibility (Narrator announcing a virtualized list);
- OLE drag-and-drop and native menus;
- DPI rendering, IME, multi-window, Snap Layouts, Mica;
- **license** (see 9.1).

The decision is recorded in `docs/adr/0001-ui-framework.md` before any production UI code is written.

---

## 12. Quick Look and compression engine

### 12.1 Governing principle

For each content type, Ferrix picks **the fastest library available under a permissive license**. Faster engines under restrictive licenses (LZTurbo, MuPDF, full ffmpeg) are added as **optional backends**, installed and explicitly enabled by the user, **never by default**.

There is no single optimal decompressor for every format: each archive imposes its format and algorithm. Ferrix therefore uses a **codec registry** and selects the fastest compatible backend for the format at hand.

### 12.2 Engine table per type

| Type | Default engine (OSS, included) | License | Optional "performance" backend | Notes |
|---|---|---|---|---|
| JPEG/PNG/WebP/AVIF images | `zune-image` (pure Rust, SIMD) | MIT/Apache | `turbojpeg` (libjpeg-turbo) for max JPEG throughput | WIC fallback; progressive decoding |
| RAW images | `rawloader` or maintained successor (pure Rust) | MIT/Apache | `libraw` (LGPL), enableable | Preview via embedded thumbnail first |
| Resizing / thumbnails | `fast_image_resize` (SIMD) | MIT/Apache | — | Background thumbnails, viewport-prioritized |
| ZIP / Deflate archives | `libdeflater` (libdeflate bindings) | MIT | **LZTurbo** (external plugin, third-party license) | Backend chosen by user preference |
| 7z archives | `sevenz-rust` (pure Rust) | MIT/Apache | Native 7-Zip SDK as fallback | Read and create |
| RAR archives | `unrar` (read-only) | ⚠️ Proprietary | — | `rar` feature (see 9.3) |
| TAR/GZIP/XZ/BZIP2 | Maintained Rust crates | MIT-family | Configured external tool | |
| Internal compression (cache, transfers) | `zstd` (long-range mode) | BSD | LZTurbo if installed | |
| PDF | `pdfium-render` (Pdfium, BSD) | BSD | — | MuPDF excluded (AGPL) |
| Video (thumbnail and playback) | Windows Media Foundation (native) | System | `ffmpeg` (plugin, LGPL/GPL depending on build) | Zero license dependency by default |
| Audio (waveform, metadata) | `symphonia` (or Media Foundation) | MPL-2.0 | — | |
| Source code and text | `tree-sitter` (incremental) | MIT | `syntect` fallback (Sublime grammars) | Encoding detection mandatory |
| Fonts | DirectWrite (native) | System | — | Preview with sample text |
| Office (docx, xlsx, pptx) | Native Shell Preview Handlers | System | Custom parsing in v2 | Metadata as last resort |
| Archive listing (preview) | Archive backend above | — | — | No extraction |

Exact libraries are validated by an ADR covering: license, security, maintenance, performance, ARM64 compatibility, distribution cost, vulnerability history.

### 12.3 Pluggable architecture (Rust traits)

```rust
/// Compression/decompression backend. Implementations: builtin, perf (opt-in), external (plugin).
pub trait CompressionBackend: Send + Sync {
    fn id(&self) -> &'static str;
    fn kind(&self) -> BackendKind;              // Builtin | OptionalPerf | External
    fn supports(&self, format: ArchiveFormat) -> bool;
    fn decompress(&self, input: &[u8], out: &mut Vec<u8>) -> Result<usize, CodecError>;
    fn compress(&self, input: &[u8], level: u8, out: &mut Vec<u8>) -> Result<usize, CodecError>;
}

/// Archive codec registry: probe, list, extract.
pub trait ArchiveBackend: Send + Sync {
    fn id(&self) -> &'static str;
    fn probe(&self, source: &ArchiveSource) -> ProbeResult;
    fn list(&self, source: &ArchiveSource) -> Result<Vec<ArchiveEntry>>;
    fn extract(
        &self,
        source: &ArchiveSource,
        destination: &Path,
        options: ExtractOptions,
        cancel: CancellationToken,
    ) -> Result<ExtractReport>;
}

/// Quick Look preview provider.
pub trait PreviewProvider: Send + Sync {
    fn id(&self) -> &'static str;
    fn supports(&self, input: &PreviewInput) -> SupportLevel;   // extensions, MIME, signature
    fn priority(&self) -> u8;
    fn preview(
        &self,
        input: PreviewInput,
        context: PreviewContext,            // time/size budget, trust level
        cancel: CancellationToken,
    ) -> PreviewFuture;
}
```

Each provider declares: supported extensions; MIME or signature detection; priority; limits; trust level; possible isolation needs; its license and dependencies.

**Backend selection rule** (in `BackendRegistry` and `PreviewRegistry`):

1. Explicit user preference (settings), if the backend is available.
2. `OptionalPerf` or `External` backend enabled and detected.
3. `Builtin` backend (always available).

**Fallback rule:** if a provider fails or exceeds its budget, the next one is tried, then the native Shell Preview Handler, then the Windows Imaging Component, then the system icon. Failure is never silent: it is logged (`tracing`) and discreetly surfaced in the preview.

### 12.4 Quick Look experience

- Triggered by the **spacebar** on the selected item; closed by Escape or another spacebar press.
- Navigation while previewing: up/down arrows (previous/next item), fullscreen (F11 or double-click).
- Rendering is **asynchronous, cancellable, and memory-limited**: changing the item cancels the ongoing render.
- Visible loading state; risky formats parsed out-of-process / isolated (see 14.3).
- L1 cache of recent previews.
- MVP types: images (+ animated GIFs), PDF, text, Markdown, source code, JSON/XML/YAML/TOML, audio, video, fonts, archives (listing), metadata, Office via Shell Handlers.
- v1.5 types: RAW, SVG (strict security policy), common 3D models, e-books, richer Office previews, hex viewer.
- v2 types: archive browsing as a virtual folder (PRV-10), enriched EXIF panel (PRV-09).

### 12.5 Archive security

The engine must protect against: Zip Slip and path traversal; absolute paths; dangerous symbolic links; decompression bombs; lying declared sizes; recursive archives; excessive memory usage; invalid Windows names; silent overwriting; special files.

Before extraction, the UI shows: compressed size; estimated extracted size; file count; suspicious ratio warning; required disk space.

### 12.6 LZTurbo policy

LZTurbo is treated as an **experimental optional backend** (see 9.2). If it is incompatible: never in the official build, never automatically downloaded, only supported through an external adapter explicitly installed by the user, status documented in `docs/LEGAL.md`. The same policy applies to any proprietary or restrictively licensed codec.

---

## 13. Detailed functional requirements

> This chapter expands, requirement by requirement, the identified backlog in **Annex A**: each subsection carries its IDs (`NAV-01`, `OPS-03`…). In case of conflict, this chapter prevails; Annex A serves as the prioritization index.

### 13.1 Navigation (NAV-01 → NAV-16)

The application must allow:

- navigating local files and folders, UNC shares, mapped network drives, and WSL locations;
- direct path entry, including long paths (`\\?\`) and Unicode;
- going back to the previous folder, forward to the next one, up to the parent, refreshing;
- opening multiple windows and multiple tabs;
- moving, detaching, and restoring tabs;
- opening a folder in a new tab, a new window, or the other pane;
- double-click: open a folder (navigate) or a file (associated application via `ShellExecute`);
- instant filtering (real-time filtered list);
- multi-criteria sorting and grouping;
- list virtualization (only visible entries are created).

**Breadcrumb (NAV-04)** — shows every path segment; allows clicking a segment; shows a segment's subfolders; can be temporarily replaced by an editable address bar (NAV-05, with autocomplete); accepts local paths, UNC, compatible URIs, and WSL paths.

**History (NAV-06)** — each pane has its own history: back (`Alt+←`, mouse back button), forward (`Alt+→`), parent folder, scroll position, and current selection when possible.

### 13.2 Views (NAV-02, NAV-03, NAV-09, NAV-10, UX-06, UX-07)

Required views: list; details; small icons; medium icons; large icons; extra-large icons; tiles; content.

Associated functions: zoom with `Ctrl+wheel`; view mode remembered per folder or folder type; resizable and reorderable columns; ascending/descending and multi-criteria sorting; grouping (type, date, size); showing hidden and system files; showing extensions; optional checkboxes; compact/normal/comfortable density.

The list must be virtualized and only create visible components.

### 13.3 Selection (OPS-11)

- single click; double-click; `Ctrl+click`; `Shift+click`; `Ctrl+A`;
- rectangular selection; keyboard selection; inverting the selection; selection by pattern;
- reasonable selection persistence after refresh;
- accessible announcement of the number of selected items.

### 13.4 File operations (OPS-01 → OPS-24, ORG-01)

**Required functions** — copy; cut; paste; move; delete to Recycle Bin; permanent delete (`Shift+Del`); rename (F2); create folder; create file; create shortcut; open; open with; show properties; duplicate; send to; compress; extract; compute a checksum (MD5, SHA-256, BLAKE3); copy path; copy name; open a terminal in this location; clean empty folders.

**Activity center (OPS-17)** — all long operations appear in an activity center: global and per-file progress; instantaneous and average speed; estimated time; current file; pause; resume; cancel; retry; error display; collapsible pane. Operations go through `IFileOperation` when possible, with the native progress dialog (OPS-08).

**Conflicts (OPS-09)** — replace; skip; keep both; compare metadata; apply the decision to all similar conflicts; cancel the operation.

**Undo/Redo (OPS-05)** — records rename, move, folder creation, Recycle Bin, tags, pins. **The UI must not claim an operation is reversible when it is not.**

**Integrity (OPS-18)** — optional source/destination hash verification after copy, reported to the user.

### 13.5 Batch rename (OPS-03)

Preview before applying; find/replace; regular expressions; numbering; case change; prefix/suffix; date usage; EXIF metadata usage; invalid character cleanup; collision detection; cancellation of the whole operation when possible.

### 13.6 Sidebar (SID-01 → SID-10)

**Expected sections** — Home; Pinned; Recent; Downloads; Documents; Pictures; Music; Videos; Desktop; This PC; local volumes; removable volumes; network drives; Network; WSL; cloud accounts; tags; saved searches; workspaces.

**Functions** — pin/unpin; reorder by drag-and-drop; rename a favorite **without renaming the real folder**; collapse/expand a section; show used space (gauge) for volumes; show the status of unavailable locations; persist configuration across sessions. Libraries come from KnownFolders (SID-04), removable detection is hotplug (SID-07), Dev Drive is detected (SYS-01).

### 13.7 Tabs, windows, and dual-pane (NAV-13 → NAV-16, SID-08)

**Tabs** — new tab (`Ctrl+T`); close (`Ctrl+W`); reopen closed tab (`Ctrl+Shift+T`); duplicate; pin; reorder; detach; move to another window; close others; close to the right; restore session.

**Dual-pane** — two independent panes; one tab or tab group per pane; fast copy and move to the other pane; pane swapping; optional navigation sync; visual comparison; clearly identifiable focus; Commander-style shortcuts.

**Workspaces (SID-08)** — a workspace saves: windows; tabs; paths; dual-pane layout; pane sizes; the associated terminal; filters and searches; monitor and window position, if still available.

### 13.8 Windows context menu (SYS-03, SYS-04)

Functions: context menu on a file, on multiple files, and on the folder background; Shell verbs; submenus; third-party extensions; custom Ferrix commands; keyboard support for the Menu key and `Shift+F10`.

**Isolation and stability** — third-party Shell extensions can be slow or unstable. The architecture provides: open-time measurement; timeout; technical logging; local blocklist; disabling an extension; an out-of-process helper if isolation is achievable; fallback to a minimal Ferrix menu.

Exact parity with Windows 10 and Windows 11 menus **is not guaranteed** (Microsoft does not expose all behaviors uniformly to third-party applications) — but a right-click must show the same entries as Explorer for common extensions (7-Zip, Git…).

### 13.9 Drag-and-drop (OPS-06, OPS-07)

Must work: between Ferrix folders; between panes; between windows; from Ferrix to Windows Explorer and vice versa; to and from OLE-compatible applications; to the sidebar; to the tabs; with copy/move/shortcut modifiers.

**A keyboard alternative must exist for every essential action.**

### 13.10 Search and indexing (SRC-01 → SRC-07)

**Search within the current folder** — no mandatory index; instant filter by name; filter on extension, size, and date; optional recursive search; immediate cancellation.

**Global search** — the index contains at least: name; path; extension; size; dates; volume identifier; file identifier when available; attributes; file type.

**Search syntax** — `name:report` ; `ext:pdf` ; `size:>100MB` ; `modified:this-week` ; `path:C:\Projects` ; `kind:image` ; `is:hidden` ; `is:directory`.

**Indexing** — 1) read the USN Journal on NTFS volumes when possible; 2) classic traversal (`jwalk`) as a privilege-free fallback; 3) listen to USN and filesystem changes; 4) periodic reconciliation; 5) transactional index updates (`tantivy`); 6) configurable path exclusions; 7) background resource throttling; 8) visible progress and control (pause/resume).

Indexing **must not require permanent elevation**. Without privileges, the application runs in a degraded mode with a clear message (SRC-05).

**Content search (SRC-06)** — optional and disableable: plain text; source code; PDF when extraction is available; Office documents via an allowed extractor; configurable maximum size; exclusions by type or location; nothing sent to a remote service.

### 13.11 Command palette (PWR-01)

Default shortcut: `Ctrl+Shift+P`. Allows: searching a command; opening a recent folder; changing view; toggling the theme; opening a terminal; computing a checksum; launching a batch rename; managing tabs; running a plugin action; showing the associated shortcut; everything executable without a mouse.

**The command list comes from a central registry shared with menus and shortcuts.**

### 13.12 Integrated terminal (PWR-02)

ConPTY when available. Functions: PowerShell; CMD; external Windows Terminal; WSL profiles; custom shell; opening in the current folder; detachable bottom pane; multiple sessions; resizing; copy/paste; ANSI colors; clickable links; search; configurable shortcuts.

The terminal **must not automatically send commands to an elevated shell**.

### 13.13 Companion CLI (PWR-03, EXT-04)

Binary `fx`:

```powershell
fx .
fx open C:\Projects
fx search "ext:rs path:C:\Projects"
fx workspace open development
fx checksum SHA256 .\archive.zip
fx reveal .\target\release\app.exe
```

The CLI must: talk to an existing instance when possible; start Ferrix if needed; return documented error codes; provide human-readable output and JSON output for automation.

### 13.14 Git (PWR-06)

v1.5 functions: repository detection; added, modified, deleted, ignored, and untracked badges; current branch; cache invalidation after modification (under 1 s); global or per-folder disabling; cost bounding on monorepos; `.gitignore` respect.

**Ferrix must not become a full Git client in the MVP.**

### 13.15 WSL (PWR-04)

Distribution discovery; navigation through the WSL paths exposed by Windows (`\\wsl$\`); opening in a WSL terminal; copying Windows and Linux paths; warning about potentially slow operations; correct case handling; symbolic link display; **no NTFS assumption on WSL files**.

### 13.16 Cloud (CLD-01 → CLD-06)

**Principles** — Ferrix does not reimplement OneDrive, Google Drive, or Dropbox synchronization when an official client is installed. Priorities: 1) detect local synchronized roots; 2) show placeholders; 3) show availability states; 4) use the official client's behavior; 5) request on-demand download through documented APIs; 6) offer optional remote integration later.

**Providers** — OneDrive; OneDrive for Business; Google Drive for desktop; Dropbox; Cloud Files API (CfApi)-compatible providers; other providers via plugins eventually.

**States** — locally available; always available; online-only; syncing; error; conflict; provider disconnected.

**Remote APIs (optional and separate)** — OAuth2 with system browser; PKCE; tokens stored in Windows Credential Manager; minimal permissions; session revocation; no embedded client secrets when possible; logging without tokens.

### 13.17 Network (NET-01, NET-02)

**MVP** — UNC paths; mapped network drives; automatic reconnection; timeout handling; offline status display; cancellation of slow requests.

**v1.5/v2** — SMB discovery (network neighborhood); SFTP; FTP/FTPS; WebDAV; connection profiles; secrets in Credential Manager; network favorites; local cache bounds; offline files (network cache).

**Network locations must never block application startup.**

### 13.18 Organization and analysis (ORG-01 → ORG-05, OPS-19, OPS-20, PRV-10)

**Tags (ORG-02)** — colored, multiple tags; storage by stable identifier when possible; path fallback; search by tag; export/import. Tags never silently modify the file or its ADS without an explicit option.

**Saved searches** — query; locations; filters; sort; view mode.

**Disk usage analysis (ORG-03)** — treemap; size ranking; grouping by extension; cancellable scan; exclusion of links and reparse points; clear logical/physical size distinction; report export.

**Duplicate finder (OPS-19)** — 1) group by size; 2) partial hash; 3) full hash; 4) optional final comparison. Functions: preview; assisted selection; hard links when relevant; move to Recycle Bin; **no automatic deletion by default**.

**ISO/VHD mounting (OPS-20)** — via Windows APIs when possible (passthrough = low effort).

**Archives as virtual folders (PRV-10)** — browse archives without extraction.

### 13.19 Properties and metadata (OPS-14, PRV-09)

Display: name; type; path; logical size; on-disk size; dates; attributes; owner; permissions; checksum; NTFS alternate data streams; reparse point; link target; EXIF information; media information; digital signature; binary version; cloud state.

Sensitive ACL operations are deferred to v2 (SYS-08).

### 13.20 UI and customization (UX-01 → UX-10)

**Themes** — light; dark; follow Windows (change in under 1 s); high contrast; accent colors; animation reduction; Mica/Acrylic when available; opaque fallback on Windows 10 or incompatible hardware.

**Customization** — compact/normal/comfortable density; columns; toolbar; icon size; sidebar; shortcuts; terminal application; view per folder type; single or double-click opening; startup behavior; icon packs (Community).

**Multi-monitor and DPI (A11Y-05, UX-05)** — restoration on the right screen; DPI change and mixed-DPI handling; reasonable position memory; automatic return to an available screen.

**System (UX-04, SYS-05, SYS-06, SYS-07, SYS-09)** — portable mode (specific marker, data next to the executable, neither registry nor `%APPDATA%`); session restore after crash; occasional UAC elevation without restarting the application; "default explorer" option if clean; Jump Lists; opt-in telemetry; Dev Drive (SYS-01); compressed/encrypted file indicators (SYS-02).

### 13.21 Accessibility (A11Y-01 → A11Y-07)

**Mandatory requirements** — UI Automation; Narrator; full keyboard navigation; coherent tab order; visible focus; accessible names/roles/states; folder-change announcements; operation and error announcements; high contrast; zoom; respect for reduced-motion preferences; drag-and-drop alternatives; indicators not based on color alone; verification with Accessibility Insights for Windows.

**Validation criteria** — all MVP features usable without a mouse; Narrator announces item name, type, and state; dialogs do not trap focus; virtualized lists stay accessible; accessibility tests are integrated into **every** UI tranche.

### 13.22 Keyboard shortcuts (PWR-05)

| Action | Shortcut | Action | Shortcut |
|---|---|---|---|
| New tab | `Ctrl+T` | Rename | `F2` |
| Close tab | `Ctrl+W` | Refresh | `F5` |
| Reopen tab | `Ctrl+Shift+T` | Delete | `Del` |
| Switch tab | `Ctrl+Tab` | Permanent delete | `Shift+Del` |
| New window | `Ctrl+N` | Properties | `Alt+Enter` |
| Address bar | `Ctrl+L` or `Alt+D` | Back | `Alt+←` |
| Search | `Ctrl+F` | Forward | `Alt+→` |
| Command palette | `Ctrl+Shift+P` | Parent | `Alt+↑` |
| Terminal | `` Ctrl+` `` | Select all | `Ctrl+A` |
| Quick preview | `Space` | Copy / Cut / Paste | `Ctrl+C` / `Ctrl+X` / `Ctrl+V` |
| Context menu | `Shift+F10` | Undo | `Ctrl+Z` |

Full Explorer parity (all standard shortcuts work) plus extensions. Shortcuts are **remappable** with conflict detection. An optional Vim mode may be added by the community (PWR-07).

---

## 14. Non-functional requirements

### 14.1 Performance and benchmarks

Measured targets: section 5.2. Rules:

- Never block the UI thread; virtualize every long list; load thumbnails only if visible or about to be visible; cancel obsolete tasks; LRU-bounded caches; measure before optimizing.
- Any regression greater than **10%** on a benchmark scenario blocks the merge (CI).
- Reference machine documented in `docs/benchmarks/REFERENCE.md` (Tranche 0).

**Benchmark scenarios**: 1,000 files; 10,000 files; 100,000 files; 1 million synthetic entries; image thumbnails; slow network folder; disconnected drive; large archive; global search; copying small files; copying a very large file.

**Comparison**: Ferrix vs `explorer.exe` vs other popular file managers, under the same conditions.

**Report**: machine; Windows version; storage; cache state; median; p95; number of repetitions. Published in `docs/benchmarks/` at every release.

### 14.2 Reliability

- No destructive operation without confirmation or going through the Recycle Bin (except an explicitly requested permanent deletion).
- Never confirm success before the system confirms it; interrupted operations (power loss, USB removal) leave consistent files; incomplete operations are reported.
- No crash on invalid paths, illegal names, denied permissions, full disks, locked files: errors are logged and presented to the user.
- Long operations are cancellable; detailed error reporting and recovery.
- Session restore after crash (SYS-05); repairable, rebuildable index.

### 14.3 Security

**Threat model** — malformed files; hostile thumbnails; malicious archives; unstable Shell extensions; compromised network paths; untrusted plugins; cloud tokens; directory traversal; reparse points; privilege escalation; DLL hijacking; injected terminal commands; decompression bombs (12.5).

**Measures** —

- out-of-process parsing for risky formats when possible; Job Objects and resource limits;
- canonical path verification; strict reparse point policy;
- OAuth tokens in Windows Credential Manager, never plaintext in config;
- WASM plugins with no default access: declared and requested permissions, no implicit sensitive permission; versioned IPC protocol;
- signed releases; no unsigned auto-update;
- `cargo audit` and `cargo deny` in CI; dependency security review; SBOM;
- no file contents, tokens, or sensitive paths in logs.

**Plugin security (v2)** — each plugin declares its capabilities: read a selection; read a file; write a file; use the network; add a command; add a preview provider; access the clipboard. No sensitive permission is granted implicitly; a plugin without network permission cannot use it.

### 14.4 Privacy and telemetry

- No mandatory telemetry; no account required; no file paths sent by default.
- Crash reporting **explicitly opt-in**; exact display of collected data; ability to export the report before sending; ability to disable everything.
- Searches and indexing are **local**.
- Documented in `docs/PRIVACY.md`.

### 14.5 Logs and diagnostics

Levels `error`/`warn`/`info`/`debug`/`trace` via `tracing`. Requirements: rotation and size limit (rotating file in `%LOCALAPPDATA%`); secret redaction; anonymized paths in exported reports; voluntary diagnostic bundle; separate log for indexing and operations; no file data read into logs.

### 14.6 Compatibility

**Systems (priority order)**: 1) Windows 11 x64; 2) Windows 10 22H2 x64; 3) Windows 11 ARM64; 4) Windows 10 ARM64 if reasonably supportable. Windows 11 features (Mica, Snap Layouts, Dev Drive) degrade cleanly on Windows 10.

**File systems**: NTFS; ReFS; exFAT; FAT32; SMB shares; WSL-exposed file systems; cloud file placeholders.

Not every feature is available on every file system: **the UI shows the limits instead of failing silently.**

### 14.7 Internationalization

Strings externalized from Tranche 0 (resource files); FR and EN shipped in 1.0; consistent tables and messages; code and comments remain in English.

### 14.8 Maintainability

- High test coverage on `fx-domain`, `fx-fs`, `fx-ops`, `fx-index`, `fx-codec`, `fx-preview`.
- Rustdoc on all public APIs; README in each crate.
- ADR for every structural decision; `docs/LEGAL.md` updated on every dependency.
- User documentation in `docs/user/`.

---

## 15. Tests, validation, and CI

### 15.1 Unit tests

Path normalization; name conflicts; batch rename; search syntax; operation computation; selection rules; SQLite migrations; codec selection; archive validation; plugin permissions.

### 15.2 Integration tests

Temporary filesystem; copy/move; Recycle Bin; links and reparse points; long paths; Unicode; locked files; removable volumes; SQLite; index; archives.

### 15.3 Real-Windows tests

Context menus; drag-and-drop; properties; COM; Cloud Files API; ConPTY; Narrator; multi-DPI; dark theme; high contrast; Snap Layouts. COM is hard to simulate: **documented manual tests** for `fx-shell`.

### 15.4 Robustness tests

Parser fuzzing; malformed archives; invalid paths; network cuts; removed drives; hung Shell processes; full disks; denied permissions; crash during migration; crash during copy.

### 15.5 CI

**On every pull request**: `cargo fmt --check`; `cargo clippy --all-targets --all-features -- -D warnings`; unit tests; CI-compatible integration tests; `cargo audit`; license checking (`cargo deny`); Windows x64 build; non-regression benchmarks when a hot path was touched.

**Periodically**: ARM64; E2E tests; full benchmarks; fuzzing; packaging.

---

## 16. Roadmap in tranches

> Each tranche is a deliverable, testable version. A tranche does not start until the previous tranche's acceptance criteria are validated. **Accessibility (UIA) and performance are cross-cutting**: verified at every tranche, not only at the end (decision D-3).

### 16.1 Overview

| # | Tranche | Key content | Priority |
|---|---|---|---|
| 0 | Foundations and technical validation | Multi-crate workspace, CI, logging, `fx-a11y` skeleton, UI/COM/UIA spikes, UI decision | MVP |
| 1 | Basic navigation | Async listing, views, breadcrumb, history, filtering, long paths, virtualization | MVP |
| 2 | File operations | Copy/cut/paste, rename, delete, create, selection, undo/redo, checksums, activity center | MVP |
| 3 | Native Shell | `IContextMenu`, `IFileOperation`, properties, "Open with", OLE drag-and-drop, thumbnails | MVP |
| 4 | Sidebar and drives | Pins, quick access, recent, libraries, This PC, hotplug, WSL, Dev Drive, tree view | MVP |
| 5 | Tabs, windows, dual-pane | Tabs, multi-window, split view, Commander mode, workspaces, session restore | MVP |
| 6 | Global search | USN/MFT, tantivy, live updates, syntax, filters, fallback, optional full-text | MVP |
| 7 | Power-user | Palette, ConPTY terminal, `fx` CLI, WSL, regex rename, shortcuts and remapping | MVP |
| 8 | Quick Look and compression | `fx-preview`, `fx-codec`, full previews, archives (ZIP, 7z, TAR…), extraction security | MVP |
| 9 | Windows 11 customization | Themes, Mica, Snap Layouts, zoom, views, columns, density, portable mode, DPI, multi-monitor | MVP |
| 10 | Accessibility and i18n | Full UIA, high contrast, Narrator/NVDA audit, FR/EN | MVP |
| 11 | Cloud | OneDrive (CfApi), Google Drive, Dropbox, quotas, OAuth2 | v1.5 |
| 12 | Advanced tools | Duplicates, treemap, operation queue, integrity, tags, EXIF, virtual archives, ISO/VHD | v1.5 |
| 13 | Dev and system | Git badges, Jump Lists, crash recovery, UAC, default explorer, opt-in telemetry | v1.5 |
| 14 | Network | SMB (discovery), FTP/SFTP/WebDAV | v1.5 / v2 |
| 15 | Performance | 1 M-file benchmarks, profiling, GPU cache, cold start, memory leaks | Cross-cutting |
| 16 | Extensibility | WASM plugins, Rhai, associations, diff, smart folders, secure delete, versions, ACL | v2 |
| 17 | Packaging | MSI/MSIX, signing, signed updates, SBOM, portable version | MVP (release) |
| 18 | Release 1.0 | User documentation, contributor guide, public beta, announcement | MVP (release) |

### 16.2 Delivery milestones

- **Technical prototype** — tranches 0 to 1: prove UI, accessibility, and performant navigation feasibility.
- **Alpha** — tranches 2 to 5: a daily-driver local explorer (operations, sidebar, tabs, Shell).
- **Beta** — tranches 6 to 10: search, productivity, Quick Look, archives, customization, accessibility.
- **Version 1.0** — stabilized MVP tranches (11, 17, 18), packaging, and documentation. Advanced cloud, specialized tools, and plugins may ship progressively if their stability excessively delays 1.0.

### 16.3 Tranche details

#### Tranche 0 — Foundations and technical validation

**Objective**: functional workspace, empty window, green CI, UI decision made, high technical risks exercised.

**Content** — multi-crate workspace (10.1); `rust-toolchain.toml`; `CLAUDE.md`; Slint **and** iced prototypes (100,000-row list); UI Automation, COM, drag-and-drop, and `IContextMenu` prototypes; main window (title, icon, remembered size); `fx-a11y` skeleton (root element: name + UIA role); rotating `tracing` logging; GitHub Actions CI (build, tests, `clippy -D warnings`, `fmt`, `cargo deny`, `cargo audit`); `docs/LEGAL.md`, `docs/benchmarks/REFERENCE.md`, and `docs/adr/0001-ui-framework.md`; strings externalized from the start.

**Acceptance criteria** — `cargo run` opens a window on Windows 10 and 11; reproducible build; green CI; Narrator announces the window name; a 100,000-row list stays interactive; UI decision recorded in the ADR.

#### Tranche 1 — Basic navigation

**Objective**: browse the filesystem read-only, fluidly.

**Content** — `fx-fs`/`fx-domain`: `FileEntry`, `Location` abstraction, async batch-streamed listing (`jwalk`), cancellation, folder watching (`notify`); list and details views, simple sorting, virtualization; double-click (navigate / `ShellExecute`); editable address bar, breadcrumb, back/forward history (including mouse buttons); instant filtering; long paths `\\?\` and Unicode; system icons (`SHGetFileInfo`); basic keyboard navigation and UIA roles on the list and columns.

**Acceptance criteria** — full navigation in `C:\` and Known Folders, back/forward working; no UI blocking during listing; 50,000-file folder: first items under 100 ms, 60 FPS scrolling; a path over 260 characters readable and navigable; correct Unicode; Narrator announces items and columns; navigation tests green.

#### Tranche 2 — File operations

**Objective**: complete CRUD with safety (Recycle Bin, undo).

**Content** — copy/cut/paste (Windows clipboard, `CF_HDROP`, `Preferred DropEffect`); rename (F2), create folder and file; delete to Recycle Bin, explicit permanent delete (`Shift+Del`); multiple and rectangular selection; internal drag-and-drop; conflict dialog; **activity center** (progress, speed, errors); undo/redo (restore from Recycle Bin); checksums (MD5, SHA-256, BLAKE3); empty-folder cleaner; enriched status bar.

**Acceptance criteria** — all operations work with visible progress feedback; deleted files found in the Windows Recycle Bin; `Ctrl+Z` undoes the last reversible operation; conflicts offer the four choices; **no false success confirmation**.

#### Tranche 3 — Native Shell

**Objective**: behavior identical to Explorer where expected.

**Content** — native `IContextMenu` (merged entries, third-party extensions) plus a Ferrix menu; `IFileOperation` with the native progress dialog; native properties; "Open with"; `IThumbnailProvider` thumbnails with disk cache; OLE drag-and-drop both ways; open-time measurement and fallback; keyboard-accessible menu (`Shift+F10`).

**Acceptance criteria** — right-click shows the same entries as Explorer (7-Zip, Git if installed); a file dragged to the Desktop or Explorer is copied correctly, and vice versa; a 2 GB file copy shows the native progress dialog; the menu is keyboard-usable; an extension crash does not corrupt the session more than technically possible.

#### Tranche 4 — Sidebar and drives

**Objective**: navigation through the complete sidebar.

**Content** — pins (drag-and-drop, SQLite persistence); quick access, recent, libraries (KnownFolders); This PC with usage gauge; mapped network drives and UNC; USB hotplug; tree view; Dev Drive; WSL; unavailable locations shown without blocking.

**Acceptance criteria** — pins persist and can be reordered; a USB key appears without restart and disappears cleanly on removal; usage gauges match Explorer within 1%; WSL accessible when installed; no UI blocking on unavailable locations.

#### Tranche 5 — Tabs, windows, dual-pane

**Objective**: multi-context productivity.

**Content** — tabs (`Ctrl+T`, `Ctrl+W`, `Ctrl+Tab`, reordering, detaching); independent windows; split view and Commander dual-pane mode (copy to the opposite pane, optional synchronized navigation); saved/restorable workspaces; tab restore on startup (full crash recovery is in Tranche 13).

**Acceptance criteria** — tab management comparable to a web browser (smooth reordering, detaching without state loss); a named workspace restores identically; independent panes; unambiguous keyboard focus.

#### Tranche 6 — Global search

**Objective**: Everything-style instant search.

**Content** — MFT and USN Journal reading on all volumes; `tantivy` index (name, path, size, dates, attributes, type); live updates; periodic reconciliation; configurable exclusions; `jwalk` fallback without privileges (clear message); results UI with highlighting; search syntax (13.10); advanced filters; optional full-text (off by default); indexing progress and control.

**Acceptance criteria** — initial indexing of a 1 M-file volume in under 60 s; name search under 50 ms (p95) after indexing; creation/deletion visible within 2 s; privilege-free mode works with a clear message; UI always interactive; repairable index; **no permanent elevation required**.

#### Tranche 7 — Power-user

**Objective**: keyboard productivity and developer tooling.

**Content** — command palette (`Ctrl+Shift+P`) wired to a central registry shared with menus/shortcuts; ConPTY terminal in the current folder (PowerShell, cmd, WSL); `fx` CLI (text and JSON); WSL support (`\\wsl$\`); batch regex rename with preview; keyboard shortcuts: full Explorer parity + remapping.

**Acceptance criteria** — all standard Explorer shortcuts work; terminal opens in the right folder in under 500 ms; `fx open .` opens a window on the current folder; rename previewed and cancellable; all main actions accessible through the palette.

#### Tranche 8 — Quick Look and compression

**Objective**: safe instant preview and archives.

**Content** — `fx-preview`: providers for images, RAW, PDF, video, audio, text/code, archives (listing), fonts, Office (Shell); spacebar preview, keyboard navigation, fullscreen, cancellable async rendering, L1 cache, risky-format isolation; `fx-codec`: backend registry (ZIP/libdeflater, 7z/sevenz-rust, TAR/GZIP/XZ/BZIP2, internal zstd), optional backends through the registry; creation and extraction; RAR reading via `rar` feature; secure extraction (12.5); progress, passwords; backend benchmarks.

**Acceptance criteria** — standard image previewed in under 100 ms; first PDF page under 300 ms; changing the item cancels the previous render without resource leaks; a malformed file does not take the main app down; memory limits tested; the application compiles and runs **without** any optional backend; LZTurbo is neither included, downloaded, nor referenced by default; **protected against Zip Slip and decompression bombs**; no restrictive dependency in the default build; benchmark results published.

#### Tranche 9 — Windows 11 customization

**Objective**: native look and visual settings.

**Content** — light/dark themes synced with Windows + override; Mica with Windows 10 fallback; Snap Layouts; icons/tiles/content views; `Ctrl+wheel` zoom; density; customizable columns and toolbar; compressed/encrypted indicators; portable mode; per-screen geometry; system DPI and text size (mixed DPI).

**Acceptance criteria** — theme follows Windows dark mode in under 1 s and without restart; Mica visible on Windows 11, clean fallback on Windows 10; a portable copy changes neither the registry nor `%APPDATA%`; the window finds its position on the right screen after unplugging a monitor; mixed DPI works; settings persist.

#### Tranche 10 — Accessibility and internationalization

**Objective**: usable without a mouse and with a screen reader.

**Content** — full UI Automation (virtualized lists, sidebar, tabs, palette, dialogs, preview); announcements (folder, results, operations, errors); high contrast; keyboard audit without focus traps; FR/EN translation; documented Narrator + NVDA + Accessibility Insights audit.

**Acceptance criteria** — a complete journey (navigate, copy, rename, search, open preview) achievable by keyboard and by ear with NVDA; no MVP feature mouse-only; no control without an accessible name (automated UIA check); no critical defect in Accessibility Insights; high-contrast mode respects system colors.

#### Tranche 11 — Cloud (v1.5)

**Content** — `CloudProvider` trait (list, status, quota, sync_state); OneDrive (roots, placeholders, CfApi badges); Google Drive and Dropbox (local client detection, API fallback); "Cloud accounts" sidebar section, quotas; OAuth2 + PKCE via browser; on-demand hydration/release.

**Acceptance criteria** — badges consistent with the official client; an "online-only" file downloadable on demand; OAuth tokens in the Credential Manager; no reimplementation of the synchronization engine.

#### Tranche 12 — Advanced tools (v1.5)

**Content** — operation queue (pause, resume, reorder, cancel); post-copy integrity verification; staged duplicate finder; disk usage treemap; colored tags; EXIF panel; archives as virtual folders; ISO/VHD/VHDX mounting.

**Acceptance criteria** — the duplicate finder never reads a whole file if size or partial hash rules it out; **no automatic deletion**; the treemap of a 500 GB disk builds without blocking the UI; cancellable scans; exportable reports.

#### Tranche 13 — Dev and system (v1.5)

**Content** — Git badges (modified, added, ignored…) with cache and invalidation; Jump Lists; session restore after crash; occasional UAC elevation; "default explorer" if clean; opt-in telemetry and anonymous crash reports; improved WSL (copy Windows/Linux paths); terminal profiles.

**Acceptance criteria** — Git badges updated within 1 s after `git add`; after a crash, the session is restored on restart; large repositories usable; features disableable.

#### Tranche 14 — Network (v1.5 / v2)

**Content** — SMB discovery (network neighborhood); FTP, FTPS, SFTP, and WebDAV clients browsable like a local folder; connection profiles; secrets in Credential Manager; network favorites; timeouts and recovery.

**Acceptance criteria** — a discovered SMB share appears in the sidebar; an SFTP connection allows copy, rename, and delete; **no network wait on the UI thread**; secrets protected; network errors understandable.

#### Tranche 15 — Performance (cross-cutting)

**Content** — automated benchmarks (1k, 10k, 100k, 1M) compared with `explorer.exe`; profiling (`tracing`, Windows Performance Analyzer); GPU cache for icons and thumbnails (texture atlas); cold start reduction; memory leak detection; stabilization and fuzzing.

**Acceptance criteria** — a reproducible benchmark report published in `docs/benchmarks/` for every release; targets in 5.2 met or a documented gap accepted; no significant leak; regressions blocked in CI when possible.

#### Tranche 16 — Extensibility and advanced features (v2)

**Content** — WASM plugins (`fx-plugin-api`/`fx-plugin-host`) with versioned API and declared permissions; SDK and examples; Rhai scripting; file association manager; folder comparison (diff); smart folders; secure deletion; transfer resumption; version restore; ACL editor.

**Acceptance criteria** — an example plugin adds a palette action without recompiling Ferrix; a plugin without network permission cannot use it; plugin crashes isolated; permissions visible; documented API.

#### Tranche 17 — Packaging and distribution

**Content** — MSI (`cargo-wix`) and MSIX; portable ZIP version; code signing; signed updates via GitHub Releases (rollback, changelog, SHA-256); stable/beta/nightly channels; SBOM; generated metadata, icon, release notes.

**Acceptance criteria** — a user with no Rust knowledge installs and launches Ferrix in under two minutes; clean uninstall; upgrade between versions without losing configuration; signed and verified artifacts; SBOM published.

#### Tranche 18 — Release 1.0

**Content** — user documentation (installation, shortcuts, FAQ); `CONTRIBUTING.md`, `SECURITY.md`, architecture guide; "good first issue" list (backlog `Community`); public beta (FR/EN); announcement (GitHub, Reddit r/rust, r/Windows10, Hacker News).

**Acceptance criteria** — beta tested by at least 20 users on varied configurations; no open blocking bug, major bugs fixed; release benchmarks published; no known critical vulnerability; validated on Windows 10 and 11; complete documentation.

---

## 17. Technical risks

| Risk | Probability | Impact | Mitigation |
|---|---|---|---|
| Slint license incompatible with the target license | High | High | Decision in Tranche 0 (Slint and iced prototypes), documented ADR (9.1) |
| UI framework insufficient (virtualization, UIA) | Medium | Very high | Spike and ADR before mass development (section 11) |
| Accessibility completed too late | High | Very high | Start in Tranche 0, continuous tests, real Narrator/NVDA audits (D-3) |
| Complexity of Shell COM interop (`IContextMenu`, `IFileOperation`) | High | High | Dedicated time in Tranche 3, isolated prototypes in `fx-shell`, documented manual tests |
| Third-party Shell extensions slow or unstable | High | High | Timeouts, open-time measurement, blocking, isolation, fallback (13.8) |
| Corruption during operations | Medium | Critical | System APIs (`IFileOperation`), transactions, journals, robustness tests (15.4) |
| USN Journal requires admin privileges on some volumes | Medium | Medium | `jwalk` fallback (SRC-05), clear message to the user |
| Corrupted index | Medium | Medium | Rebuild, versioning, backup before migration |
| CfApi poorly documented in Rust | High | Medium | Manual wrapper in `fx-cloud`, tests on real machines with OneDrive |
| Google Drive and Dropbox change local clients or APIs | Medium | Medium | `CloudProvider` abstraction, API fallback, regression tests |
| LZTurbo license incompatible with OSS | Certain | Low (if respected) | Never included by default, opt-in plugin, `docs/LEGAL.md` (9.2) |
| Restrictive UnRAR license | Certain | Low | Read-only, `rar` feature, disableable (9.3) |
| UI Automation hard to get right | High | High | Start in Tranche 0, real Narrator/NVDA tests, not just a checkbox |
| Indexing on very large volumes (> 5 M files) | Medium | Medium | Incremental indexing, visible progress, never blocking |
| Codec licenses | High | High | Audit, optional components, SBOM, `cargo deny` in CI |
| Scope too large | High | Critical | Strict tranches and acceptance criteria, assumed v1.5/v2 backlog |
| "Pure Rust" positioning unrealistic | Medium | Medium | Transparent Rust-first (D-7) |
| Unpredictable network performance | High | High | Timeouts, cancellation, async, no UI blocking (13.17) |
| Incomplete ARM64 | Medium | Medium | Periodic ARM64 CI, dependency inventory |
| Windows updates changing Shell behavior | Medium | Medium | Test matrix on recent Windows builds, `windows-latest` CI |
| Maintenance of a RAW crate or inactive binding | Medium | Low | Choose active crates, abstraction via the `PreviewProvider` trait |
| Underestimated workload (very large project) | High | High | Tranche breakdown, strict MVP, assumed v1.5/v2 backlog |

---

## 18. Development conventions and definition of done

### 18.1 Definition of done (DoD) per tranche

- [ ] Tranche acceptance criteria validated (automated or documented manual tests).
- [ ] `cargo fmt`, `cargo clippy --all-targets --all-features -- -D warnings`, tests green in CI.
- [ ] Unit tests on business logic (`fx-domain`, `fx-fs`, `fx-ops`, `fx-index`, `fx-codec`, `fx-preview`).
- [ ] Long operations cancellable; no UI thread blocking; failure scenarios tested.
- [ ] UIA check: every new control has a name and a role, tested with Narrator at minimum.
- [ ] Keyboard check: no new action is mouse-only.
- [ ] Logs contain no secret or file content; paths anonymized in exported reports.
- [ ] Non-regression benchmarks run when a hot path was touched.
- [ ] Strings added to translation files; user documentation updated.
- [ ] `docs/LEGAL.md` updated if a dependency was added; license validated.
- [ ] ADR created for any structural decision; limitations documented.
- [ ] Manual Windows validation checklist produced; changelog updated.

### 18.2 Code and quality

- Unit tests wherever possible; documented manual integration tests for `fx-shell` (COM is hard to simulate).
- `unsafe` forbidden by default: allowed only in interop crates, with a `// SAFETY:` comment and a targeted test.
- No `unwrap()` or `expect()` outside tests and initialization (documented invariant accepted); typed errors (`thiserror`) in libraries, `anyhow` only in binaries, with context.
- No blocking of the UI loop: all I/O goes through the runtime or a pool.
- Never claim a Windows API is supported without validation on real Windows.

### 18.3 Git

- Conventional Commits: `feat:`, `fix:`, `perf:`, `a11y:`, `docs:`, `refactor:`, `test:`, `build:`, `ci:`, `security:`.
- One tranche = one branch = one or more PRs, never mixing tranches; mandatory review before merging to `main`.
- Small, coherent commits.

### 18.4 Documentation

- README in each crate, Rustdoc on all public APIs.
- Numbered ADRs in `docs/adr/`.
- User documentation in `docs/user/`; `ARCHITECTURE.md`, `SECURITY.md`, `PRIVACY.md`, `CONTRIBUTING.md`, `BENCHMARKS.md`.

### 18.5 Licences

- Project license: MIT OR Apache-2.0 (subject to decision 9.1).
- Every non-permissive dependency listed in `docs/LEGAL.md` with its status: **default**, **optional**, **isolated**, or **opt-in**.
- `cargo deny` blocks any unauthorized license in CI without a documented exception.

### 18.6 Manual validation process

Each tranche ends with a manual validation checklist on real Windows (at minimum: one Windows 10 22H2 machine and one Windows 11 machine), listing the scenarios to replay and what remains to be tested manually. An AI agent's work follows the same checklist (section 20).

---

## 19. Packaging, licenses, and metrics

### 19.1 Packaging and updates

**Formats** — MSI (user); MSIX if restrictions are acceptable; portable ZIP (portable-mode marker, settings stored locally).

**Functions** — per-user installation; optional machine-wide installation; clean uninstall; optional file association; signed update with rollback; stable, beta, and nightly channels; changelog; SHA-256 sums; artifact signatures; **never an unsigned auto-update**.

**Metadata** — generated icon, manifest, and release notes.

### 19.2 Licenses and dependencies

The project maintains `LICENSE-MIT`, `LICENSE-APACHE`, `THIRD_PARTY_NOTICES.md`, `docs/LEGAL.md`, and an **SBOM per release**. Each dependency is examined for: license, project activity, vulnerabilities, native code, ARM64 compatibility, size, update policy, actual necessity. Sensitive components (LZTurbo, RAR, FFmpeg, PDFium, SDKs) require an ADR and legal review before distribution (section 9).

### 19.3 Success metrics

**Product** — crash rate; startup time; navigation latency; search latency; operation error count; memory usage; restore success rate; active users (only if opt-in measurement); tester satisfaction.

**Quality** — coverage of critical modules; open vulnerabilities; accessibility defects; performance regressions; mean time to resolution; percentage of up-to-date dependencies.

---

## 20. Usage with an AI agent (Claude Code and equivalents)

### 20.1 `CLAUDE.md` file at the repository root (to create in Tranche 0)

```markdown
# CLAUDE.md — Rules for the agent

## Context
- Project: Ferrix Explorer, a Windows 10/11 file explorer in Rust.
- Source of truth: PRD.md (this document) and docs/adr/.

## Working rules
1. Always read the PRD and the current tranche before coding.
2. Propose the file and module structure first, then wait for validation if the change is structural.
3. One tranche = one branch. Never implement a feature from a later tranche.
4. No dependency added without checking its license and updating docs/LEGAL.md.
5. No non-permissive backend enabled by default.
6. Every `unsafe` has a // SAFETY: comment and a test, and stays in interop crates.
7. COM and low-level Win32 stay in fx-shell, fx-terminal, fx-git, fx-a11y.
8. fx-domain, fx-fs and fx-ops never depend on fx-ui.
9. No blocking I/O on the UI thread; every long task is cancellable.
10. Every UI control has an accessible name and role.
11. Run cargo fmt, cargo clippy -D warnings and cargo test before every commit.
12. Use Conventional Commits.
13. Never claim a Windows API is supported without validation on real Windows.
14. Do not start the next tranche without validating the acceptance criteria.
```

### 20.2 Working method per tranche

For each tranche: 1) read the PRD and the ADRs in full; 2) inspect the actual state of the repository and summarize it; 3) list the tranche's requirements and acceptance criteria (backlog IDs); 4) identify technical risks and dependencies (with licenses); 5) propose the module structure and an implementation plan; 6) flag risks before coding; 7) create or update the required ADRs; 8) implement in small coherent commits with tests; 9) run `cargo fmt`, `cargo clippy`, `cargo test`; 10) document limitations; 11) produce a manual Windows validation checklist; 12) do not start the next tranche without validation.

### 20.3 Template prompt per tranche

```text
You are working on Ferrix Explorer, a Rust-first Windows file explorer.

Read PRD.md and the ADRs of the repository in full. We are at Tranche [NUMBER AND NAME].

Before touching any code:
1. inspect the workspace and summarize its state;
2. list the tranche's requirements (backlog IDs and acceptance criteria);
3. identify technical risks and dependencies with their licenses;
4. propose the module structure and an implementation plan;
5. state the verifiable acceptance criteria.

Then implement the tranche in atomic steps with tests.

Constraints:
- never block the UI thread; use cancellation for long tasks;
- preserve accessibility (name/role on every control);
- limit and document any unsafe code;
- add no dependency without checking license and maintenance (docs/LEGAL.md);
- run cargo fmt, cargo clippy and cargo test;
- never claim a Windows API is supported without validation;
- do not start the next tranche;
- document limitations and the manual Windows test procedure.

At the end, provide: a summary of changes; modified files; validation commands;
test results; remaining limitations; the acceptance checklist.
```

### 20.4 Recommended starting prompts

- **Tranche 0**: "Create the multi-crate workspace of section 10.1, set up CI, and open an empty window with a root UI Automation element. Prepare both UI prototypes (Slint and iced) and the COM/UIA/DnD spikes for comparison, without deciding the UI."
- **Tranche 1**: "Implement async listing in `fx-fs` and the virtualized list view in `fx-ui`. Goal: 50,000 fluid entries. Add tests and a benchmark."

---

## 21. Glossary

| Term | Definition |
|---|---|
| **USN Journal** | Change journal of an NTFS volume, readable for incremental indexing. |
| **MFT** | Master File Table, NTFS file table enabling fast enumeration. |
| **CfApi** | Cloud Files API of Windows, used by synchronization clients for online-only files. |
| **ConPTY** | Windows pseudo-terminal API, base of the integrated terminal. |
| **UIA** | UI Automation, Windows accessibility API used by screen readers. |
| **Quick Look** | Fast preview of a file without opening it, via the spacebar. |
| **Dual-pane** | Two side-by-side panes mode, Commander-style. |
| **Optional backend** | Compression or preview engine installed and explicitly enabled by the user. |
| **Placeholder / hydration** | Cloud file represented locally by a virtual location, downloaded on demand. |
| **Reparse point** | NTFS redirection point (links, mounts, placeholders); a risk in recursive traversal. |
| **Acceptable floor** | Minimum tolerated performance value (see 5.2); beyond it, the gap must be documented and accepted. |
| **Tranche** | Deliverable, testable project version; conditioned on validating the previous tranche. |

---

## Annex A — Functional backlog (traceability index)

**Legend**

- **Impact**: 1 (low) to 5 (critical).
- **Effort**: 1 (trivial) to 5 (very complex).
- **Priority**: `MVP` (1.0 release), `v1.5` (fast post-launch), `v2` (long-term vision), `Community` (ticket for external contributors).
- **Tranche**: tranche number from section 16. Each ID's detailed requirements are in section 13.

### A.1 Navigation (NAV)

| ID | Feature | Impact | Effort | Priority | Tranche |
|---|---|---|---|---|---|
| NAV-01 | Asynchronous folder listing (jwalk) | 5 | 2 | MVP | 1 |
| NAV-02 | List and details views (name, size, date, type columns) | 5 | 2 | MVP | 1 |
| NAV-03 | Icon views (small, medium, large, extra large), tiles, content | 4 | 3 | MVP | 9 |
| NAV-04 | Clickable breadcrumb with subfolder dropdown menu | 5 | 2 | MVP | 1 |
| NAV-05 | Editable address bar (direct path entry, autocomplete) | 4 | 1 | MVP | 1 |
| NAV-06 | Back/forward history (Alt+←/→, mouse side buttons) | 4 | 1 | MVP | 1 |
| NAV-07 | Folder tree view | 3 | 2 | MVP | 4 |
| NAV-08 | Instant filtering (real-time filtered list) | 4 | 2 | MVP | 1 |
| NAV-09 | Multi-criteria sorting and grouping (type, date, size) | 4 | 2 | MVP | 1 and 9 |
| NAV-10 | Ctrl+wheel zoom (icon size) | 4 | 2 | MVP | 9 |
| NAV-11 | List virtualization (render only visible items) | 5 | 3 | MVP | 1 |
| NAV-12 | Long paths (> 260 characters, `\\?\` prefix) | 4 | 2 | MVP | 1 |
| NAV-13 | Multi-tab (Ctrl+T, Ctrl+W, Ctrl+Tab, reordering, detaching) | 5 | 3 | MVP | 5 |
| NAV-14 | Independent multi-window | 4 | 2 | MVP | 5 |
| NAV-15 | Split view (two panes in one tab) | 4 | 3 | MVP | 5 |
| NAV-16 | Commander-style dual-pane mode (copy to opposite pane, optional synchronized navigation) | 4 | 3 | MVP | 5 |

### A.2 Sidebar (SID)

| ID | Feature | Impact | Effort | Priority | Tranche |
|---|---|---|---|---|---|
| SID-01 | Pins (add, remove, drag-and-drop reordering) | 4 | 2 | MVP | 4 |
| SID-02 | Quick access (frequent folders, dynamic calculation) | 3 | 2 | MVP | 4 |
| SID-03 | Recent (files and folders, stored in SQLite) | 4 | 2 | MVP | 4 |
| SID-04 | Libraries (Documents, Pictures, Music, Videos, Downloads via KnownFolders) | 4 | 1 | MVP | 4 |
| SID-05 | This PC: local drives with free/used space gauge | 4 | 2 | MVP | 4 |
| SID-06 | Mapped network drives (automatic reconnection) | 3 | 2 | MVP | 4 |
| SID-07 | Removable drive detection (USB hotplug) | 3 | 2 | MVP | 4 |
| SID-08 | Saved workspaces (named sets of tabs and panes) | 3 | 2 | MVP | 5 |
| SID-09 | "Cloud accounts" section with status and quota | 4 | 3 | v1.5 | 11 |
| SID-10 | SMB network discovery (network neighborhood) | 3 | 3 | v1.5 | 14 |

### A.3 File operations (OPS)

| ID | Feature | Impact | Effort | Priority | Tranche |
|---|---|---|---|---|---|
| OPS-01 | Copy, cut, paste (Windows clipboard, CF_HDROP and Preferred DropEffect formats) | 5 | 3 | MVP | 2 |
| OPS-02 | Inline rename (F2) | 5 | 2 | MVP | 2 |
| OPS-03 | Batch rename with regex (preview before applying) | 4 | 2 | MVP | 7 |
| OPS-04 | Delete to Recycle Bin, and permanent delete (Shift+Del) | 5 | 2 | MVP | 2 |
| OPS-05 | Undo / redo of the application's operations | 4 | 3 | MVP | 2 |
| OPS-06 | Internal drag-and-drop (move, copy) | 5 | 3 | MVP | 2 |
| OPS-07 | OLE drag-and-drop to and from other applications | 4 | 4 | MVP | 3 |
| OPS-08 | Operations via IFileOperation (native progress dialog) | 4 | 3 | MVP | 3 |
| OPS-09 | Conflict dialog (replace, keep both, skip, cancel) | 4 | 2 | MVP | 2 |
| OPS-10 | Create folder and file (Ctrl+Shift+N) | 4 | 1 | MVP | 2 |
| OPS-11 | Multiple selection (Ctrl/Shift+click, rectangular selection) | 5 | 2 | MVP | 2 |
| OPS-12 | MD5, SHA-256, BLAKE3 checksums (menu and panel) | 3 | 1 | MVP | 2 |
| OPS-13 | Empty folder cleaner | 2 | 1 | MVP | 2 |
| OPS-14 | Native properties (ShellExecuteEx, `properties` verb) | 4 | 1 | MVP | 3 |
| OPS-15 | Native "Open with" | 3 | 1 | MVP | 3 |
| OPS-16 | Compression and decompression (ZIP, 7z, TAR…) via `fx-codec` | 4 | 3 | MVP | 8 |
| OPS-17 | Operation queue (pause, resume, reorder, cancel) | 4 | 3 | v1.5 | 12 |
| OPS-18 | Post-copy integrity verification (source/destination hash) | 3 | 2 | v1.5 | 12 |
| OPS-19 | Duplicate finder (by hash, not only by name) | 4 | 3 | v1.5 | 12 |
| OPS-20 | Mount ISO and VHD/VHDX images | 2 | 1 | v1.5 | 12 |
| OPS-21 | Resume interrupted transfers | 3 | 4 | v2 | 16 |
| OPS-22 | Secure deletion (multi-pass overwrite) | 2 | 2 | v2 | 16 |
| OPS-23 | Previous versions restore (Volume Shadow Copy / File History) | 3 | 3 | v2 | 16 |
| OPS-24 | File and folder comparison (diff, visual synchronization) | 3 | 3 | v2 | 16 |

### A.4 Quick Look and previews (PRV)

| ID | Feature | Impact | Effort | Priority | Tranche |
|---|---|---|---|---|---|
| PRV-01 | Instant spacebar preview (frame, navigation, fullscreen) | 5 | 4 | MVP | 8 |
| PRV-02 | Images (common formats) and RAW (embedded thumbnail first) | 4 | 3 | MVP | 8 |
| PRV-03 | PDF (pages, zoom, navigation) | 4 | 3 | MVP | 8 |
| PRV-04 | Video and audio: thumbnail, playback, waveform | 4 | 4 | MVP | 8 |
| PRV-05 | Source code and text with syntax highlighting | 3 | 2 | MVP | 8 |
| PRV-06 | Archive listing (without extraction) | 4 | 2 | MVP | 8 |
| PRV-07 | Fonts (preview with sample text, DirectWrite) | 2 | 2 | MVP | 8 |
| PRV-08 | Office documents via Shell Preview Handlers | 3 | 2 | MVP | 8 |
| PRV-09 | EXIF and media metadata panel | 3 | 2 | v1.5 | 12 |
| PRV-10 | Browsing archives as a virtual folder (without extraction) | 4 | 2 | v1.5 | 12 |

### A.5 Organization (ORG)

| ID | Feature | Impact | Effort | Priority | Tranche |
|---|---|---|---|---|---|
| ORG-01 | Enriched status bar (selection, total size, free space) | 3 | 1 | MVP | 2 |
| ORG-02 | Colored tags independent of the folder tree | 3 | 3 | v1.5 | 12 |
| ORG-03 | Disk usage treemap (built-in visualization) | 4 | 3 | v1.5 | 12 |
| ORG-04 | Virtual folders (smart folders, criteria-based) | 3 | 3 | v2 | 16 |
| ORG-05 | Automatic organization rules (Hazel-style) | 2 | 3 | Community | — |

### A.6 Search (SRC)

| ID | Feature | Impact | Effort | Priority | Tranche |
|---|---|---|---|---|---|
| SRC-01 | Global search bar (Ctrl+F and direct typing) | 5 | 3 | MVP | 6 |
| SRC-02 | USN Journal and NTFS MFT reading for initial indexing | 5 | 4 | MVP | 6 |
| SRC-03 | tantivy index with live updates (USN) | 5 | 4 | MVP | 6 |
| SRC-04 | Advanced filters (type, size, date, attributes) | 4 | 2 | MVP | 6 |
| SRC-05 | `jwalk` traversal fallback when the volume is inaccessible without privileges | 4 | 2 | MVP | 6 |
| SRC-06 | Full-text search in content (text, code, extracted PDF) | 3 | 4 | MVP (optional, off by default) | 6 |
| SRC-07 | Indexing progress indicator and control (pause, resume) | 3 | 2 | MVP | 6 |

### A.7 Power-user (PWR)

| ID | Feature | Impact | Effort | Priority | Tranche |
|---|---|---|---|---|---|
| PWR-01 | Command palette (Ctrl+Shift+P) | 5 | 2 | MVP | 7 |
| PWR-02 | Integrated terminal (ConPTY) in the current folder (PowerShell, cmd, WSL) | 4 | 2 | MVP | 7 |
| PWR-03 | Companion CLI `fx` (`fx open .`, `fx search "*.rs"`) | 4 | 2 | MVP | 7 |
| PWR-04 | Native WSL support (`\\wsl$\`, distributions listed) | 3 | 2 | MVP | 7 |
| PWR-05 | Keyboard shortcuts: Explorer parity and user remapping | 4 | 2 | MVP | 7 |
| PWR-06 | Git badges (modified, added, ignored) in lists | 4 | 3 | v1.5 | 13 |
| PWR-07 | Vim-style navigation mode (hjkl) | 2 | 2 | Community | — |

### A.8 Customization and Windows 11 (UX)

| ID | Feature | Impact | Effort | Priority | Tranche |
|---|---|---|---|---|---|
| UX-01 | Light and dark themes, synced with Windows (and manual override) | 4 | 2 | MVP | 9 |
| UX-02 | Mica effect (Win11) with clean fallback on Win10 | 4 | 2 | MVP | 9 |
| UX-03 | Snap Layouts compatibility (Win11) | 3 | 1 | MVP | 9 |
| UX-04 | Portable mode (executable on a USB stick, data alongside) | 3 | 1 | MVP | 9 |
| UX-05 | Remembering position and size per screen | 2 | 1 | MVP | 9 |
| UX-06 | Customizable columns (add, remove, reorder, width) | 3 | 2 | MVP | 9 |
| UX-07 | Display density (compact, comfortable) | 3 | 1 | MVP | 9 |
| UX-08 | Customizable toolbar | 2 | 2 | MVP | 9 |
| UX-09 | Jump Lists (right-click on the taskbar icon) | 3 | 2 | v1.5 | 13 |
| UX-10 | Custom icon packs | 2 | 2 | Community | — |

### A.9 System (SYS)

| ID | Feature | Impact | Effort | Priority | Tranche |
|---|---|---|---|---|---|
| SYS-01 | Dev Drive support (Windows 11): detection and best practices | 2 | 1 | MVP | 4 |
| SYS-02 | Visual indicators for compressed and encrypted files | 3 | 1 | MVP | 9 |
| SYS-03 | Complete native context menu (merging `IContextMenu`, third-party extensions) | 5 | 4 | MVP | 3 |
| SYS-04 | Ferrix-specific additional context menu | 3 | 2 | MVP | 3 |
| SYS-05 | Session recovery after crash | 3 | 2 | v1.5 | 13 |
| SYS-06 | Occasional UAC elevation (admin operation without restarting the application) | 3 | 3 | v1.5 | 13 |
| SYS-07 | Set Ferrix as default explorer (if cleanly possible) | 3 | 3 | v1.5 | 13 |
| SYS-08 | Simplified NTFS ACL editor | 2 | 3 | v2 | 16 |
| SYS-09 | Opt-in transparent telemetry (anonymous crash reports) | 2 | 2 | v1.5 | 13 |

### A.10 Accessibility (A11Y)

| ID | Feature | Impact | Effort | Priority | Tranche |
|---|---|---|---|---|---|
| A11Y-01 | `fx-a11y` skeleton (UIA root element, basic roles) | 5 | 2 | MVP | 0 |
| A11Y-02 | Full UI Automation support (lists, trees, tabs, buttons, announcements) | 5 | 4 | MVP | 1 to 10 (continuous) |
| A11Y-03 | 100% keyboard navigation, including menus and palette | 4 | 2 | MVP | 1 to 10 (continuous) |
| A11Y-04 | High contrast mode (follows system theme) | 3 | 1 | MVP | 10 |
| A11Y-05 | DPI scaling and system text size | 4 | 2 | MVP | 9 |
| A11Y-06 | FR and EN internationalization | 3 | 2 | MVP | 10 |
| A11Y-07 | Documented audit with Narrator and NVDA | 4 | 2 | MVP | 10 |

### A.11 Cloud (CLD)

| ID | Feature | Impact | Effort | Priority | Tranche |
|---|---|---|---|---|---|
| CLD-01 | Generic `CloudProvider` trait (list, status, quota, sync_state) | 4 | 3 | v1.5 | 11 |
| CLD-02 | OneDrive: synchronized folder detection, badges via Cloud Files API | 5 | 4 | v1.5 | 11 |
| CLD-03 | Google Drive: local client detection, fallback to Drive v3 API | 4 | 4 | v1.5 | 11 |
| CLD-04 | Dropbox: local client detection, API fallback | 3 | 3 | v1.5 | 11 |
| CLD-05 | Per-account quota display | 3 | 2 | v1.5 | 11 |
| CLD-06 | OAuth2 authentication (system browser, secure token storage) | 4 | 3 | v1.5 | 11 |

### A.12 Advanced network (NET)

| ID | Feature | Impact | Effort | Priority | Tranche |
|---|---|---|---|---|---|
| NET-01 | FTP, SFTP, and WebDAV client browsable as a folder | 3 | 4 | v2 | 14 |
| NET-02 | Offline files (network cache) | 2 | 3 | Community | — |

### A.13 Extensibility (EXT)

| ID | Feature | Impact | Effort | Priority | Tranche |
|---|---|---|---|---|---|
| EXT-01 | Sandboxed WASM plugins (stable, versioned API) | 5 | 5 | v2 | 16 |
| EXT-02 | Rhai scripting for custom actions | 3 | 3 | v2 | 16 |
| EXT-03 | Simplified file association manager | 2 | 2 | v2 | 16 |
| EXT-04 | Extended command-line interface (scripts, JSON output) | 3 | 2 | v1.5 | 7 (base) and 13 |

### A.14 Priority synthesis

| Priority | Scope |
|---|---|
| **MVP** | Most of NAV, SID (except cloud), OPS (except v2), PRV, PWR, UX, A11Y — 1.0 release (tranches 0 to 10, 17, 18) |
| **v1.5** | Cloud, advanced tools, Git, Jump Lists, SMB — tranches 11 to 14 |
| **v2** | WASM plugins, Rhai, diff, smart folders, FTP/SFTP, ACL, versions — tranche 16 |
| **Community** | Automatic rules, vim-like, icon packs, offline files — open tickets |

---

## Annex B — "Killer features" matrix (synthesis)

> Extracted from the sonnet PRD: quick framing of differentiating features. In case of conflict with Annex A, Annex A prevails.

| Feature | Impact | Effort | Priority |
|---|---|---|---|
| Command palette (Ctrl+Shift+P) | 5 | 2 | **MVP** |
| Quick Look / fast preview (Space) | 5 | 4 | **MVP** (Tranche 8 — decision D-1) |
| Integrated terminal (ConPTY) | 4 | 2 | **MVP** |
| Companion CLI (`fx open .`) | 4 | 2 | **MVP** |
| Commander-style dual-pane | 4 | 3 | **MVP** (Tranche 5) |
| Native WSL support | 3 | 2 | **MVP** |
| Git status badges | 4 | 3 | v1.5 |
| Saved workspaces | 3 | 2 | **MVP** |
| UI Automation support (screen readers) | 5 | 4 | **MVP — started in Tranche 0** |
| WASM plugins | 5 | 5 | v2 (big project, good for attracting contributors) |

**Version note (sonnet PRD):** accessibility (Tranche 10) and power-user quick wins (Tranche 7) were moved earlier than in the initial versions, because adding them after the fact on a frozen UI is far more costly. Quick Look becomes its own dedicated tranche given its complexity and impact.

---

## Annex C — Source traceability and document history

### C.1 Contributions per source PRD

| Source | What was reused | Where |
|---|---|---|
| **PRD_claude-haiku-5.5-high** | Document backbone, identified backlog, acceptance criteria per tranche, performance objectives, legal decisions (Slint/LZTurbo/UnRAR), threading model, caches, personas, DoD, CLAUDE.md, glossary | §5.2, §6, §7, §9, §10.3, §10.8, §16, §18, §20.1, §21, Annex A |
| **PRD_gpt-5.6-sol** | Detailed functional requirements, threat model and security, test strategy (5 levels), shortcut table, search syntax, Shell extension isolation, archive security, Rust-first positioning, KPIs, SBOM, packaging, milestones | §4, §10.2/10.4/10.5/10.7, §13, §14.3, §15, §19, §20, Annex A (crate layout) |
| **PRD_claude-sonnet-5-high** | "Killer features" framing, thematic matrix, rationale for reordering tranches, preview/compression engine | Annex B, §6 (D-1, D-3), §12 |

### C.2 Document history

| Version | Changes |
|---|---|
| 1.0 | First version (vision, stack, initial roadmap) — haiku PRD |
| 2.0 | Impact/effort matrix, Quick Look engine and optional backends, anticipated accessibility, measurable performance objectives, legal decisions, identified backlog, acceptance criteria, agent conventions and prompts — haiku PRD |
| — | "Three parts" variant (matrix + preview engine + revised PRD with Quick Look as a dedicated tranche) — sonnet PRD |
| — | Complete functional specification (41 sections: requirements, security, tests, KPIs, packaging) — gpt-5.6 PRD |
| **3.0** | **Merge of the three PRDs into a single document**; conflicts arbitrated (section 6); restructured into 22 sections + 3 annexes |

---

**End of document — Ferrix Explorer PRD v3.0**

