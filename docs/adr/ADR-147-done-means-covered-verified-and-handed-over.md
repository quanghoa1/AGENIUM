# ADR-147: Done means covered, verified and handed over; nothing lost between a decision and its effect

Status: Accepted (2026-10-30). Owner, on an external review of commit `194274a` ("cái nào bạn thấy không ổn thì sửa hết
và kiểm chứng verify lại giúp tôi"). Every finding of the review was confirmed in the source first; each fix below
has a test.

## Context

The review measured Agenium against its goal — a team that completes the work it was asked for and reports it in
full — and found ways the system could say "done" early, lose progress, or learn from the wrong evidence:

1. **Coverage.** A plan was checked only for items tracing to requirements that exist, not for every requirement
   having an item. A ticket only had to name a plan item; its dependencies were the Lead's free choice. A plan item
   was done when one of its tickets closed and the rest ended — a cancelled half of the work counted.
2. **Evidence.** A criterion marked Met needed no reviewer's word; its evidence was free text. Nothing tied a cited
   check (`cargo test …`) to a run that passed after the last change.
3. **Recovery.** A run that died with the process left no record of its worktree; the guard before the next run took
   the previous run's state as the reference and undid the interrupted run's edits as "made outside Agenium".
4. **The user's answer.** In a team the answer to a worker's question goes to the Lead; the desktop acknowledged it,
   then ran the Lead's turn in the background. Until that turn ended the message lived in memory only.
5. **Panics.** A run ended on an error path only; a panic, or a dropped future, left the run open (its agent Working,
   its ticket skipped) until the app restarted.
6. **Memory attribution.** A continuation run recalled memories and recorded them as used, but showed none (they go in
   a conversation's first message only); the close credited — even validated — lessons the agent never saw, and not
   the ones it did.
7. **Created skills.** A revision of a skill the agent made skipped the judge's contradiction read (its preview needed
   a library or role base) and kept no earlier text to undo.
8. **Learning trigger.** Learning ran after the Lead's review run only: a ticket the user approved, closed or reopened
   on the board taught nothing.
9. **Chain verdict.** The memory-chain scenario passed on tickets 1 and 2; tickets 3–4 and the "library skill
   unchanged" invariant were recorded, not judged.
10. **Report.** The project report was a status snapshot (requirements, item ticks, a ticket table); its author column
   came from the current team, empty once the project closed. A project completed without any handover.

## Decision

1. **Coverage** (`coverage.rs`).
   - A plan (`create_plan`, `submit_plan_draft`, `revise_plan`) must trace every requirement of its spec.
   - A ticket of an item that depends on another item is refused until that item has a ticket, and its open tickets
     must be among the new ticket's dependencies.
   - A cancel is recorded with how its work is accounted for (`ticket_cancel`, core migration 5): the user's cancel is
     a waiver; the Lead's names `replaced_by`, or a later ticket that `follows` it becomes its replacement; otherwise
     the user is asked (Needs you), the Lead's inbox lists it, and the ticket page offers **Waive** (`agenium ticket
     waive`). An item is done when each of its tickets is closed or accounted for (a replacement that closed, or a
     waiver), at least one by work; an item whose tickets were all waived is "waived". A project is Done only when
     every item is, and its plan covers every requirement of the agreed spec. Tickets cancelled before the migration
     count as waived, so finished projects stay finished.
2. **Verification records** (`verification.rs`). From the runs' logs, never the report: the commands run after the
   last change with their exit codes, and per criterion what backs its evidence — a cited check (a code span that reads
   as a command) passed after the last change, failed, ran only before it, never ran, or no check cited (judgement).
   Approving a Met criterion whose cited check is not backed needs the reviewer's own word, like a Partial one; the
   record is in the review task and the report. The report form gains `checks_not_run` (with why).
3. **Recovery.** Reconcile records the worktree state an interrupted run left, as the run's own; the next run goes on
   from it. A change made by hand after the restart is still undone and kept as a patch.
4. **Durable messages to the Lead** (`lead_messages.rs`, `lead_message`). A message is stored before it is
   acknowledged; it is delivered once a run of the Lead had it in its conversation. After a restart, undelivered
   messages open the Lead's next turn. The desktop stores an answer before it answers "routed to the Lead".
5. **Runs end on a panic or a drop.** Every entry point holds its run in a guard (`OpenRun`) that ends it as
   Interrupted when the entry point does not: its agent goes Idle and its ticket gets a continuation without a restart.
6. **Attribution.** A continuation run recalls and records nothing; the episode credits the memories of the run that
   started the conversation (the one that showed them).
7. **Created skills.** Their revision is previewed and read by the judge like an overlay; whether the judge read it
   (read, not read, skipped) is logged. Each revision keeps the text before it; the user can undo the last one.
8. **Learning owed.** A transition to Closed or Reopened records a learning row in the same transaction
   (`learn_pending`); each round (and a project that is no longer running) pays it, once per ticket state.
9. **Chain verdict.** Every ticket closed and correct, and the library skills unchanged.
10. **Handover report.** `project_report_md` is the handover: coverage per requirement; per ticket who worked on it
   (from its runs), the result per criterion with its evidence and verification, checks run and not run, artifacts,
   the review and who made it (a Solo agent's own review is said to be one), the commit, the cancel and how it is
   accounted for; open risks and follow-ups; usage. A project becomes Completed only after its report is written as
   a new version (`<root>/reports/<project>/handover-vN.md`), and the user is told where it is.

## Verified

Tests (the ones marked † were also run with their fix removed, and failed):

- `reconcile::an_interrupted_runs_edits_survive_the_restart` †
- `runtime::failures::a_panicking_or_dropped_run_is_ended_without_a_restart` †
- `runtime::memory::a_ticket_the_user_closes_teaches_the_assignee` † (and the Completed handover)
- `runtime::messages::a_message_to_the_lead_survives_a_stop_before_its_turn`
- `lifecycle::coverage::*` (a plan must cover every requirement; a Lead's cancel waits for a replacement that closes;
  the user's cancel or waiver accounts for the work; the handover report is complete and outlives the team)
- `lifecycle::review::a_cited_check_the_logs_do_not_back_needs_the_reviewers_word`,
  `dependencies_unlock_on_approve_and_merge_not_close` (plan dependencies)
- `verification::tests` (cited checks, backing), `memory_reuse::tests::a_chain_fails_on_any_ticket_or_a_changed_library_skill`
- `team::skills::an_agent_creates_a_skill_only_through_every_gate` (preview, undo of a revision)
- `store::core::v1_database_with_data_migrates_to_the_latest` (legacy cancels waived)

Real models (VERIFICATION.md, runs of 2026-10-30): the default golden set 5/5 on DeepSeek v4.1 flash and Codex
gpt-6-luna (twice), the team task, the memory chain and the learning loop on DeepSeek, all PASS; every golden and team
project ended Completed with `handover-v1.md`. The eval did not complete projects before (its rounds stop at the last
ticket); it now runs the app's loop once a project is done, and records each task's checks, report lists and handover.

## Consequences

- A Lead that creates tickets out of plan order, or cancels without a replacement, is told what to do; the user may
  have to waive a cancelled ticket before a project completes.
- A reviewer approving a criterion whose cited check never ran must say how it verified it. A worker that cites the
  command it ran is approved without that step.
- A message to the Lead may, after a crash, be read twice; it is never lost.
- Still not guaranteed: that a learned skill improves the work (no baseline-vs-candidate gate, ADR-118), or that the
  judge reads every change (it is optional and fail-open; now logged).
