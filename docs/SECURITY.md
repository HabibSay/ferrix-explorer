# Security baseline

Ferrix handles arbitrary user-selected files and Windows Shell extensions. Security and data integrity take precedence over speed.

## Tranche 0 rules

- The initial Win32 window prototype performs no filesystem operations and makes no network requests.
- Keep unsafe/COM code in `fx-shell`, `fx-terminal`, `fx-git`, and `fx-a11y`; document every unsafe block with its safety invariant.
- Do not run Shell extensions or parse untrusted file formats in the UI process without a reviewed design.
- Do not add dependencies without checking license, maintenance, native code, and known vulnerabilities.
- Do not log file contents, credentials, OAuth tokens, or sensitive paths.
- No elevated process or automatic elevation. A future UAC operation must be explicit and narrowly scoped.
- Run dependency audit and license checks in CI before release.

The detailed threat model and mitigations are in `PRD.md` section 14.3. Update this file when the implementation adds filesystem, archive, network, cloud, or plugin capabilities.
