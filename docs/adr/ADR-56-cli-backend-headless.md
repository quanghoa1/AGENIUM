# ADR-56: CliBackend = headless CLI in its YOLO mode, per-run home, permissions decided by Agenium

- **Status:** Accepted; implemented for `codex` and `agy` (golden tasks 5/5 through each, see Verification)
- **Date:** 2026-10-24 (Asia/Saigon)
- **Implements:** decision 3c and ADR-30. An ACP-based attempt (`devin acp`) was dropped at the owner's request:
  only the API backend and the CLIs' own headless JSON-stream modes are used.
- **Owner's rules (2026-10-24):** every CLI runs in its own YOLO mode inside Agenium and Agenium handles the
  permissions; the CLIs must not use the user's own configuration.

## Decision

1. **Headless JSON stream.** The CLI's agent loop runs headless and Agenium reads its event stream. Two shapes:
   - one process for the run, one stdin line per user message (agy: `--input-format/--output-format stream-json`);
   - one process per turn, the message on stdin, later turns resume the same conversation of this run (codex:
     `exec --json -`, then `exec resume <thread> --json -`). CLI sessions are never resumed across runs.
2. **Per-run home instead of the user's configuration (ADR-30).** Each run gets a fresh directory holding only
   Agenium's configuration; the CLI is pointed at it. The user's MCP servers, hooks, rules, skills, plugins and
   permissions are not loaded and none of the user's files are modified, except that a codex login refreshed during
   the run is written back to the user's `auth.json` (only when that file did not change meanwhile) so the user's
   own codex stays logged in.
   - codex: `CODEX_HOME` = per-run home with Agenium's `config.toml` (its MCP server; the token comes from an
     environment variable) and a copy of `auth.json`.
   - agy: `USERPROFILE`/`HOME` = per-run home (agy's configuration lives in `<home>/.gemini`, its login in the OS
     credential store) with Agenium's `mcp_config.json`, `hooks.json` + hook script and `settings.json`.
3. **YOLO mode, Agenium decides** — two ways, chosen per CLI (`Driver::control`):
   - **Sandbox** (codex): the CLI runs with `--dangerously-bypass-approvals-and-sandbox` inside Agenium's sandbox
     (`agenium_sandbox::service::spawn_contained`: MXC on Windows, bubblewrap on Linux, Seatbelt on macOS), worktree
     + per-run home writable, toolchains and the CLI's installation readable, network on for the vendor API. No
     sandbox backend = the run is refused (fail-closed).
   - **Hook** (agy, whose login in the OS credential store cannot be reached from a container): the CLI runs with
     `--dangerously-skip-permissions` on the host; its `PreToolUse` hook (in the per-run home) posts every tool call
     to Agenium's loopback server (`POST /hook`, same per-run bearer token as MCP) and Agenium answers allow/deny:
     shell commands go to the run's command gate (allowlist or the user's approval; YOLO does not approve them because
     they run outside the sandbox — `RunGate.contained = false`), file arguments must stay inside the worktree (or
     the per-run home), only Agenium's MCP server may be called, web and browser tools are refused. A hook that fails
     (Agenium unreachable, bad answer) blocks the tool.
4. **Agenium's MCP server** (`agent/mcp/src/server.rs`: loopback, per-run bearer token, `Origin` must be loopback)
   offers the protocol tools and the agent's own MCP bindings; it is the only MCP server the CLI sees.
5. **Limits and mandatory report** from the event stream: tool steps, repeated steps, wall-clock timeout (the process
   is stopped -> `Checkpoint` / `LoopDetected`); a work turn that ends without a protocol tool gets a nudge (2 at
   most) -> `NoProgress`; after `submit_report` / `ask_question` / ... the CLI gets 30 s to finish its turn.
6. **Agent setting:** `backend = Cli`, `model.provider` = CLI program, `model.model` = the CLI's model
   (`agenium agent cli <name> --cli codex --model gpt-6-luna`). Cost 0 (subscription), tokens recorded.

## Per-CLI status (2026-10-24)

| CLI | Headless stream | Per-run home | Control | Status |
|---|---|---|---|---|
| codex 0.163.0 | `exec --json -` / `exec resume` | `CODEX_HOME` | sandbox | driven, golden 5/5 (gpt-6-luna) |
| agy 1.4.4 | stream-json in/out | `USERPROFILE`/`HOME` | hook | driven, golden 5/5 (gemini-3.8-flash-low) |
| opencode 2.1.2 | `run --format json` | `XDG_*`, `OPENCODE_CONFIG_CONTENT` | — | not checked (login unknown) |
| claude | `-p --output-format stream-json` | `CLAUDE_CONFIG_DIR`, `--strict-mcp-config` | — | not run (owner's rule) |
| devin | `-p` prints text only | none (always loads the user's MCP servers and hooks) | — | not drivable |

## Findings that shaped the design

- codex without YOLO and with no user config blocks writes, commands and MCP calls in `exec` (nobody can approve).
- agy headless without YOLO refuses anything not in `permissions.allow` and ends the turn; its hooks run in YOLO mode
  too, before every tool; a hook command is run through `cmd /c`, which mangles quoted arguments (hence the script).
- agy inside an MXC container is not logged in (the credential store is out of reach); codex is, with its login file
  in the per-run home.

## Open

- opencode and claude drivers; API fallback when the CLI is missing or incompatible; contract tests per minimum
  version in CI.
- Hook mode runs agy's own processes on the host: Agenium decides each tool call, but an allowed command runs
  uncontained (like an approved command without a sandbox backend).

## Verification

- `agent/cli-backend/tests/stream_run.rs` with `agenium-fake-agy` (reads its configuration from `USERPROFILE` and runs
  the `PreToolUse` hook like agy): per-run home used and deleted, only Agenium's MCP server, hook decisions through
  the real script and curl (write outside the worktree, unapproved command, web tool denied; approved command and
  worktree write allowed), report through MCP, nudges, loop, step limit, missing CLI.
- `security/sandbox/tests/contained.rs`: live stdio in the sandbox (Windows MXC and Linux bubblewrap); opt-in codex
  run inside MXC.
- Golden tasks: codex `gpt-6-luna` 5/5 (`quality/eval/results/20261024T055002Z-cli-codex-gpt-6-luna.json`), agy
  `gemini-3.8-flash-low` 5/5 (see `quality/eval/results/*-cli-agy-gemini-3.8-flash-low.json`).
