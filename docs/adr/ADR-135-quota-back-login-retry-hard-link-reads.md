# ADR-135: An agent goes back to work when its quota is back; a login refreshed elsewhere is taken; no read through a hard link

Status: Accepted (2026-10-29). Owner: "xử lí các việc còn lại đi ngoại trừ macOS … verify trên ubuntu vmware luôn" (the
limits left by ADR-132 to ADR-134 and the path review). Amends ADR-132 (Resume is the user's step), ADR-134 (kept
sessions, shared logins) and ADR-56 (path review).

## Context

What the last round left:
- **The old vendor's quota coming back did nothing.** An agent set aside for a used-up subscription window (Claude Code's
  5 hours, Codex's, OpenCode Go's) waited for the user to press Resume, though the window resets by itself.
- **Two runs refreshing one login at the same instant** (within the 5 s of the login sync, ADR-134): the later run used
  a refresh token the other had just used, and its turn failed on the login.
- **Reads followed hard links.** A hard link inside the worktree is another name of a file that can be anywhere on the
  disk; Agenium's read tools (they run outside the sandbox) read it.
- **A closed ticket's kept CLI session** stayed until the 14-day sweep.

## Decision

1. **An agent set aside for its quota goes back to work once the quota should be back, with the same model**
   (`runtime/quota_back.rs`, before each scheduling round). When its vendor's last reading (Claude Code's, Codex's,
   OpenCode Go's subscription usage) shows the used-up window and when it resets, it goes back a minute after; otherwise
   it is tried again after 30 minutes, then after 1, 2 and 4 hours (4 hours at most between tries). A try that finds the
   quota still used up sets it aside again. Its open tickets get a System comment ("`<agent>`'s quota on `<model>`
   should be back …: it goes on from where it stopped, with the same model"), Needs you says it will go on by itself,
   and the user can still resume it at once or give it another model. Never another model (ADR-132 holds).
2. **A turn refused for its login runs again, once** (`run/relogin.rs`): the run waits for the other run's login sync
   (6 s), takes the user's current login into its own copy and starts the turn again. The CLI's session goes on when it
   has worked in it (told that its last turn stopped on a login error); one that did nothing yet starts the turn again.
   A second login error is a login that is really gone: the run fails and says to log in again.
3. **Agenium's read tools read a file only through its open handle**, as writes already did (`work/real_read.rs`, with
   the handle checks shared in `work/handle.rs`): the handle must be inside the run's roots (a folder swapped for a link
   after the path's check is refused) and the file must have no other name. `grep` leaves such files out. A file with
   other names (a package manager's hard-linked store, for one) can still be read with `shell`, where the sandbox
   decides what is reachable.
4. **A ticket's kept CLI sessions end with its worktree** (closed or cancelled): they held its work in progress.

## Consequences

- A subscription's window resetting at 3 am is used at 3 am: the team goes on without the user.
- An agent on a pay-as-you-go API whose credit is gone is tried again on the same schedule: a few refused requests a
  day, no cost, until credit is added or the user gives it another model.
- One CLI account can run several sessions at once without a run failing on a refresh another run made.
- Not done: macOS has no check of an open file's real path (writes and reads still go through the path checks). A
  vendor whose usage Agenium cannot read (Grok, agy) is tried on the schedule, not at its reset.

## Verified (2026-10-29)

Tests:
- `an_agent_out_of_quota_goes_back_to_work_when_the_quota_is_back` (not before 30 minutes, then an hour later; at the
  vendor's reset when it reported one, with no 30-minute try meanwhile; the same model; the ticket told);
- `a_turn_refused_for_its_login_runs_again` (the fake CLI's first turn refused for its login, the second reports);
- `a_hard_link_out_of_the_worktree_is_not_read_or_written` (read refused, grep leaves it out, a write refused also for
  a file that got another name after it was read);
- `a_closed_tickets_kept_cli_sessions_go_with_its_worktree`.

Real models: `agenium-eval --quota back` (DeepSeek through OpenCode Go): the Solo agent ran out of quota mid-ticket;
nobody changed anything; the round half an hour later set it back to work with the same model, and it finished, merged
and closed. Linux: the workspace's tests on the Ubuntu 25.10 VM (VMware). See [VERIFICATION](../VERIFICATION.md).
