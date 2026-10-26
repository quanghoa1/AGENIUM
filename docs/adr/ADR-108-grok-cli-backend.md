# ADR-108: Grok Build CLI (`grok`) as a CLI backend

Status: Accepted (2026-10-26). Evidence: `agent/cli-backend/src/grok.rs` tests; golden tasks on grok 1.0.41 /
grok-4.7 (below). Amends ADR-56 (CLI backends).

## Context

The owner uses the Grok Build CLI and is signed in to it. Its headless mode is close to Claude Code's: `-p` /
`--prompt-file`, `--output-format streaming-messages-json` (the Anthropic Messages `stream-json` shape), allow/deny
rules, `PreToolUse` hooks, MCP servers in `config.toml`, and `GROK_HOME` to relocate its whole folder.

## Decision

1. **Turns:** one process per turn; the message goes into a file in the per-run home and `--prompt-file` points at
   it (headless mode does not read the prompt from stdin; the driver interface gained `prompt_file`). Later turns
   add `--resume <session id>` from the stream's `init` line. The stream is parsed with Claude Code's parser.
2. **Isolation:** `GROK_HOME` = the per-run home: Agenium's `config.toml` (Agenium's MCP server; its hook; Claude,
   Codex and Cursor compatibility scanning off; `[memory] enabled = false` and `GROK_MEMORY=0`; `cli.use_leader =
   false`, so no shared leader process outside the run) and a copy of the user's `auth.json` (a refreshed login is
   handed back as for Claude Code). The user's own `~/.grok` hooks, MCP servers (e.g. `gitnexus` seen in a plain
   run), skills and memory are not used.
3. **Control:** `--permission-mode bypassPermissions` with a `PreToolUse` hook on every tool that asks Agenium (edits
   inside the worktree, shell commands through the run's command gate, Agenium's MCP tools allowed, others denied,
   unknown tools denied). Only Agenium's tool list is offered (`--tools`): shell and its output/kill, read, write,
   search-replace, list, grep, to-do, the MCP dispatchers, and `spawn_subagent` for Solo only (`--no-subagents`
   otherwise). Web search is off (`--disable-web-search`); scheduling, monitors, workflows, plan mode, questions to an
   absent user, feedback and image/video generation are withheld.
4. **Fail closed although Grok's hooks fail open.** Grok lets a call through when a hook times out or crashes. The
   hook's timeout is 3 600 s and its own call to Agenium gives up 100 s earlier; when Agenium cannot be reached the
   hook writes `{"decision":"deny"}` and exits 2, which Grok always honours. On Windows the hook is a PowerShell
   script (Grok runs hooks through PowerShell there), elsewhere `sh` + `curl`.
5. Models come from `grok models`; the login state is reported as unknown (Grok has no status command).

## Verification

Golden tasks, grok-4.7 through `grok` 1.0.41 on Windows (2026-10-26, `agenium-eval --cli grok --approve-commands`):
**5/5 passed** (hello-file, fix-config-key, rename-function, readme-usage, csv-total), each with its work run and the
Lead's review run completed; 17–37 steps, 0.23–0.53 M tokens per task (`quality/eval/results/*-cli-grok-grok-4.7.json`).
The hook refused 1–3 calls per task, all reads outside the ticket worktree — the project folder above it and, twice,
files of the Agenium repository the eval was started from — and the agent finished without them. A plain run had shown
the user's `gitnexus` MCP server and Claude skills; the per-run home holds only Agenium's configuration (the `init`
line of an isolated run was not recorded, so what it listed is not verified here).

Not checked: the hook on macOS/Linux (`sh` script), a vendor-CLI Lead receiving a message during its run (ADR-104
covers API Leads only), login refresh hand-back (no refresh happened during the runs).
