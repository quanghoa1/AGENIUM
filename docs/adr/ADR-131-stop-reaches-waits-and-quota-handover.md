# ADR-131: The user's stop reaches what a run waits on; a worker set aside mid-ticket hands its work on

Status: Accepted (2026-10-29). Owner: "việc agenium điều khiển cli như thế nào, handle chuẩn chưa"; "cần test kịch bản
hết quota giữa chừng của các agent/vendor thì sẽ thế nào, việc thay thế, resume lại có dễ không, có follow tiếp công
việc ổn không". Amends decision 3j / 15 (QuotaExhausted belongs to the agent) and ADR-63 (the team works to the end).

## Context

- **A stop waited for the step.** A run looked at its cancel flag between steps; a command it ran (`shell`, 120 s by
  default, 600 s at most) or an approval it waited for (30 minutes in the app) went on to their end. The desktop's
  Stop all held for up to five minutes waiting for such runs, and an approval request given up stayed in Needs you,
  where answering it did nothing.
- **A worker out of quota mid-ticket left the work stuck.** The ticket said "change the model or add credit to
  continue", but changing the model left the agent set aside until it was also set back to Idle by hand. In a team, the
  Lead was told only of a *Reopened* ticket whose worker could not work: one the worker had begun (InProgress) or was
  waiting for (Assigned) stayed with it, and could not be assigned to anyone (only New or Reopened tickets could).

## Decision

1. **A run's stop reaches what it waits on.** The tool context carries the run's stop flag (`ToolCtx::stop`), set by
   the run's cancel (`Cancel::forward`, both the API loop and a CLI run): a running command is ended at once, its whole
   process tree with it (host, MXC and bubblewrap runners already watched a stop flag for background commands), and a
   command waiting for the user's approval is given up (`stopped`). An approval request whose run no longer waits leaves
   Needs you (answered, unanswered for 30 minutes, or given up).
2. **A new model frees an agent set aside for its old one.** Changing an agent's model or backend sets an agent that
   was QuotaExhausted or Offline back to Idle: the new model has its own quota and login. Its tickets go on, the same
   agent resuming from its conversation (a continuation run keeps the messages across providers and between the API
   and a vendor CLI, which gets them in its first prompt).
3. **A team hands a set-aside worker's tickets on.** The Lead's inbox names every ticket (Reopened, Assigned or
   InProgress) whose worker cannot work, with the worker and why. The Lead (or the user) assigns it to another member:
   it keeps its status and worktree, a System comment says who handed it to whom, and the taker's first message says
   who worked on it before and that their changes are in its worktree (look at them, go on from them, do not start
   over). Not while a run of the ticket is still going.
4. A used-up subscription of a vendor CLI is recognised by its own words too ("5-hour limit reached", "You've hit your
   limit"; a rate limit is not one). A lost CLI login is a failure that says to log in again (it said nothing); not a
   reason to set the agent aside, because a run that started from a login another run refreshed since fails the same
   way and the next run passes; a login that is really gone stops the runs after two and asks the user.

## Consequences

- Stop all lifts as soon as the runs going at the click have ended, which is now seconds, not a command's timeout.
- Quota mid-ticket costs no work: the worktree and the conversation stay; the user changes the model (one step) or the
  Lead hands the ticket on.
- Not done: a CLI subscription window that resets does not bring the agent back by itself (the user sets it to Idle or
  changes its model); a worker's conversation does not go to the member taking its ticket (its changes and the
  ticket's comments do).

## Verified (2026-10-29)

Tests:
- `a_stopped_run_ends_its_command_and_its_wait_for_approval` (tools);
- `the_users_stop_ends_a_long_command_and_a_wait_for_approval` (a CLI run, both through Agenium's shell);
- `the_users_stop_ends_a_running_command_at_once` (an API run: a 120-second command, stopped after 1.5 s; the run ends
  cancelled in about 3 s with no further model call);
- `quota_used_up_mid_work_goes_on_with_another_model`, `the_lead_hands_a_begun_ticket_of_a_set_aside_worker_to_another`.

Real models: [VERIFICATION](../VERIFICATION.md) (runs of 2026-10-29, `agenium-eval --quota same|team`).
