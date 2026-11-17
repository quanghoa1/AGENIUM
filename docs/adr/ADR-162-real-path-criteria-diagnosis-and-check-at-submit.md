# ADR-162: Criteria on the real path, a diagnosis after a reopen, no hidden failures, the project's check at submit

Status: Accepted (2026-11-17). Owner, after a study of babysitter (`github.com/a5c-ai/babysitter`), an open-source
orchestrator for coding agents. Its `production-contract`, `diagnostic-first-phase` and `evil-fallbacks` rules and its
in-run convergence loop were studied; their ideas are taken, none of its code or text is used.

## Context

Most of babysitter's quality machinery is already in Agenium: criteria with evidence the reviewer checks (ADR-147),
checks tied to the files they ran on and a review in a fixed order (ADR-148), the project's pre-merge check (ADR-79), a
ticket reopened too often escalated to the user, approvals and a journal. Not taken: its 0-100 quality scores (mostly the
model grading itself: about 18.8k agent tasks against 66 shell tasks in its process library) and its ~2000 generated
skills. Four of its ideas close gaps Agenium still had:

1. **A fix can pass every test and leave the bug.** The test asserted on a mock of the part that was broken, or on a
   path the user never takes; the criterion said "tests pass", so the review agreed.
2. **A reopened ticket was worked like a new one.** The next run read the review and tried again; a second guess at the
   same bug is how a ticket reaches "reopened too often".
3. **A failure can be hidden instead of fixed.** A catch that swallows the error, a default that stands in for a result
   that never came, a retry that leaves no trace: the check goes green and the user meets the failure later.
4. **The project's check ran only in the merge queue.** A failure there reopened the ticket after the review: a review
   run and a new work run, the worker's context gone.

## Decision

1. **Every ticket has a criterion on the real path.** The Lead's rules (a Solo agent's too) and the planner's task:
   at least one acceptance criterion says what a user or caller observes ("running X prints Y", "a request to X returns
   Y"); "tests pass" is never one. The worker's rules: evidence for such a criterion runs that path, not a test of a mock
   of the part that was broken. The review order gains: evidence that runs only a mock, a stub or another path is a
   reason to reopen.
2. **After a reopen, diagnose first.** A work run on a ticket that was reopened (`reopen_count > 0`; its run starts a
   new conversation) is told, before it edits code: reproduce the failure on the real path or gather evidence; list two
   or three candidate causes, each with the observation that would rule it out; fix the cause the evidence points to.
   A first run is not told this, nor a run that resumes its own conversation (it was told already). The report gains
   `diagnosis` (how the failure was reproduced, the causes ruled out, the evidence for the one fixed), a list the strict
   tool form always asks, `[]` for none, as `beyond_ticket` and `tests_changed` (ADR-148); the review checks it, the
   ticket page and the handover show it.
3. **No hidden failures.** The worker's rules: never make a failure disappear by hiding it (a catch that swallows the
   error, a default value for a missing or failed result, a retry with no record), beside "never make a check pass by
   weakening it". The review order: new code that hides a failure is a reason to reopen. These are rules and review
   items, not a filter on the code.
4. **The project's check runs when the worker submits** (`runtime/submit_check.rs`). A worker's `submit_report` on a
   ticket whose worktree holds work (changes or commits since its base), in a project with `[checks] pre_merge`, first
   runs that check in the worktree, with the check the merge queue uses (the sandboxed runner, its 30-minute timeout).
   - It fails: the report is refused, and the tool result gives the worker the output's last 4000 characters in the
     same run (code `check_failed`, not an invalid call). After two refusals in a run the report goes through, and a
     system comment on the ticket says the check still fails, with its output, for the Lead to see.
   - It cannot run unattended (`CheckRunner::ready`: no sandbox backend, or the headless CLI's check that asks at the
     terminal): the submit is not blocked; a system comment says why the check did not run.
   - Each run of it is in the run's log (`check`, `use: submit_check`). The merge queue still runs the check (ADR-79).

## Verified

- `agenium_backend::prompt::tests::rules_ask_for_the_real_path_no_hidden_failures_and_a_diagnosis`.
- Scripted runs, real git (`orchestration/orchestrator/tests/runtime/quality_rules.rs`):
  `the_run_after_a_reopen_is_told_to_diagnose_first` (not on the first run, on the run after a Reopen, and the report
  keeps the diagnosis); `a_failing_check_refuses_the_submit_until_the_fix` (refused with the output, accepted once the
  worktree is fixed, in one run); `after_two_refusals_the_report_goes_through_marked_failing`;
  `a_check_that_cannot_run_does_not_block_the_submit`.
- Real model, through the runtime (`quality/eval/examples/quality_rules_real.rs`, a Solo agent on Claude Code `haiku`,
  effort low, Windows MXC sandbox, 2026-11-17). A Python project whose CLI reads only the first line of a file, while
  its unit test patches the reader and stays green; `[checks] pre_merge` runs the tests and a changelog check.
  1. A scripted "fix" backed only by a test of the patched reader: the reviewer ran the real command, saw `words: 3`,
     and reopened ("a unit test that patches read_text, which is not the real path").
  2. The worker on the reopened ticket diagnosed first (reproduced on the real path; ruled out the counter and the file;
     cause `readline()`), fixed it, submitted without a changelog line, was refused with the check's output in the run,
     added the line and submitted again: checks at submit "failed, refused" then "passed".
  3. The reviewer approved, the merge queue ran the check again and merged; the command prints `words: 5` on main.
  Three runs, every check passed in each (reviews 10-14 s, the work run 26-43 s, one refusal per run). In the second
  run the fixture's check also failed for a reason of its own (`tests/` was no package, so discovery could not import
  it); the worker named it in its diagnosis as a separate failure and fixed it. The fixture was corrected and the third
  run checks first that the check fails only on the changelog while the unit tests are green on the mock.
  Not run for real: the cap (two refusals, then the report goes through marked) and a check that cannot run, which
  the scripted tests cover; a team with a separate Lead (the Solo agent reviews with the same Lead rules).

## Consequences

- A submit in a project with a pre-merge check takes as long as the check; a slow suite slows each submit, once more
  per refusal (at most two).
- A worker meets the project's check while it still has the context, instead of after a review and a reopen.
- One more report field, empty by default; older reports read as before.
