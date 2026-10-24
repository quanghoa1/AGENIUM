# ADR-56: CliBackend = headless JSON stream in an Agenium-owned per-run home

- **Status:** Accepted; `agy` driven; codex next (owner logged in, test model Luna 6); permission handling in YOLO
  mode being designed (see "Open")
- **Date:** 2026-10-24 (Asia/Saigon)
- **Implements:** decision 3c and ADR-30. An ACP-based attempt (`devin acp`) was dropped at the owner's request: only
  the API backend and the CLIs' own headless JSON-stream modes are used.

## Decision

1. **Headless JSON stream, one process per run.** The CLI reads user messages from stdin and writes events to stdout
   (agy: `--input-format stream-json --output-format stream-json`). Each turn's end is an event; Agenium can send a
   follow-up message (nudge, refusal notice) in the same process. CLI sessions are never resumed across runs.
2. **Per-run home instead of the user's configuration (ADR-30).** Each run gets a fresh directory; the driver writes
   Agenium's configuration there and points the CLI at it. The user's MCP servers, hooks, rules, skills and
   permissions are not loaded and nothing of the user's is written. The login is reused read-only.
   - agy: `USERPROFILE`/`HOME` = the per-run home (agy keeps its configuration in `<home>/.gemini`, its login outside
     it). Verified 2026-10-24: in a redirected home agy sees none of the owner's MCP servers (figma) or hooks, is still
     logged in, and writes all its state into the per-run home.
3. **Agenium's MCP server** (`agent/mcp/src/server.rs`: loopback, per-run bearer token) offers the protocol tools and
   the agent's own MCP bindings; it is the only MCP server in the per-run configuration.
4. **Limits and mandatory report** are counted from the event stream: tool steps, repeated steps, wall-clock timeout
   (the process is stopped at a limit -> `Checkpoint` / `LoopDetected`); a work turn that ends without a protocol
   tool gets a nudge (2 at most) -> `NoProgress`.
5. **Agent setting:** `backend = Cli`, `model.provider` = CLI program, `model.model` = the CLI's model. Cost 0
   (subscription), tokens recorded.

## Per-CLI status (2026-10-24)

| CLI | Headless stream | Per-run isolation | Status |
|---|---|---|---|
| agy 1.4.4 | `-p` + stream-json in/out | `USERPROFILE`/`HOME` -> per-run home (verified) | driven |
| codex 0.163.0 | `exec --json` | `CODEX_HOME`, `--ignore-user-config`, `--ignore-rules`, `--ephemeral`, `-c` | next (owner logged in) |
| opencode 2.1.2 | `run --format json` | `XDG_*`, `OPENCODE_CONFIG_CONTENT`, `OPENCODE_DISABLE_PROJECT_CONFIG` | not checked (login unknown) |
| claude | `-p --output-format stream-json` | `CLAUDE_CONFIG_DIR`, `--strict-mcp-config`, `--settings` | not run (owner's rule) |
| devin | `-p` prints text only | none (always loads the user's MCP servers and hooks) | not drivable |

## Open

- **Permissions in YOLO mode.** The owner's rule: every CLI runs in its YOLO mode inside Agenium and Agenium handles
  permissions. The current agy driver still uses agy's allow-rules written from the project allowlist (refused
  commands end the turn and are reported back); the YOLO + Agenium-side enforcement design is not settled yet.
- The CLI process does not run inside Agenium's sandbox yet (it needs egress to the vendor API).
- No fallback to the API backend when the CLI is missing or incompatible.

## Verification

- `agent/cli-backend/tests/stream_run.rs` against `agenium-fake-agy` (reads its configuration from `USERPROFILE`
  like agy): per-run home used and deleted, only Agenium's MCP server, refused command reported in the next turn,
  report through MCP, nudges, loop, step limit, missing CLI.
- Manual agy runs (Gemini 3.8 Flash Low) in a per-run home: allowed command ran, refused command ended the turn,
  a follow-up message continued the same session, `submit_report` reached the MCP server once `mcp(agenium/*)` was
  allowed.
- An earlier ACP run of the golden tasks through devin (5/5) is kept in `quality/eval/results/` as history only.
