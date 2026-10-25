# ADR-70: Scheduler aging; outside edits to a worktree are undone (decision 19)

Status: Accepted (2026-10-25, M4 rest; architecture "Scheduler & vận hành" and decision 19).

## Decision

1. **Aging.** The scheduler orders ready tickets by an effective priority: one level higher for every
   `[runtime] aging_hours` (default 24, 0 = off) since the ticket was created, never above P0, then by age. A P2 that
   waited a day ranks with a fresh P1 and, being older, goes first. The idle pool stays as built: the Lead assigns,
   and a run starts only when the assignee is Idle.
2. **Worktree guard (decision 19).** While a project runs, ticket worktrees and `agenium/*` branches belong to the
   agents.
   - When a run on a ticket ends, Agenium records what it left: HEAD and the tree of every file (uncommitted and
     untracked included), written with a scratch index in the worktree's git dir so the real index is untouched
     (`worktree_state` in the run log).
   - Before the next work or review run on the ticket, a difference is a change made outside Agenium when HEAD did
     not move, or the branch got commits without Agenium's `Agenium-Intent` trailer that main does not hold.
     Agenium's own catch-up or rebase is not one (the guard then only cleans files changed on top of the new HEAD).
   - The change is undone (branch and files back to the recorded state; the agent's own uncommitted work stays),
     saved as a binary patch in `<root>/recovered/<project>/<ticket>-<time>.patch`, and reported with a System
     comment on the ticket and a NeedsUser notification. The user changes the work through the ticket or the Lead;
     `main` is edited by hand only while the project is stopped.
3. **Not taken:** a file watcher. Checking at the run boundaries catches the same edits without a watcher per
   worktree, and nothing reads the worktree between runs.

## Evidence

- `sched::tests::waiting_raises_the_priority_up_to_p0`; `vcs.rs::outside_changes_to_a_worktree_are_detected_and_undone`;
  `team.rs::outside_edits_to_a_worktree_are_undone_before_the_next_run`.
- A real golden task (opencode CLI, free model) after the change: PASS, no false detection across work, review and
  merge.
