# ADR-99: Dependency policy — cargo-deny in local CI

Status: Accepted (2026-10-26). Evidence: `deny.toml`; `cargo deny check` on both workspaces (core and desktop):
advisories ok, bans ok, licenses ok, sources ok.

## Context

GitHub Actions is not used (the account's minutes are spent), so checks run locally
(`devtools/scripts/ci-local.{sh,ps1}`). Until now nothing checked the dependency tree: licenses, known
vulnerabilities, crates from outside crates.io. The first run found three RustSec advisories against `zip`
(RUSTSEC-2026-0367/0368/0369: `ZipArchive::extract` keeps SUID bits, follows extracted symlinks outside the
destination, applies permissions through symlinks).

## Decision

1. **`cargo deny check` is a local CI step** for the core workspace and the desktop app's own workspace.
   `cargo install cargo-deny --locked` once per machine; the step says so when it is missing.
2. **Licenses:** permissive only (Apache-2.0, MIT, BSD, ISC, Zlib, Unicode-3.0, Unlicense, BSL-1.0, CC0-1.0,
   CDLA-Permissive-2.0, 0BSD, MIT-0) plus MPL-2.0 for an unmodified file-level-copyleft crate (option-ext). A crate
   under several licenses passes when one is listed.
3. **Advisories:** any vulnerability in the tree fails. An exception names the advisory and why it does not apply,
   and when to drop it.
4. **Sources:** crates.io only; no git dependencies. Several versions of one crate warn, never fail.

The `zip` advisories: Agenium's own unpacking (skill archives, `agent/skills`) never called `extract()` — it refuses
symbolic links and paths that leave the folder and writes files with default permissions — but it now uses `zip` 9.1
(fixed). The other copy, `zip` 8.6 under `candle-core`, only reads model files (`.npz`, `.pth`) and never extracts
(checked in candle-core, candle-nn and candle-transformers 0.11); the three advisories are ignored for it with that
reason until candle moves to `zip` 9.

## Consequences

- A new dependency with a non-permissive license, a crate from a git source, or a newly published vulnerability
  stops local CI.
- The advisory database is fetched from GitHub at each run (network needed).
