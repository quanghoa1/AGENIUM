# ADR-160: The user may turn the sandbox off on their computer (full YOLO)

Status: Accepted (2026-11-12; owner: "người dùng cần thêm phần bỏ qua sandbox, tức yolo hoàn toàn, vì họ dùng trên
macos gặp vài vấn đề khi gọi tool trong agenium"). Extends ADR-117.

## Context

- ADR-117 lets the user run without a sandbox only on a machine where none works; the checkbox is hidden elsewhere.
- On macOS the Seatbelt profile (ADR-55, experimental) blocks some tools a project needs (a toolchain outside the
  read roots, a socket, a cache in the home folder). Users there want what Codex calls full access: no sandbox, no
  questions.

## Decision

1. **`[sandbox] off`** (default false), set in Settings by a checkbox shown where the OS has a sandbox, after a
   confirmation that names the risk. It is a choice for this computer, not a project.
2. **Off means no backend.** `agenium_sandbox::set_off` makes `detect()` report none (with the reason "turned off in
   Settings"); `detect_os()` still tells what the OS offers. The app sets it from its config when it opens and on
   every config change. Every place that already handled "no sandbox" behaves as on a machine without one.
3. **And the user accepted running without one.** `SandboxSection::runs_unsandboxed()` is `unsandboxed || off`: the
   command gate allows commands without asking (under `sandboxed` and `never`), Codex and the other CLIs that run only
   contained run on the host in their YOLO mode, documents convert with markitdown on the host. The agent's run
   context says commands run directly on the user's computer.
4. **Pre-merge checks and plugin hooks run too.** Without a sandbox they are refused (fail-closed); with the sandbox
   turned off they run directly on the host like every other command.

What does not change: `on-request` still asks for every command; installing software on the computer still asks
(ADR-120); secrets never reach agents; write scope is checked at `submit_report`; the vendor CLIs' own shells stay
off (ADR-129) — their commands go through Agenium's `shell`, which now runs on the host.

## Consequences

- `agenium_sandbox::{set_off, is_off, detect_os}`; `SandboxSection.off`, `runs_unsandboxed()`; desktop
  `set_sandbox_off`, `command_policy` reports `off` and `os_backend`; Settings → Commands has the switch.
- Tests: `security/sandbox/tests/off.rs` (its own binary, since the switch is process-wide: a command writes outside
  its worktree when off, the backend comes back when on), `config.rs` defaults.
- Risk accepted by the owner: with the sandbox off an agent's command can touch anything the user can.

## What the macOS run showed, and what changed in the sandbox (2026-11-12)

`security/sandbox/examples/tool_probe.rs` runs 43 everyday developer commands as the `shell` tool does, with the
sandbox on and then off. Kit v8 on Devin's macOS 26.6 (M4) — 743 tests passed, golden tasks 5/5 both ways, 0
containment escapes — found Seatbelt blocking (`docs/reference/bench-raw/macos-2026-11-12-run8/tool-probe.txt`):

- **Caches in the home folder** (npm's `~/.npm` logs, cargo opening a crate in `~/.cargo/registry`, swift's
  `~/.cache/clang/ModuleCache`, any `~/.cache`). Now each sandboxed command gets cache folders in its worktree's
  temporary directory (`cache_env`: `XDG_CACHE_HOME`, `npm_config_cache`, `PIP_CACHE_DIR`, `CLANG_MODULE_CACHE_PATH`,
  `CARGO_HOME`, `GOMODCACHE`, `GOCACHE`, `GRADLE_USER_HOME`, `YARN_CACHE_FOLDER`): a ticket downloads its own
  dependencies once, and nothing it fetches reaches the user's caches.
- **Homebrew's JDK**: inside Seatbelt the `/usr/bin/java` stub finds no JDK by itself (kit v9: still "Unable to
  locate a Java Runtime" with `/Library/Java` readable). A sandboxed command now gets the host's `JAVA_HOME` (the
  user's, else what `/usr/libexec/java_home` picks), and `JAVA_HOME` passes through the cleaned environment.
- **`swift build`**: past the module cache (kit v9), SwiftPM stops at its own `sandbox-exec` ("sandbox_apply:
  Operation not permitted": Seatbelt cannot be nested). The agent's run context on a Seatbelt machine says to pass
  `--disable-sandbox` to `swift build`/`test`/`run`; the probe checks that form too.
- **User lookups** (`ssh`: "No user exists for uid 501"; `whoami` printed the number): the Seatbelt profile now
  allows the directory service `com.apple.system.opendirectoryd.libinfo` (read-only account data).
- Kit v9 (`macos-2026-11-12-run9`) confirmed npm, npx, cargo with a registry dependency and `ssh`/`whoami` work in
  the sandbox; 743 tests, golden 5/5 both ways, 0 containment escapes.
- Kit v10 (`macos-2026-11-12-run10`) confirmed `java` (through `JAVA_HOME`) and `swift build --disable-sandbox` in the
  sandbox: 34 of 44 probe commands work both ways, 743 tests, golden 5/5 both ways, 0 containment escapes.
- Still refused, on purpose: a server the command starts on 127.0.0.1 (the sandbox cannot reach the host's
  loopback, nor its own), `git commit` in the worktree (the worktree's git directory is read-only; Agenium commits),
  `/bin/ps` (setuid), the pasteboard, the keychain and the user's home folder. Turning the sandbox off is the way past them.
