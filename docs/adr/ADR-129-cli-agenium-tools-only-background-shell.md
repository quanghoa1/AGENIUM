# ADR-129: Vendor CLIs work only through Agenium's tools; background commands for every agent

Status: Accepted (2026-10-28). Owner: "đảm bảo chúng gọi tool của agenium chứ không phải tool riêng của chúng nhé";
"chúng ta cần có tất cả tool cần để cli cũng như agent từ api gọi nhé". Amends ADR-117 (hooked CLIs kept their own
file tools in the worktree; Codex kept its own shell inside the sandbox) and ADR-84 (a contained CLI got only `fetch`).

## Context

The `find_code` A/B (ADR-128) showed the CLIs working with their own tools: Claude Code's `Read`/`Grep`, Grok's
`read_file`/`grep`, Codex's `command_execution`. ADR-117 had turned off a hooked CLI's own shell but left its file tools
on in the worktree ("the instructions prefer Agenium's"), and Codex, contained in Agenium's sandbox, worked entirely
with its own shell. So the same ticket ran under different rules depending on the vendor: write scope checked only at
the report, no trimmed outputs, no `find_code`, no Agenium logging of what was read.

Comparing the tool sets showed one capability missing on Agenium's side: a command that keeps running (a dev server to
check the work, a watcher). Claude Code (`run_in_background`), Grok (`get_command_or_subagent_output`), agy
(`command_status`) and Codex (`unified_exec`) all have one; Agenium's `shell` waited for the end.

## Decision

1. **Every vendor CLI gets the run's full tool set over MCP**, as an API agent does: `read_file`, `list_dir`, `glob`,
   `grep`, `find_code`, `edit_file`, `multi_edit`, `apply_patch`, `write_file`, `shell` (+ `shell_output`,
   `shell_stop`), `todo`, `fetch`, `view_image`, and the agent's MCP servers. In a read-only run (review, chat) the
   shell trio is added where a sandbox makes the run read-only.
2. **Its own tools are off**:
   - its own shell, everywhere: Claude Code and Grok are started with `--tools` lists without them; agy's and
     OpenCode's are refused by the hook / permission config; Codex runs with `shell_tool`, `unified_exec`,
     `view_image`, `sleep_tool` and `tool_suggest` off;
   - its own file and search tools reach **only its scratch folder** (the per-run home: Claude Code saves a long tool
     result there and reads it back, agy keeps its task notes, Grok its session notes), never Agenium's files there.
     On the worktree, or without a path (which means the worktree), the hook refuses them naming Agenium's tools;
   - OpenCode's file, search, shell and to-do tools are denied in its permission config.
   The instructions every CLI gets say so in one text (the per-backend variants are gone).
3. **Background commands** (for API agents and CLIs alike): `shell` with `background: true` starts the command and
   returns a job id; `shell_output {job, wait_ms}` reads what it printed since the last read and whether it ended;
   `shell_stop {job}` ends its process tree. Same sandbox, gate, environment and write scope as `shell`; at most 4 at
   a time, at most an hour, and every one is stopped when the run ends (the tool context is dropped). The sandbox's
   `CommandSpec` gained `stop` and `live` (output as it is read) on all three backends (plain, MXC, bubblewrap /
   Seatbelt).

4. **A Solo agent's subagents** (owner: "verify việc solo agent gọi subagent thì trong agenium hiển thị như thế nào"):
   - the CLI's own subagent tool stays on for Solo agents, and the instructions say so (after rule 2 Codex took "not
     your own" for its `spawn_agent` and answered alone; Grok looked for one among Agenium's tools);
   - Claude Code 2.1 names its subagent tool `Agent` (`Task` before): the old allowlist refused every Solo agent's
     subagent; both names are allowed, with `TaskOutput`; Grok keeps `get_command_or_subagent_output` and
     `kill_command_or_subagent` for its subagents (its own commands are off, so they reach only those);
   - the run's log names the subagent of each step (Claude Code's `parent_tool_use_id`): the step that starts one is
     logged when it starts, labelled `subagent N · <description>`, and every step the subagent makes — its own tools and
     its calls of Agenium's tools over MCP — carries that label (`_subagent` in the logged arguments). The run page and
     the Live tab indent those steps under a violet label. Subagents follow the same tool rules (their own tools are
     refused on the worktree; they move to Agenium's);
   - a call the hook refused is logged once, under its own name, as refused (it was logged as `cli:permission` and
     then a second time as if it ran).
5. **Arguments under another harness's names** are renamed to the schema's when the schema's is absent and the other
   is not a field of it (`file_path`, `TargetFile`, `Path` → `path`; `new_str`, `replacement` → `new_string`; `query`
   → `pattern`; `CommandLine` → `command`...), for CLI calls as for API calls (ADR-105's repairs now run on the MCP path
   too). Grok's calls of Agenium tools through its `use_tool` dispatcher were logged twice; now once.

What does not change: the hook still checks every call a hooked CLI makes; subagents only for Solo agents; web only
through `fetch`; Codex itself still runs inside Agenium's sandbox (or on the host by the user's choice, ADR-117).

## Consequences

- `agenium_cli_backend::driver::{own_tool_off, AGENIUM_TOOLS}`; `file_access` allows the per-run home only;
  `run::render` has no `Commands` variants; `agenium_tools::{jobs, ShellOutput, ShellStop, shell_tools}`;
  `agenium_sandbox::{CommandSpec::{stop, live}, Live}`.
- Tests: `claude.rs`, `grok.rs`, `agy.rs`, `opencode.rs` (own tools refused on the worktree and without a path, its
  scratch folder open), `stream_run.rs` (a fake agy writes through Agenium's `write_file`, its own write and shell are
  refused, its notes are written; the user's messages ride on Agenium tool results and open the next turn),
  `tools.rs::a_background_command_is_read_while_it_runs_and_stopped`.
- The upstream Codex sandbox was brought to f021168 the same day (MXC's object normalizer no longer called; separate
  commits, `docs/reference/SANDBOX-MAINTENANCE.md`).

## Verified on real models (2026-10-28)

Golden tasks (`agenium-eval`, five single-ticket tasks, Solo agent) and the team task (Lead and planner on DeepSeek
v4.1 Flash over the API, the workers on one CLI), with the models the owner set for tests: Claude Code Haiku 5.5,
Codex gpt-6-luna, Grok 4.7, agy Gemini 3.8 Flash. The eval now counts tool calls by name (`cli:<name>` = the CLI's
own).

| | before (ADR-117) | after |
|---|---|---|
| Solo golden tasks | 25/25 (with DeepSeek over the API) | 20/20 on the four CLIs |
| Team task | 4/4 | 4/4 |
| The CLIs' own tools in the work | Claude `Read`/`Grep`/`Write`, Grok `read_file`/`grep`, Codex `command_execution` | none: Codex only Agenium's; Claude only `ToolSearch` (loads Agenium's; since ADR-158 none: Agenium's tools are listed); Grok only its MCP dispatchers; agy reads Agenium's tool schemas in its scratch folder; every attempt on the worktree refused and redirected |

The workers used the background commands to check their pages (`shell_output`, `shell_stop`). Found while verifying,
and fixed: an API Lead's review run was offered `shell_output` without `shell` (it guessed job ids); the other
harnesses' argument names cost a step each; Grok's dispatched calls were logged twice; Claude Code's `Agent` was
refused; Codex and Grok took "not your own" for their subagent tool; Grok's `workflow` (tried, then allowed, then
withheld again) answers after the turn has ended.

Subagents, Solo agents on a pinned Flask 3.1.0: Claude Code started two in parallel; the run page shows both starts and
each subagent's steps labelled and indented under them, its own tools refused on the worktree and Agenium's
`grep`/`read_file` used instead. Codex (gpt-6-luna, low) has its `spawn_agent` (checked in its session) but answered
alone; Grok and agy did the work themselves. Codex's own `apply_patch` cannot be withheld by configuration in codex
0.163 (it belongs to the model family); it runs inside Agenium's sandbox, the write scope is checked at the report, and
it was not used in these runs.

Found in the full verification after the code review (same day), and fixed:

- **A step whose hook never reached Agenium was logged as if it ran.** Six CLIs in parallel loaded the machine; agy's
  hook calls then failed before reaching Agenium (agy blocks the tool when its hook fails), and the end of each blocked
  `run_command` was logged as a successful step (18 in one task, ~1 100 s). A hooked CLI's own step is now logged as
  run only when Agenium let it through; otherwise it is logged as not run, with the reason.
- **agy's reads of Agenium's MCP resources** (`agenium://schemas/...`) were refused as worktree paths: a URI of another
  scheme than `file://` names no file, and MCP resources of Agenium's own server are allowed.
- **A CLI's used-up subscription** ("Free usage limit reached" from OpenCode's free model, a usage limit from Codex or
  Claude Code) ended the run as a failure, retried once; it is now a quota, as for an API key: the agent is parked
  and the user told.
- The refusal note told a CLI to "use your file tools"; it now names Agenium's tools.
- **OpenCode with a paid model**: the driver ran only OpenCode's free models (no login in its isolated home), and the
  free allowance ran out during the verification. A model of a provider the user signed in to in their own opencode
  (`opencode-go/deepseek-v4.1-flash`, an OpenCode Go subscription) now gets that provider's key from the user's
  `opencode.db` (read-only), through the environment; free models still get none. opencode 2's code-mode `execute`
  only runs other tools (each checked: Agenium's over MCP, its own through the permission ask), so it is logged as
  run. Golden tasks on OpenCode with OpenCode Go: 5/5.

Final verification on the reviewed code: golden tasks 5/5 each on DeepSeek over the API, Claude Code Haiku 5.5, Codex
gpt-6-luna, Grok 4.7, agy Gemini 3.8 Flash and OpenCode (OpenCode Go DeepSeek v4.1 Flash); team task 4/4 (workers on
each hooked or contained CLI); workspace tests 498 passed, 0 failed.

## Probed again (2026-10-28, late): what each CLI actually has in an Agenium run

Each CLI was asked, in a chat turn of a Solo agent, which tools, skills, instruction files and MCP servers it has, and
then to use one of its own tools Agenium does not allow:

- **None of the user's own configuration reaches a run**: no `CLAUDE.md`/`AGENTS.md`/`GEMINI.md`, none of the user's
  MCP servers (gitnexus) or skills (gitnexus-*, hermes); MCP servers are Agenium's only (the agent's own bindings are
  reached through it), and they start with a cleared environment.
- **Refused as designed**: Grok's `todo_write` and `run_terminal_command`, agy's `search_web` (each with the way through
  Agenium's tools). A tool a CLI lists is not one it may use: Grok listed its image, video and feedback tools, which its
  hook refuses like any tool outside the run's list.
- **The instructions now name only the tools a run offers.** A Lead's chat turn was told of `edit_file`, `write_file`
  and `submit_report`, which a chat run does not have; Claude Code reported it as a contradiction. A run without write
  tools is now told it changes no files.
- **Codex**: its skill search, the MCP servers its skills would install, and its own git worktrees are now off as well
  (`skill_search`, `skill_mcp_dependency_install`, `worktrees`). Its bundled system skills (`imagegen`, `openai-docs`,
  `skill-creator`, `skill-installer`) are still listed: they live in the run's home and go with it.
- **OpenCode 2's code mode offers its own management tools inside `execute`** (`opencode.worktree_create`,
  `worktree_list`, its sessions and models), outside both `permission` and `policies`: a probe created a worktree. Not
  blocked yet; contained instead: OpenCode's worktree directory is the run's scratch home, so a worktree it makes goes
  with the run (no branch, a detached HEAD), and the registration it leaves is pruned the next time Agenium removes a
  ticket's worktree.
