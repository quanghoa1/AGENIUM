# ADR-41: Codex sandbox: vendored unmodified, Agenium code in a wrapper crate

- **Status:** Accepted (owner decision, 11:10); details of 'reference-only' are Proposed
- **Date:** 2026-10-23 (Asia/Saigon)
- **Context / decision / consequences:**

- `vendor/codex-sandbox/upstream/` holds unmodified Codex files pinned to a commit (VENDOR.md: repo, hash, Apache-2.0, files, update procedure; `scripts/sync-codex-sandbox.sh`).
- All Agenium additions live in `crates/agenium-sandbox` behind `trait Sandbox`. Edits to vendored files are forbidden; if unavoidable, numbered patches in `vendor/patches/` applied by script (none today).
- Deviation discovered in M0: Codex's Rust sandbox files depend on Codex-internal crates (`codex-protocol`, `codex-sandboxing`, `codex-utils-*`). Vendoring those would drag in half of Codex, so the Rust files are **reference-only** (diffed on each sync, hardening ported by hand); only the `.sbpl` policies are compiled in. **Needs owner confirmation** (alternative: vendor the internal crates too).
- Attack suite is a CI test target `sandbox-attack-tests`; maintenance guide: `docs/SANDBOX-MAINTENANCE.md`.
