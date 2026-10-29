# ADR-56: CliBackend = headless CLI in its YOLO mode, per-run home, permissions decided by Agenium

- **Status:** Accepted; implemented for `codex`, `agy` and `claude` (golden tasks through each, see Verification)
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
   - claude: `CLAUDE_CONFIG_DIR` = per-run home with Agenium's `settings.json` (its hook only), MCP config file
     (`--strict-mcp-config`) and a copy of `.credentials.json` (handed back when refreshed, like codex);
     `--setting-sources user` loads only that folder's settings (not the repository's). Verified: none of the
     user's MCP servers or plugins loaded, the user's `~/.claude.json` untouched. macOS keeps the login in the
     Keychain: not verified there.
   - agy: `USERPROFILE`/`HOME` = per-run home (agy's configuration lives in `<home>/.gemini`, its login in the OS
     credential store) with Agenium's `mcp_config.json`, `hooks.json` + hook script and `settings.json`.
3. **YOLO mode, Agenium decides** — two ways, chosen per CLI (`Driver::control`):
   - **Sandbox** (codex): the CLI runs with `--dangerously-bypass-approvals-and-sandbox` inside Agenium's sandbox
     (`agenium_sandbox::service::spawn_contained`: MXC on Windows, bubblewrap on Linux, Seatbelt on macOS), worktree
     + per-run home writable, toolchains and the CLI's installation readable, network on for the vendor API. No
     sandbox backend = the run is refused (fail-closed).
   - **Hook** (agy and claude; agy's login is in the OS credential store, which a container cannot reach;
     Claude Code's Bash tool needs Git Bash, which does not start in an MXC container): the CLI runs with
     `--dangerously-skip-permissions` on the host; its `PreToolUse` hook (in the per-run home) posts every tool call
     to Agenium's loopback server (`POST /hook`, same per-run bearer token as MCP) and Agenium answers allow/deny:
     shell commands go to the run's command gate (allowlist or the user's approval; YOLO does not approve them because
     they run outside the sandbox — `RunGate.contained = false`), file arguments must stay inside the worktree (or
     the per-run home), only Agenium's MCP server may be called, web and browser tools are refused. A hook that fails
     (Agenium unreachable, bad answer) blocks the tool. claude runs hooks in a POSIX shell and blocks only on exit
     code 2: its hook script exits 2 on any failure; agy's is a `.cmd` (its `cmd /c` mangles quoted arguments).
4. **Agenium's MCP server** (`agent/mcp/src/server.rs`: loopback, per-run bearer token, `Origin` must be loopback)
   offers the protocol tools and the agent's own MCP bindings; it is the only MCP server the CLI sees.
5. **Limits and mandatory report** from the event stream: tool steps, repeated steps, wall-clock timeout (the process
   is stopped -> `Checkpoint` / `LoopDetected`); a work turn that ends without a protocol tool gets a nudge (2 at
   most) -> `NoProgress`; after `submit_report` / `ask_question` / ... the CLI gets 30 s to finish its turn.
6. ~~**API fallback (ADR-30)**~~ (withdrawn by [ADR-132](ADR-132-no-automatic-fallback-resume-the-agent.md)): when
   the agent's CLI cannot run (unknown, not driven, not installed, older than the checked version, logged out, or no
   sandbox backend for Codex), the agent is set aside and the user is told why; the user fixes the CLI or gives the
   agent another model, then resumes it. `[cli] fallback_provider/fallback_model` is only the API model that draws a
   CLI agent's lessons.
7. **Contract per version:** each driver's stream parsing is unit-tested against lines captured from the minimum
   version in `CliKind::min_version` (codex 0.163.0, agy 1.4.4); an older CLI is not used.
8. **Agent setting:** `backend = Cli`, `model.provider` = CLI program, `model.model` = the CLI's model
   (`agenium agent cli <name> --cli codex --model gpt-6-luna`). Cost 0 (subscription), tokens recorded.

## Per-CLI status (2026-10-24)

| CLI | Headless stream | Per-run home | Control | Status |
|---|---|---|---|---|
| codex 0.163.0 | `exec --json -` / `exec resume` | `CODEX_HOME` | sandbox | driven, golden 5/5 (gpt-6-luna) |
| agy 1.4.4 | stream-json in/out | `USERPROFILE`/`HOME` | hook | driven, golden 5/5 (gemini-3.8-flash-low) |
| claude 2.1.310 | `-p` stream-json in/out | `CLAUDE_CONFIG_DIR` | hook | driven, golden 5/5 (haiku) |
| opencode 2.1.2 | `serve` + HTTP API (ADR-68) | all `XDG_*` and opencode's home in the per-run home | Agenium answers `permission.asked` (hook endpoint) | driven since 2026-10-25 for OpenCode's free models (ADR-68); paid OpenCode models stay on the API backend |
| devin | `-p` prints text only | none (always loads the user's MCP servers and hooks) | — | not drivable |

## Findings that shaped the design

- codex without YOLO and with no user config blocks writes, commands and MCP calls in `exec` (nobody can approve).
- agy headless without YOLO refuses anything not in `permissions.allow` and ends the turn; its hooks run in YOLO mode
  too, before every tool; a hook command is run through `cmd /c`, which mangles quoted arguments (hence the script).
- agy inside an MXC container is not logged in (the credential store is out of reach); codex is, with its login file
  in the per-run home.

## opencode 2.1.2 findings (CLI dropped on 2026-10-24, driven again since ADR-68)

Decision (owner, 2026-10-24): the opencode CLI is not driven. The same subscription works through the API backend
(OpenCode Go relay, ADR-52 §12-13), where Agenium runs the loop and holds every permission itself. This narrows
decision 3c ("Opencode via API or CLI") to API only. The findings below are kept in case it is revisited.

- Headless works (`opencode run --standalone --format json -m <provider/model>`, events `step_start`, `tool_use`,
  `text`, `step_finish` with tokens); the working directory comes from `PWD`; `--standalone` avoids the user's
  background service.
- Isolation: `XDG_CONFIG_HOME`/`XDG_STATE_HOME`/`XDG_CACHE_HOME` per run drop the user's config, plugins and skills;
  the login is in the data folder (`opencode.db`), so the data folder (and the session history) stays the user's.
- Control: no server-side plugin hook was found in this version (`tool.execute.before` is gone; a plugin registered
  in `opencode.json` was not loaded). `--auto` + `permission` deny rules work (`bash`, `external_directory`), but the
  code-mode `execute` tool runs JavaScript with network access (it fetched a page despite `webfetch: deny`); denying
  `execute` closes that, but Agenium's MCP tools are then not reachable even with `codemode: false`; the `bash`
  rule `echo hi` also let `echo hi ;` through (prefix-like matching), so exact allowlist entries are not exact.
- Next option: drive `opencode serve` (v2 API) and answer `permission.asked` events per call instead of `run`.

## Open

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
  `gemini-3.8-flash-low` 5/5 (`20261024T060626Z-cli-agy-gemini-3.8-flash-low.json`), claude `haiku` 5/5
  (`20261024T065834Z-cli-claude-haiku.json`). Those runs often needed two review runs; fixed in M3 (below): claude
  `haiku` 5/5 again with one work run and one review run per task, ~25 % fewer tokens
  (`20261024T090226Z-cli-claude-haiku.json`).

## Review in one run (M3, 2026-10-24)

- Cause (traced with `AGENIUM_EVAL_TRACE=1`): review runs ran in chat mode, so a text-only turn ended them, and the
  `review_ticket` result said `merge_queued: true, status: Resolved` (computed before the merge ran). Haiku read that
  as "wait for the merge" and stopped; the next round started a second review that closed the ticket.
- Fix: the approval result carries the real merge outcome (`merged`) and the next step (`next`: close, wait for the
  user's merge review, or the merge failed/is retried); review runs are nudged like work runs (review-specific
  text) instead of ending on text. A review run still ends by itself once the ticket leaves Resolved or waits for a
  merge.
- Journaled git operations that fail during a run (on Windows `remove_worktree` while the CLI process still has
  its working directory in the worktree, os error 32) are replayed at the start of every scheduler round, not only
  at startup.
- Memory extraction is an API call: a CLI agent uses the `[cli] fallback_provider/fallback_model` for it (its only use
  since ADR-132); without one the pipeline is skipped with an info log (it used to fail with "provider claude not
  found").

## Subagents and withheld tools (M4, 2026-10-24)

- Owner's rule: **no subagents in team mode; only Solo agents may call agents** (a team delegates by tickets through
  the Lead). `CliRun.subagents` = the team is Solo.
- claude 2.1.310 announces many tools beyond code work (`CronCreate`, `RemoteTrigger`, `PushNotification`,
  `PublishPlugin`, `ScheduleWakeup`, `EnterWorktree`, `Workflow`, `Skill`, ...). The hook used to allow every tool not
  denied explicitly; it is now an **allowlist**: `Bash, PowerShell, Read, Write, Edit, NotebookEdit, Glob, Grep,
  ToolSearch` (+ `Task, SendMessage, ListAgents, TaskStop` for Solo), and the same list is passed as `--tools` so the
  model never sees the rest (verified: the init event lists exactly these).
- agy 1.4.4 names some tools differently in its hook (`write_file`) and its init event (`write_to_file`), so it keeps
  a deny rule set, extended to subagents (team), its own scheduling/messaging/inbox/marketplace/plugin/image/knowledge/
  workflow tools, `notebook_execution` (code outside the command gate) and its interactive `ask_*`/permission tools
  (pointed to Agenium's `ask_question`).
- codex 0.163 has no hook here (sandbox control): the per-run `config.toml` sets `web_search = "disabled"`,
  `[features] multi_agent = <solo>` and turns off `browser_use`, `browser_use_external`, `computer_use`,
  `in_app_browser`, `apps`, `plugins`, `remote_plugin`, `plugin_sharing`, `image_generation` (keys verified with
  `codex features list` and a config parse error listing the `web_search` variants).
- Real runs after the change: `fix-config-key` PASS with claude haiku, codex gpt-6-luna and agy gemini-3.8-flash-low.

## Reviewed again (2026-10-29)

An independent review of the crate, and a task whose command runs 80 seconds (`agenium-eval --task slow-check`, opt-in)
on every CLI, found these; each is fixed:
- **Claude Code's hook command did not quote its path.** A per-run home with a space (a profile folder such as `John
  Smith`) made `sh` exit 127, which Claude Code takes for a hook error that does not block: every tool ran without
  Agenium's decision. The path is quoted (`hook_command`).
- **A CLI gave up waiting for a long Agenium tool call.** Claude Code waited less than 80 s: its report said "the run's
  stdout was lost to the shell timeout", and an earlier run started the check a second time and then asked. Every CLI
  that can be told is given one hour (`MCP_CALL_TIMEOUT`: a command lasts 10 minutes at most, an approval of one 30):
  Claude Code's server `timeout`, Codex's and Grok's `tool_timeout_sec` (Codex waited the 80 s anyway; Grok's own
  default is 6000 s). agy and OpenCode have no such setting and waited for the 80-second command. Claude Code then
  waited for it and passed.
- **A dead OpenCode server was waited for until the 45-minute deadline**: the end of its event stream ends the turn
  with an error now. A CLI process gone while its output stays open (a process it started holds the pipe) is noticed
  after 15 s.
- **The run's MCP server stopped accepting after one failed accept** (every later hook refused), and the run loop spun
  on a closed channel: both fixed.
- **One stdout line that is not UTF-8 ended the reader**, and the run was told the CLI had exited: lines are read as
  bytes; lines that are not JSON are kept with the stderr tail, where they explain a failure.
- **A used-up Claude Code subscription** ("5-hour limit reached", "You've hit your limit") was retried as a failure; a
  lost login said nothing. See ADR-131.
- **Two runs handing back a refreshed login at once** wrote the same temporary file, and a login written halfway could
  reach the user's: each hand-back writes its own file, and a JSON login that does not parse is kept out.
- **Per-run homes left by a crash**, each with a copy of a login, are removed once a day old.

Then, the same day (ADR-132 to ADR-134 and the path review below):
- **One CLI account in several runs at once**: logins are kept in step while runs go on (ADR-134), not only handed back
  at a run's end, so a run started meanwhile no longer holds a used refresh token.
- **A command longer than a CLI's tool wait** goes on as a job the run waits for (ADR-133); agy and OpenCode, which
  have no tool-wait setting, no longer depend on waiting.
- **Every CLI resumes its session of a ticket** across the ticket's runs (ADR-134).
- **Dead code removed**: `Check::Command` (a hook's shell command put to the run's command gate) could no longer be
  reached once ADR-129 turned the CLIs' own shells off; it went with the `RunGate.contained` flag it needed.
- **Path review** (crafted paths meant to slip past the checks), each fixed and tested:
  - a path under an argument name no driver listed, in a list, as `FILE:///`, or percent-encoded (`%2F..`) in a
    `file:` URI was not checked: every string of a CLI tool's arguments that names a path is (`named_paths`), and a
    search pattern that starts outside the folder it searches is refused;
  - agy's resource tool read any MCP server's resource and any `file:` URI: Agenium's own server only, URIs checked;
  - a CLI's own tools could write the files of its scratch folder that other programs read as the user's (the folder
    is their `HOME`): a planted `.gitconfig` ran a command through git. Hidden entries there are refused (agy's
    `.gemini` excepted), and the git a CLI runs reads no global or system configuration;
  - a link inside the worktree (`src/auth/l -> ../billing`) took a write out of the ticket's write scope: the scope is
    checked on where the path really leads too;
  - a folder of the worktree turned into a link between the check and the write (Agenium's tools run outside the
    sandbox) took the write outside the worktree, and a hard link carried it to the file's other names: the file is
    opened first and written only when the open handle is inside the worktree and has no other name (Windows and
    Linux);
  - on Windows a Unicode case pair (K and the Kelvin sign) made a twin folder beside the root pass as inside it: names
    are compared with ASCII case folding only.

Known and left: the Windows job object is attached right after the CLI starts, so a process it starts in that instant
escapes it; macOS has no check of the open handle's path (writes and reads still go through the path checks). Reads
through a hard link are refused since ADR-135.
