# Vendored OpenAI Codex sandbox sources

- Upstream: https://github.com/openai/codex (Apache License 2.0, `upstream/LICENSE`, `upstream/NOTICE`)
- Upstream commit: `bd3eaf7829187f18d05205222bf2f67c0af80fb4` (2026-11-18T14:53:00Z)
- Files: `FILES.txt` (what is vendored), `FILES.lock` (sha256 of each vendored file, checked by
  `devtools/scripts/sync-codex-sandbox.sh --check` in local CI)

Everything under `upstream/` is byte-for-byte upstream and **never edited**. Agenium code that is adapted from these files
lives only in `security/sandbox/src/codex_derived/`, each file naming its upstream source; all other Agenium sandbox
code is written independently. Process for an upstream change: `docs/reference/SANDBOX-MAINTENANCE.md` §2.

## Where each upstream file is used

| Upstream file | Agenium | How |
|---|---|---|
| `codex-rs/mxc-sandbox/src/availability.rs` | `codex_derived/mxc_probe.rs` | **derived**: the native MXC availability probe (create + close a process security environment, no launch) |
| `codex-rs/mxc-sandbox/src/policy.rs` (`build_request`) | `codex_derived/mxc_policy.rs` | **derived**: the container policy shape (registry-read capability, no DACL fallback, directional network rules, desktop UI isolation, conflict normalization) |
| `codex-rs/mxc-sandbox/src/windows.rs` (`platform_read_roots`) | `codex_derived/mxc_policy.rs` | **derived**: Windows platform folders granted read-only |
| `codex-rs/windows-sandbox-rs/src/cmd_script.rs` (`cmd_script_command_line`) | `codex_derived/mxc_policy.rs` | **derived**: `cmd [/d] /s /c SCRIPT` passed as cmd reads it (the script quoted once, not CRT-escaped) |
| `codex-rs/mxc-sandbox/src/native.rs` | `security/sandbox/src/codex_derived/mxc_spawn_error.rs`, `security/sandbox/src/mxc.rs` | derived (since e5839cb): the Win32 error number of a failed launch, read from the SDK's extended error (nothing else of it, as it holds the working directory). The rest is reference: Agenium launches with piped stdio and its own timeout instead of an inherited-stdio helper |
| `codex-rs/mxc-sandbox/src/lib.rs`, `transport.rs`, `README.md`, `Cargo.toml` | — | reference: helper re-exec transport (not used), dependency pins (`tracelogging = "=1.2.3"` for GNU linking is mirrored). bd3eaf7: the helper re-exec passes `CODEX_HOME` for its runtime log, and `windows.rs` starts that log; `platform_read_roots` (derived) is unchanged, so nothing to port |
| `codex-rs/linux-sandbox/README.md`, `src/bwrap.rs`, `src/seccomp.rs`, `src/linux_run_main.rs` | `security/sandbox/src/unix.rs` | reference only: Codex runs `bwrap --ro-bind / /` (whole disk readable) + seccomp; Agenium uses mxc-sdk's allow-list bubblewrap backend (ADR-54). Watch the diff for new hardening (blocked syscalls such as `io_uring`, bwrap flags). e5839cb: seccomp now also covers the x32 syscall numbers and denies `request_key`, and adds a transparent-TCP mode; none applies, as Agenium installs no seccomp filter of its own (mxc-sdk bubblewrap, allow-list binds, its own network namespace). bd3eaf7: transparent TCP can inherit the host PID namespace (a supervisor runs the relay and the command), and managed proxy routes need bubblewrap's network namespace; neither applies (no transparent TCP, no managed proxy in Agenium) |
| `codex-rs/sandboxing/src/seatbelt.rs`, `seatbelt_*.sbpl` | `security/sandbox/src/unix.rs` (macOS, ADR-55) | reference: Codex's Seatbelt profiles; read the diff for new deny rules |
| `LICENSE`, `NOTICE` | `NOTICE` | attribution |
