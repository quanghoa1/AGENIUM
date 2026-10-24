# ADR-57: Team mode — the Lead's inbox, SLA and escalation

Status: Accepted (2026-10-24, fresh implementation, M4)

## Context

The architecture page ("Phản hồi Lead", "Scheduler & vận hành") describes an event-driven Lead that wakes when a
worker asks, requests a permission or reports, answers within an SLA, and escalates to the user when it cannot
decide. Until M4 the runtime only started work runs, review runs and user chat turns: in a team, a worker's question
or an approved plan waited for the user to chat with the Lead.

## Decision

1. **Inbox = derived view, not a table.** `App::lead_inbox` computes it from the database on demand (nothing to keep
   in sync): blocking questions, permission requests inside the project allowlist (outside ones are the user's),
   approved plan items without a ticket, tickets without a usable worker (New unassigned, or Reopened whose worker is
   Offline/QuotaExhausted), then non-blocking questions; oldest first inside a kind. Reports are not inbox items:
   Resolved tickets keep their own review runs. Solo agents have no inbox (their questions go to the user).
2. **Lead inbox run.** Each scheduler round starts with the SLA check and, when the Lead is Idle and the inbox is not
   empty, one Lead run on the inbox (before work runs, so answers and new assignments are used in the same round).
   The run gets the items, the tickets they concern with their comments, the project state and the team roster; the
   Lead's work tools are read-only; spec/plan creation stays in the chat with the user, review/close in review runs.
   The run ends by itself when the inbox is empty; text-only turns are nudged.
3. **Rounds, not interrupts.** The runtime is round-based (work runs of a round run in parallel, the Lead between
   rounds). A blocking question parks the worker's run at once, so nobody waits on a busy Lead longer than a round.
   The design's optional "interrupt" of a long Lead activity is not implemented.
4. **Escalation is a flag on `answer_question`.** The role table gives the team Lead no `ask_question`; instead
   `answer_question` takes `escalate: true` (Lead only): the question stays open for the user, the Lead's note is
   posted as a reply, a NeedsUser event is emitted, and the item leaves the inbox. The user answers with the same
   `answer` command as for Solo questions. This adds one optional field to the design's tool schema.
5. **SLA.** A blocking question of a worker open longer than `runtime.blocking_question_sla_minutes` (default 10)
   gets one System comment (reply to the question) and a NeedsUser event; the Lead can still answer it.
6. **No token burn on a stuck inbox.** If the Lead's inbox run does not finish twice in a row on the same inbox, the
   runtime stops starting it (until the inbox changes) and emits NeedsUser.

## Consequences

- A team takes an approved plan to closed, merged tickets without the user chatting: covered by
  `runtime.rs::team_lead_inbox_takes_a_plan_to_closed_tickets` (Lead + 2 workers, a dependency, a blocking question
  answered by the Lead) and `team.rs::lead_inbox_order_sla_and_escalation`.
- Lead load metrics (inbox length, response time, SLA misses) can be computed from the same view and the event log
  (dashboard, later).
