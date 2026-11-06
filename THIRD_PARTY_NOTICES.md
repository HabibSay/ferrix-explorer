# Third-party notices

This is an initial development inventory, not a release notice. Verify all resolved transitive
dependencies from `Cargo.lock` before publishing binaries.

- `windows` 0.62.2 — MIT OR Apache-2.0. Windows API bindings used by the Win32 host and Shell/OLE
  spike.
- `windows-implement` 0.60.2 — MIT OR Apache-2.0. COM `#[implement]` attribute macro used by the
  Shell/OLE spike.
- `tracing` 0.1.44, `tracing-appender` 0.2.5, `tracing-subscriber` 0.3.23 — MIT. Logging and
  daily log rotation.
- `directories` 6.0.0 — MIT OR Apache-2.0. Standard Windows data locations.
- `slint` 1.18.1, `slint-build` 1.18.1, and the `i-slint-*` crates they use — product dependency (ADR 0001 accepted 2026-10-29). This
  project uses them under the **Slint Royalty-free 2.0** option; the GPLv3 option is not used.
  Attribution requirements of the Royalty-free option must be satisfied before distributing a
  binary that contains Slint. License choice is still subject to ADR 0001.
- `iced` 0.14.0 — MIT; spike only.
- `jwalk` 0.9.0 — MIT. Parallel filesystem walker used by `fx-fs`. Pulls `rayon` and `crossbeam` transitively.
- `notify` 8.2.1 — CC0-1.0. Filesystem event watcher used by `fx-fs`. Product dependency (ADR 0001).
- `tokio` 1.54.1 — MIT. Async runtime (`rt-multi-thread` feature only) used by `fx-fs` and `fx-app`.
- `tokio-util` 0.7.20 — MIT. `CancellationToken` (`sync` feature only) used by `fx-fs`.

The application currently has no product-selected UI framework and no optional compression,
preview, or cloud SDK dependency. See `docs/LEGAL.md` for the dependency policy and restricted
component register.
