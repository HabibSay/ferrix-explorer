# Dependency and licensing register

Status: Tranche 2 slice 1. This file must be updated whenever a dependency is added, removed, or changed.

## Project license decision

Ferrix is currently intended to use `MIT OR Apache-2.0`, but the UI framework decision is still open. Slint's GPLv3 option is incompatible with distributing Ferrix under this dual license; see `docs/adr/0001-ui-framework.md`. Do not publish binaries until the UI license path is reviewed and the project license is explicitly decided.

## Current direct dependencies

| Dependency | Used by | License / status | Distribution notes |
|---|---|---|---|
| `windows` 0.62.2 | `fx-app`, `fx-shell` | MIT OR Apache-2.0 | Windows API bindings; code is isolated in Windows-specific crates. |
| `windows-implement` 0.60.2 | `fx-shell` | MIT OR Apache-2.0 | `#[implement]` attribute macro for the `IDropTarget` and `IContextMenu` spikes. Proc-macro only (depends on `syn`/`quote`/`proc-macro2`), no native code. Version pinned below 0.100.0 because that release requires rustc 1.95 while the workspace pins 1.92. |
| `tracing` 0.1.44 | `fx-app` | MIT | Structured logging. |
| `tracing-appender` 0.2.5 | `fx-app` | MIT | Daily rotating file logs in `%LOCALAPPDATA%`. |
| `tracing-subscriber` 0.3.23 | `fx-app` | MIT | `RUST_LOG`-style filtering and formatting. |
| `directories` 6.0.0 | `fx-app`, `fx-settings` | MIT OR Apache-2.0 | Resolves `%APPDATA%` and `%LOCALAPPDATA%`. |
| `slint` 1.18.1 | `slint-spike` | GPLv3, Royalty-Free, or commercial (review exact selected terms) | Product dependency (ADR 0001 accepted 2026-10-29). Not a product dependency unless the license decision permits it. |
| `slint-build` 1.18.1 | `slint-spike` build | Same project family as Slint; review selected terms | Product dependency (ADR 0001 accepted 2026-10-29). Build-time dependency for the spike only. |
| `iced` 0.14.0 | `iced-spike` | MIT | Spike only; not a product decision. |
| `jwalk` 0.9.0 | `fx-fs` | MIT | Parallel filesystem walker. Pulls rayon and crossbeam transitively. |
| `notify` 8.2.1 | `fx-fs` | CC0-1.0 | Filesystem event watcher. Product dependency (ADR 0001). |
| `tokio` 1.54.1 | `fx-fs`, `fx-app` | MIT | Async runtime, rt-multi-thread feature only. |
| `tokio-util` 0.7.20 | `fx-fs`, `fx-ops` | MIT | CancellationToken (feature sync only). |
| `md-5` 0.11.0 | `fx-ops` | MIT OR Apache-2.0 | MD5 hashing for OPS-12 checksums. Pure Rust, declared MSRV 1.85. |
| `sha2` 0.11.1 | `fx-ops` | MIT OR Apache-2.0 | SHA-256 hashing for OPS-12 checksums. Pure Rust, declared MSRV 1.88. |
| `blake3` 1.8.7 | `fx-ops` | CC0-1.0 OR Apache-2.0 OR Apache-2.0 WITH LLVM-exception | BLAKE3 hashing for OPS-12 checksums. **The `pure` feature is enabled**: the default x86-64 build compiles C/assembly via `cc`, so without `pure` this entry would be a native dependency. |
| `tempfile` 3.20.0 | dev: `fx-fs`, `fx-settings`, `fx-ops` | MIT OR Apache-2.0 | Test-only temporary directories and fixtures. Not linked into product binaries. |

Slint is accepted only under the Royalty-free option for the two spike crates: `deny.toml` globally allows the permissive licenses and grants `LicenseRef-Slint-Royalty-free-2.0` through per-crate exceptions, so `GPL-3.0-only` is never globally approved. This exception must be revisited together with ADR 0001 and removed if both prototypes are deleted.

Exact dependency metadata must be checked from the resolved `Cargo.lock` and current license texts before release. CI must reject unapproved licenses, and a release SBOM plus third-party notices must be produced.

## Restricted / optional engines (not currently dependencies)

| Component | Status | Rule |
|---|---|---|
| LZTurbo / powturbo | Not included | Never ship or download by default; any user-installed adapter requires explicit opt-in and a current license review. |
| UnRAR | Not included | If later integrated, read/extract only through official `unrar.dll`, isolated behind `rar` feature; no RAR creation. |
| MuPDF | Not included | AGPL; excluded from default distribution. |
| FFmpeg full build | Not included | Optional only after build-specific LGPL/GPL and redistribution review. |
| libraw | Not included | Optional only after LGPL review. |

## Required checks before release

- Review every direct and transitive crate license from the lockfile.
- Run `cargo deny check` with the repository policy and document approved exceptions.
- Generate an SBOM for each release and maintain `THIRD_PARTY_NOTICES.md`.
- Review native code, vulnerabilities, Windows ARM64 support, and update activity.
- Re-check licenses at release time; this document is not legal advice.
