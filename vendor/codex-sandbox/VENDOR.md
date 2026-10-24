# Vendored OpenAI Codex sandbox sources

- Upstream: https://github.com/openai/codex (Apache License 2.0, `upstream/LICENSE`, `upstream/NOTICE`)
- Upstream commit: `970111f6cb62cc02fe36a82ea7d71b37a2889c97` (2026-10-23T18:50:54Z)
- Files: `FILES.txt` (what is vendored), `FILES.lock` (sha256 of each vendored file, checked by
  `scripts/sync-codex-sandbox.sh --check` in local CI)

Everything under `upstream/` is byte-for-byte upstream and **never edited**. Agenium code that is adapted from these files
lives only in `crates/agenium-sandbox/src/codex_derived/`, each file naming its upstream source; all other Agenium sandbox
code is written independently. Process for an upstream change: `docs/reference/SANDBOX-MAINTENANCE.md` §2.

## Where each upstream file is used

| Upstream file | Agenium | How |
|---|---|---|
| `codex-rs/mxc-sandbox/src/availability.rs` | `codex_derived/mxc_probe.rs` | **derived**: the native MXC availability probe (create + close a process security environment, no launch) |
| `codex-rs/mxc-sandbox/src/policy.rs` (`build_request`) | `codex_derived/mxc_policy.rs` | **derived**: the container policy shape (registry-read capability, no DACL fallback, directional network rules, desktop UI isolation, conflict normalization) |
| `codex-rs/mxc-sandbox/src/windows.rs` (`platform_read_roots`) | `codex_derived/mxc_policy.rs` | **derived**: Windows platform folders granted read-only |
| `codex-rs/mxc-sandbox/src/native.rs` | `crates/agenium-sandbox/src/mxc.rs` | reference: Agenium launches with piped stdio and its own timeout instead of an inherited-stdio helper |
| `codex-rs/mxc-sandbox/src/lib.rs`, `transport.rs`, `README.md`, `Cargo.toml` | — | reference: helper re-exec transport (not used), dependency pins (`tracelogging = "=1.2.3"` for GNU linking is mirrored) |
| `codex-rs/linux-sandbox/README.md`, `src/bwrap.rs`, `src/seccomp.rs`, `src/linux_run_main.rs` | `crates/agenium-sandbox/src/unix.rs` | reference only: Codex runs `bwrap --ro-bind / /` (whole disk readable) + seccomp; Agenium uses mxc-sdk's allow-list bubblewrap backend (ADR-54). Watch the diff for new hardening (blocked syscalls such as `io_uring`, bwrap flags) |
| `codex-rs/sandboxing/src/seatbelt.rs`, `seatbelt_*.sbpl` | `crates/agenium-sandbox/src/unix.rs` (macOS, ADR-55) | reference: Codex's Seatbelt profiles; read the diff for new deny rules |
| `LICENSE`, `NOTICE` | `NOTICE` | attribution |
