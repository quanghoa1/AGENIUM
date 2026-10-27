# ADR-116: What a mixed real-model team run changed: sandboxed shell for hooked CLIs, generated files, the planner

Status: Accepted (2026-10-27). Evidence: `quality/eval/results/team-mixed-20261027.md` (Claude Opus Lead, Codex, Grok,
agy and DeepSeek workers, 4/4 tickets merged); tests named below. Amends ADR-56 (CLI backend), ADR-59 (planner),
ADR-64 (command gate) and ADR-44 (git).

## Context

The release check ran a team that mixes every kind of backend. Three things kept it from producing well-verified
work, though every ticket closed:

1. A vendor CLI checked through its hook (Claude Code, Grok, agy, OpenCode) runs its own shell commands on the host.
   ADR-64 sends each such command to the allowlist or the user. In a team run nobody answers each `ls`, so the agents
   stopped running anything: the Lead approved tickets by reading code, a worker parked for permission to run its
   tests. Codex was unaffected: it runs its commands in its sandbox.
2. Tool caches (`__pycache__`) were committed with the work and later tripped a ticket's write-scope check.
3. A Lead asked in chat to plan right after the spec was approved planned alone; the team's planner never ran.

## Decision

1. **Hooked CLIs also get Agenium's `shell`** (over the MCP server, beside `fetch` and `view_image`) when a sandbox
   backend works. It is the API agents' tool: the command runs in the sandbox with the run's policy (worktree writable
   in a work run, read-only otherwise), so under the `sandboxed` policy it runs without asking. The CLI's
   instructions say to run tests, builds and scripts through it; a refused host command says so too. The host shell
   keeps its rule (allowlist or the user) — nothing a CLI runs on the host is approved by this change.
2. **Generated files stay out of commits.** `LOCAL_EXCLUDES` (`__pycache__/`, `*.py[cod]`, `.pytest_cache/`,
   `.mypy_cache/`, `.ruff_cache/`, `.venv/`, `node_modules/`, `.DS_Store`, `Thumbs.db`) go to `.git/info/exclude` when a
   project is created and before each worktree is added (older projects get them too). Local to Agenium's clone: the
   project's `.gitignore` is not touched and tracked files stay tracked.
3. **The planner drafts plan v1 first.** With a planner in the team, `create_plan` is refused until its draft for that
   spec exists, unless the planner ended two runs without one since the spec was approved (the user is then told the
   Lead can plan). The refusal tells the Lead to say the plan is being drafted.
4. `agenium run` lists what still waits for the user in this project (`needs_you`), not the last five NeedsUser events
   of any project.

## Consequences

- `agenium_tools::shell_tool`; runtime adds it for `Control::Hook` CLIs when `agenium_sandbox::detect()` has a
  backend; `agenium_cli_backend::render(.., shell)`; `RunGate` deny text for a host command.
- `agenium_vcs::LOCAL_EXCLUDES`, `Repo::exclude_local` (was `exclude_agenium_dir`).
- `App::planner_gave_up`; `runtime::MAX_FAILED_RUNS` shared.
- Tests: `vcs.rs::generated_files_stay_out_of_commits`, `team.rs::planner_drafts_and_the_lead_creates_the_plan` (the Lead waits for the draft).
- `agenium doctor` no longer marks Windows as below recommended (the first release ships on Windows).
