# ADR-156: Devin CLI driven over ACP, inside the sandbox

- **Status:** Accepted; implemented and verified (2026-11-04)
- **Date:** 2026-11-04 (Asia/Saigon)
- **Amends:** ADR-56 (devin was "not drivable": `-p` prints only text; the ACP route was set aside then).
- **Owner's rule (ADR-56, unchanged):** the CLI is only the brain. It runs in its own YOLO mode, Agenium decides what
  it may do, and the user's own configuration (MCP servers, hooks, rules) is not used.

## Context

Research outside the repo (`D:\Work\devin-cli-research\REPORT.md`, devin 3000.11.3) found `devin acp`, an Agent Client
Protocol server: JSON-RPC both ways on stdio. It gives structured tool calls (`session/update` `tool_call` with
`rawInput` and the tool's own name in `_meta`), sessions that load again in a new process (`session/load`), modes
(`bypass` is its YOLO), usage per turn, and it hands file reads and writes to the client (`fs/read_text_file`,
`fs/write_text_file`). It found three blockers:

1. The user's MCP servers always load from `%APPDATA%\devin\mcp_config.json` (and from other tools' configurations it
   imports); `mcpServers: []` and `--config` do not stop them.
2. No per-run home: Devin finds its folder through the Known Folder API; `APPDATA` and `XDG_*` do not move it.
3. Hooks do not run under `-p` or `acp`, and a command inside the workspace runs without asking.

## Decision

1. **`Control::Sandbox`**: the whole `devin acp` process runs in Agenium's sandbox (MXC on Windows), like Codex: its own
   commands run contained. A command it judges risky asks `session/request_permission`; the bridge asks Agenium.
2. **Isolation in the sandbox.** Measured on 2026-11-04:
   - In MXC the other tools' configurations Devin would import are not visible.
   - `%APPDATA%\devin` is read-only (its login and configuration) but for `%APPDATA%\devin\cli`, where it must write its
     logs and sessions (`Driver::writable_roots`).
   - `--config <home>/config.json` replaces the user's `config.json` (hooks, permissions).
   - The worktree's `.devin/mcp_config.local.json` turns each of the user's own servers off by name (names read from
     their `mcp_config.json`, never their settings; what the file already held is kept). It is in every project's
     `.git/info/exclude` (`LOCAL_EXCLUDES`), so it is never committed. Agenium's MCP server comes in `session/new`.
   - With all of this, a turn in MXC connected no MCP server of the user's (without the file: `github`, `memory`).
   - `APPDATA`, `LOCALAPPDATA` and `USERPROFILE` are passed as they are: without them Devin stops ("Failed to determine
     config directory"); the sandbox still shows it only its own folder.
3. **The bridge** (`devin/bridge.rs`, `Driver::acp`): one process per turn (`Turns::ProcessPerTurn`).
   - It sends `initialize`, then `session/new`, or `session/load` with the run's kept session id (the replayed history
     is not passed on).
   - It sets `session/set_mode bypass` and sends each input line as `session/prompt`.
   - Updates become the driver's events; a tool's later updates carry its name and input from its first one.
   - The prompt's answer ends the turn with its usage.
   - It answers Devin's own requests by asking Agenium's hook endpoint first: `fs/*` (`Devin::check`: reads and writes
     in the worktree or the run's home; nothing written in a read-only run; the local MCP file refused) and
     `session/request_permission` (Agenium's MCP tools allowed, any other server refused). A hook that cannot be
     reached refuses.
   - A failure carries the tail of what Devin printed.
4. **Models**: `devin models list --format json` (ids, labels, windows). The effort is in the model's name
   (`swe-2-max`), so no level is offered apart. The owner's choice for tests: `swe-2-max` (free, 262K).
5. **Updates**: `devin update` (it cannot say its newest version: "Run its updater").

## Verified

- Unit: the update parser, the checks (worktree only, read-only runs, other MCP servers refused), the local MCP file,
  the model list, the bridge's line range and tool naming.
- In MXC by hand (`security/sandbox/examples/contained`): logged in with `%APPDATA%\devin` read-only but `cli`; a
  file write handed to the client, a command run contained.
- In the app (QA root, `swe-2-max`, Solo): the chat locked the spec, approved the plan, created and assigned a ticket
  through Agenium's MCP tools. Two tickets were worked and closed. The work run used Agenium's tools (`write_file`,
  `shell`, `submit_report`) and its own contained `exec`, `read` and `grep`. `hello.py` was merged and runs. The
  local MCP file stayed untracked. A `Memory` run by Devin itself learned five facts.

## Consequences

- Devin needs a sandbox backend; without one it runs only if the user allows unsandboxed CLIs (ADR-117), and then the
  other tools' configurations it imports are visible to it (its own servers stay off).
- `%APPDATA%\devin\cli` is writable from the sandbox, its plugin list (`plugins/lock.json`) among it: a plugin a
  run put there would load in the user's own Devin. The run keeps the list as it was when it started, and a plugin
  added since (not one Devin's account manages, `scope: managed`) is taken out of the list again during the run and
  after it (`Driver::finish`, the entry only; logged). Devin's own files beside it are left to it.
- No per-run home on Windows (checked 2026-11-04 in MXC): with `APPDATA` pointed at a fresh home holding a copy of
  the login, Devin still read `C:\Users\<you>\AppData\Roaming\devin\config.json`; with `USERPROFILE` too, it could
  not find its cache folder. It locates its folders through the profile's Known Folders and only needs the variables
  to be there. Hence the read-only folder, `--config` and the servers turned off by name.
- In the sandbox the Python on PATH was the Store's alias (`WindowsApps`) and the Python install manager's launcher
  (which, unable to read its state, set out to install Python again). PATH in MXC now leaves `WindowsApps` out and
  takes the install manager's runtimes (`pythoncore-*`) in place of its launcher folder (`mxc::sandbox_path`).
- Files: `agent/cli-backend/src/devin/{mod.rs, bridge.rs}`; `driver.rs` (`acp`, `writable_roots`); `run/process.rs`;
  `updates.rs`; `foundation/vcs` (`LOCAL_EXCLUDES`); `orchestration/orchestrator/src/{model_lists.rs, cli_updates.rs,
  runtime/models.rs (`assume_cli`)}`.
