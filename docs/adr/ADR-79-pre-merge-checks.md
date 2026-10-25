# ADR-79: Pre-merge checks per template, run in the sandbox

Status: Accepted (2026-10-25, M5).

## Decision

A team template may carry a `pre_merge` command (e.g. `cargo test`). Creating a team from it writes the command to
the project manifest's `[checks] pre_merge`; the template wizard edits it, and a template made from a team keeps it.
The merge queue already runs the project's `pre_merge` before a fast-forward and reopens the ticket with the output
on a non-zero exit; what was missing was a check that actually runs it in the real app.

`SandboxCheckFactory` (used by the desktop and non-interactive runtimes) runs the command in the ticket worktree
**inside Agenium's sandbox** (writes stay in the worktree, no network) with a 30-minute timeout; with no sandbox
backend it is refused, fail-closed, since Agenium does not run shell commands unsandboxed on its own. The headless
CLI keeps its own check that asks the user before running (`apps/cli`).

## Consequences

- `TeamTemplate.pre_merge`; `create_team_from_template` and `template_from_team` carry it; the manifest's
  `[checks] pre_merge`. `agenium_orchestrator::check::SandboxCheckFactory`; the desktop uses it instead of
  `NoCheckFactory`. Template wizard: a pre-merge field.
- Test: `team.rs::a_template_sets_the_projects_pre_merge_check`.
