# ADR-68: opencode CLI backend for OpenCode's free models

Status: Accepted (2026-10-25, owner: "write the opencode CLI driver so the free models can be used"). Revisits the
2026-10-24 decision of ADR-56 that dropped the opencode CLI: OpenCode's docs say "Free-tier chat models require
OpenCode", so the free models (`opencode/*-free`) are reachable only through OpenCode itself, and Agenium never
poses as its client.

## Decision

1. **Transport: `opencode serve`, not `opencode run`.** `run` rejects every permission request itself ("this
   non-interactive run cannot ask the user"), and with `--auto` it would approve everything. Inside Agenium's sandbox
   opencode cannot start at all: Bun resolves the working directory with `lstat` on every component from the drive
   root, and the MXC container denies the root (granting it would open the whole drive). So the driver starts
   `opencode serve --hostname 127.0.0.1 --port 0` on the host with a password Agenium chooses, and an in-process
   bridge (`opencode::bridge`) drives its API: `POST /api/session` (location = the ticket worktree, model),
   `POST /api/session/{id}/prompt` per user message, `GET /api/event` for the stream. Tool events are completed with
   the tool's name and input; `session.step.ended` gives the tokens; `session.execution.succeeded|failed` ends a turn.
2. **Control: Agenium decides every permission (`Control::Hook`).** The config asks for everything (`"*": "ask"`)
   except reads, todo lists and Agenium's own MCP tools; web tools and other folders are denied, subagents too in team
   mode. Each `permission.asked` goes to the same hook endpoint Claude Code uses: edits only inside the worktree, shell
   commands through the run's command gate (allowlist or the user, like every host-run CLI), anything else refused
   with a reason; an unreachable hook rejects.
3. **Isolation.** `XDG_CONFIG_HOME`, `XDG_DATA_HOME`, `XDG_STATE_HOME`, `XDG_CACHE_HOME` and `OPENCODE_TEST_HOME`
   point into the per-run home; `OPENCODE_DISABLE_PROJECT_CONFIG`, `OPENCODE_DISABLE_CLAUDE_CODE(_SKILLS)`,
   `OPENCODE_DISABLE_AUTOUPDATE` and the file watcher are off. The user's opencode config, sessions, background
   service and login are never touched; free models need no login. The npm shim is resolved to
   `node_modules/@opencode/cli/bin/opencode(.exe)` (`Driver::program`).
4. **Models.** The model picker offers OpenCode's free models still served: the `opencode` provider of the price
   catalogue (ADR-66) at price 0 and not `deprecated` (the catalogue keeps deprecated models for old runs' prices).
   Paid OpenCode models keep running through the API backend.

## Evidence

- Golden tasks through the CLI with `deepseek-v4.1-flash-free`: **5/5 PASS**, 9-21 steps, $0
  (`quality/eval/results/20261025T050257Z-cli-opencode-deepseek-v4.1-flash-free.json`).
- Unit tests: `opencode::{parses_the_event_stream, permission_requests_follow_agenium_rules,
  the_home_isolates_opencode}` (events captured from opencode 2.1.2).

## Consequences

- `Driver::served`, `Driver::program`, `Setup::model`, `Event::Usage` (tokens per step); `AGENIUM_KEEP_CLI_HOME=1`
  keeps a run's CLI home for diagnostics; `cargo run -p agenium-sandbox --example contained` runs a command in the
  sandbox by hand.
- opencode's shell commands run on the host after the gate decides (as for claude and agy), not in the container.
