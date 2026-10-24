# ADR-53: Windows sandbox backend = Microsoft MXC process containers (native, adapted from Codex)

- **Status:** Accepted (owner, 2026-10-24: "có, chúng ta sẽ tận dụng sandbox của codex, chứ không phải gọi nó thông qua
  codex cli")
- **Date:** 2026-10-24 (Asia/Saigon)
- **Amends:** ADR-39 (Windows deferred, no backend) for the sandbox only; decision 9 / ADR-41 (Codex-derived sandbox).

## Context

Spike S2 (`docs/spike-reports/S2/REPORT.md`) ran for the first time on Windows 11 build 26300. Codex 0.163 has three
Windows backends: `unelevated` (restricted token — network and detached processes escaped: KILL), `elevated` (sandbox
users + firewall, needs admin: not run) and `mxc` (Microsoft MXC process container: no admin, no extra users). MXC with
a narrow permission profile blocked every attack of the matrix, killed process trees 20/20, ran the valid work and
cost +1.7 % on a build. The owner wants Codex's sandbox used directly, without depending on the Codex CLI.

Codex's MXC backend (`codex-rs/mxc-sandbox`, ~950 lines) is a thin layer over Microsoft's `mxc-sdk` crate (MIT,
crates.io). Codex's other Windows backend (`windows-sandbox-rs`, ~1.1 MB of Rust + a Windows service) depends on Codex
internals and admin setup.

## Decision

1. `agenium-sandbox` gets a Windows backend `WindowsMxc` built on `mxc-sdk = "=1.0.0"` (plus `flatbuffers`,
   `tracelogging = "=1.2.3"` as Codex pins it for GNU linking). The availability probe and the container-policy shape
   are adapted from Codex (Apache-2.0, attribution in `crates/agenium-sandbox/src/mxc.rs` and `NOTICE`). No Codex
   binary or Codex crate is required.
2. `detect()` reports `WindowsMxc` only when the native process-security-environment API works (cached probe that
   creates and closes an environment without launching anything); otherwise `None` and the existing fail-closed rules
   apply (approval for every command, YOLO refused).
3. With the backend, **every** agent command runs in a container (whatever the approval policy) with:
   writable = the ticket worktree + a per-worktree temp dir outside it; read-only = Windows platform folders
   (`SystemRoot`, Program Files, ProgramData), toolchain roots derived from `PATH` (never the user profile or a drive
   root; for Cargo only `bin`, `registry`, `git` — `credentials.toml` stays unreadable — plus `RUSTUP_HOME`), the
   repository's git directory (a worktree's git dir lives outside it) and the agent's skill folders; network egress,
   ingress and host loopback denied; clean environment; `registryRead` capability; desktop UI isolation as in Codex.
   Paths are passed in their long form (8.3 short names break .NET inside the container).
4. **Shell inside the container is PowerShell** (`pwsh` if installed, else Windows PowerShell 5.1), as in Codex:
   Git Bash/MSYS cannot start in the container (its runtime creates `\BaseNamedObjects\msys-2.0S5-*`, denied with
   `0xC0000022`). Native tools work (git, cargo, python, node — the latter with `--preserve-symlinks-main`). The agent
   gets a sandbox-owned global gitconfig (`GIT_CONFIG_GLOBAL`, `GIT_CONFIG_NOSYSTEM=1`); `git commit` inside the
   container is blocked by design — Agenium commits outside it.
5. Build: `mxc-sdk`'s build script embeds version info into its own helper binaries with `rc.exe`; on the GNU toolchain
   `.cargo/config.toml` sets `RC_PATH` to `scripts/rc-windres.cmd` (MinGW `windres`, or an empty resource when absent —
   those binaries are never built by Agenium).
6. Not yet: MCP stdio servers stay outside the container (user-configured, often need network; next step), network
   allowlists per domain (MXC needs a managed proxy for that), PTY, `elevated` backend.

## Consequences

- Windows is no longer "no backend": YOLO (`approval = never`) becomes possible on machines where MXC works.
- Each sandboxed command pays ~0.3 s (Python) to ~0.8 s (PowerShell start) of fixed cost; the local test suite went
  from ~290 s to ~575 s because tests run hundreds of commands.
- Availability on **stable** Windows 11 releases is unverified (measured on build 26300); there the backend simply
  reports `None`.
- `mxc-sdk` is young (1.0.0, 2026-10-06): pinned exactly; upgrades need the S2 matrix again.
