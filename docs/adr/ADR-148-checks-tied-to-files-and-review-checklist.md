# ADR-148: A check is tied to the files it ran on; the report says what went beyond the ticket; the review has an order

Status: Accepted (2026-10-30). Owner ("ok làm đi"), after a study of JetBrains ThinkRail (`github.com/JetBrains/thinkrail`,
2026-10-29), an IDE around one coding agent. Its rules were learned; none of its code is used.

## Context

ThinkRail is not a team orchestrator; it isolates work in git worktrees like Agenium (a workspace per user task, where
Agenium has one per ticket and a merge queue) and is stronger on two points that bear on Agenium's own:

1. **What a run changed is read from the files, not the tool calls.** ThinkRail snapshots the worktree into a git tree
   (a scratch index, `write-tree`) when a run starts and when it settles. Agenium's verification record (ADR-147)
   found "the last change" from its edit tools only: a check that passed, then `sed -i`, a formatter or a generator
   run in the shell, still read as "passed after the last change".
2. **The completion report and the review are held to a shape.** A step's summary discloses scope drift ("also touched
   X because Y") and any test it weakened; an independent reviewer works in a fixed order — intent first (the
   signature failure is a right answer to a slightly different problem), undisclosed scope, CI gaming (weakened,
   skipped or deleted tests, `|| true`), what is missing (state set and never cleared on some path) — and every
   finding names a concrete failure.

Already in Agenium, so not taken: per-ticket worktrees and attribution by isolation, changes made outside Agenium
undone (ADR-70), a ticket reopened too often escalated to the user, a review run in a fresh conversation (also a Solo
agent's own review). Not taken: the IDE, its engine, a spec file beside every module (a larger decision of its own).

## Decision

1. **A check carries its tree.** When a run on a ticket runs a shell command that is a check (`is_check`), its log
   entry records the tree the ticket's worktree holds right after it (`App::worktree_tree`, the scratch-index snapshot
   of the worktree guard). A check runs "after the last change" only when no edit tool ran after it **and** its tree is
   the latest on record (a later check's, or what the last run left, `worktree_state`). Entries without a tree (older
   logs, a failed snapshot) are read as before.
2. **The report says what went beyond the ticket.** `WorkReport` gains `beyond_ticket` (anything changed beyond the
   ask, with why) and `tests_changed` (tests changed, skipped, loosened or removed, with why). The worker's rules ask
   for them and forbid making a check pass by weakening it; the review task, the ticket page and the handover report
   show them. In a strict tool's form (ADR-144) a list that defaults to empty is always asked, `[]` for none: the
   worker says "nothing" instead of leaving it out, and these lists cost none of Claude's 24 optional parameters (the
   two new fields had taken the seven strict tools to 26).
3. **The review has an order.** The Lead's rules (a Solo agent's too): does the change do what the ticket asked, all of
   it and only it; does the diff touch anything `beyond_ticket` does not list; was a test weakened or a check silenced
   without a reason it accepts; what is missing. Each is a reason to reopen, and reopen feedback names a concrete case
   that fails.

## Verified

- `runtime::verify::a_file_changed_by_a_shell_command_after_the_check_makes_it_stale` (scripted run, real git): the
  worker writes a file, runs a check, rewrites the file with `echo … >`, and cites the check; the approval on the
  report's word is refused ("ran only before the last change"), the reviewer's own word gets it through; the check's
  log entry carries a tree, the shell write does not. Run with the tree rule removed, it fails.
- `verification::tests::a_check_is_tied_to_the_files_it_ran_on` (stale after a shell change, passed when nothing
  changed, backed again by a later check on the final files).
- Real models (VERIFICATION.md, runs of 2026-10-30): the new opt-in golden task `fix-failing-test` (a failing
  unittest to fix without changing it) PASS on DeepSeek v4.1 flash (2 checks, both with their tree) and Codex
  gpt-6-luna over MCP (3 of 3); the report's lists were filled in the golden and team runs. `python -m unittest` was
  not a check before: `unittest` joined the check words.
- `strict::tests::a_list_that_defaults_to_empty_is_always_asked_and_costs_nothing`,
  `runtime::context::tests::the_strict_protocol_tools_fit_claude_s_limits_together` (failed with the two fields
  before this rule).

## Consequences

- A check costs one `git add -A` into a scratch index after it (the snapshot the worktree guard already takes at each
  run's end).
- A worker that formats or generates files after its last check is asked, at review, to run it again or the reviewer
  to say how it verified the work.
- Two more report fields, empty by default; older reports read as before.
