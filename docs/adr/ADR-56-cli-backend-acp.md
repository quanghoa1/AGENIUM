# ADR-56: CliBackend runs vendor CLIs over the Agent Client Protocol (ACP)

- **Status:** Accepted; implemented for `devin`; other CLIs not driven yet
- **Date:** 2026-10-24 (Asia/Saigon)
- **Amends:** decision 3c / ADR-30 (which command line drives each CLI); the four controls are unchanged.

## Context

Decision 3c named one headless JSON-stream command per CLI (`claude -p --output-format stream-json`,
`codex exec --json`, `agy -p --output-format stream-json`, `opencode run --format json`). Checked on 2026-10-24 with
the CLIs installed on the owner's machine (`agenium doctor`: codex 0.163.0, agy 1.4.4, devin 3000.11.3, claude 2.1.310,
opencode 2.1.2):

- `devin acp` and `opencode acp` start an **ACP server** on stdio (JSON-RPC 2.0, one message per line). ACP gives what
  the four controls need in one protocol: MCP servers are given **per session** (`session/new.mcpServers`, no config
  file written), every tool call is streamed (`session/update` → `tool_call`, `tool_call_update`), the agent **asks the
  client** before risky actions (`session/request_permission`), turns can be cancelled (`session/cancel`), and
  `session/prompt` returns a stop reason and token usage.
- `agy -p --output-format stream-json` streams steps (`step_update`, `result`) but cannot ask for permission in
  headless mode: a command is denied unless the user's `settings.json` allows it or `--dangerously-skip-permissions`
  approves everything. Its MCP servers are managed by `agy mcp` (the user's own configuration). Not driven yet.
- `codex exec --json` / `codex app-server`: not checked (codex is not logged in on this machine). `claude`: never run in
  tests at the owner's request. `opencode acp`: protocol present, run not checked yet.

## Decision

1. **ACP is the CliBackend protocol.** Agenium is the ACP client (`agent/cli-backend/src/acp.rs`); a CLI is driven only
   when `acp_args()` lists it after a verified run. Today: `devin acp [--model <m>]`. A CLI without ACP needs its own
   driver and its own ADR entry.
2. **Protocol tools through Agenium's own MCP server** (`agent/mcp/src/server.rs`): Streamable HTTP on `127.0.0.1`,
   random port, a 256-bit bearer token per run, `Origin` must be loopback, JSON responses only. It is handed to the
   session as `{"type": "http", "name": "agenium", ...}`. It also offers the agent's own MCP bindings
   (`mcp__<server>__<tool>`, credentials resolved by Agenium, output marked untrusted). The CLI's built-in file and
   shell tools replace Agenium's work tools.
3. **Every permission request is decided by Agenium** (`run.rs::decide`): shell commands go to the run's command gate
   (allowlist or the user's approval; **YOLO never applies**, `RunGate.contained = false`, because the CLI's commands
   run on the host); reads and edits only inside the ticket worktree; `fetch` refused; Agenium's own MCP tools allowed
   by exact title only; anything else refused. An "always" option is never selected (decision 4).
4. **Limits from the event stream:** each `tool_call` is a step (`runtime.max_steps_per_run`); the same call
   `loop_detect_same_call` times in a row stops the run; `[cli] run_timeout_mins` (45) bounds the run. At the step or
   time limit Agenium cancels the turn and asks for a checkpoint note (same `Checkpoint` outcome as the API loop).
5. **A work run ends only through a protocol tool.** A turn that ends with text only gets a nudge (2 at most), then the
   run stops as `NoProgress`. After `submit_report` / `ask_question` / ... Agenium cancels the CLI's turn.
6. **Agent setting:** `agent.backend = Cli`; `model.provider` is the CLI program, `model.model` the CLI's model (empty =
   its default). `App::set_agent_backend` switches both together. Cost is recorded as 0 (subscription); tokens are
   recorded from `session/prompt` usage.
7. **Clean process:** the CLI starts with the clean environment (`agenium_sandbox::clean_env`, so e.g. a user's
   `DEVIN_MODEL` does not leak in), its whole process tree is killed at the end, and `session/new` gets the worktree as
   `cwd`.

## Isolation findings (ADR-30 asks for runs isolated from the user's own CLI setup)

`devin` always loads the user's MCP servers from `%APPDATA%\devin\mcp_config.json` and the hooks of its config:
`--config <file>` does not change the MCP file, `APPDATA` / `XDG_CONFIG_HOME` are ignored (the Windows known-folder API
is used), and no `DEVIN_*` variable disables them (checked in the binary's strings). Agenium refuses tool calls of
those servers when devin asks, but read-only tools may be auto-approved by devin itself, and the servers are started
in any case. Therefore `CliKind::partial_isolation()` marks devin, and a devin run is **refused unless the user lists
it in `[cli] allow_partial_isolation`** (`agenium-eval --allow-partial-isolation` sets it for the eval root only).

## Deviations from ADR-30 still open

- **The CLI process does not run inside Agenium's sandbox yet.** It needs egress to the vendor API and read access to
  its own login files; the MXC/bubblewrap policies do not express that yet. Until then: commands need approval or the
  allowlist, edits outside the worktree are refused when the CLI asks, and partial isolation is opt-in. In
  `accept-edits` mode devin applies edits inside the workspace without asking, as the mode's name says.
- No fallback to the API backend when the CLI is missing or incompatible: the run fails with the reason.
- Memory and skills reach the CLI only through the prompt (the work order rendered as text); `load_skill` works
  through the MCP server.

## Verification

- Scripted ACP agent (`agenium-acp-test-agent`) + `tests/acp_run.rs`: report path, permission decisions (allowed
  command, refused command, edit outside the worktree), nudges → `NoProgress`, loop → `LoopDetected`, step limit →
  `Checkpoint` with the note, chat runs, missing CLI.
- `tests/real_cli.rs` (opt-in, `AGENIUM_REAL_CLI=devin`): devin swe-2-medium wrote the file and called
  `submit_report` through the MCP server.
- Golden tasks through devin (`agenium-eval --cli devin --model swe-2-medium --allow-partial-isolation`, commands
  not approved), 2026-10-24: **5/5 PASS** (work run + review run each, 14-20 CLI steps, 47k-60k tokens, 52-89 s;
  `quality/eval/results/20261024T041856Z-cli-devin-swe-2-medium.json`). The M2.5 acceptance ("a golden task runs
  through at least one CLI") is met. Remaining tool errors: denied shell commands (no approver in the eval) and
  `invalid_arguments` on `submit_report` / `review_ticket` — swe-2-medium guesses field names before following the
  schema the MCP server sends (the schemas are complete and inlined); it corrected itself every time.
