# ADR-149: Messages to a busy Lead wait in a queue the user can change

Status: Accepted (2026-10-30). Owner, after the end-to-end run in the app ("lúc lead đang bận thì sẽ vào queue tin nhắn
chờ, và người dùng cũng có thể edit tin nhắn chờ trước khi lead hết bận"). Replaces the delivery part of ADR-104.

## Context

ADR-104 put a message sent while the Lead (or Solo agent) runs into that run: the loop read it before its next model
request, whatever the run was. In the app this showed two faults:

- A message steered into a review or inbox run was acted on (the Lead cancelled the tickets the user named) but never
  appeared in the conversation: the chat shows the chat session only, so after a reload the user's message was gone
  and the Lead's answer, written in another run, was nowhere.
- Once sent, a message could not be changed: it was already in a run's context.

ADR-147 already stores every message to the Lead before it is acknowledged (`lead_message`).

## Decision

1. **The queue is the stored messages.** A message to the Lead is stored first (ADR-147). When a run of the Lead goes
   on — a chat turn, a review, an inbox or planning run, a Solo agent's work run — it waits there (`chat` answers
   `kind: "queued"`); no run is given a message in its middle any more.
2. **One chat turn reads the queue.** When no run of the Lead goes on, its next chat turn takes every queued message
   at once, oldest first, in one transaction (`App::take_lead_messages`); they then are the turn's user message and
   show in the chat with its answer. A turn that does not reach its end puts them back (a guard, also on a panic). The
   scheduler answers the queue before the inbox and the work of a round (the user waits on it); after two failed turns
   in a row the queue waits for the user's next message (as before). After a restart the queue is still there.
3. **The user can change or take back a queued message** until a turn takes it (`edit_lead_message`,
   `drop_lead_message`; refused once read). The chat shows the queue above the input, each message with **Change**
   and **Take back**; the input stays open while the Lead works.
4. Answers to a worker's question that go to the Lead (ADR-122) go through the same queue. Workers are still never
   messaged (ADR-104).

## Verified

- `runtime::team::the_user_messages_a_running_lead_and_never_a_worker` (two messages while the chat turn runs: queued,
  not in that turn; one changed; the next turn reads them as changed, in order, then the new message; a read message
  cannot be taken back), `runtime::messages::overlapping_lead_runs_keep_the_users_messages` (a chat turn and an inbox
  run overlap; the queue waits for both, then one turn), `runtime::messages::queued_messages_are_answered_kept_or_taken_back`
  (answered while Agreeing; taken back; a failing turn puts it back, tried twice, then the user's next message),
  `runtime::messages::a_message_to_the_lead_survives_a_stop_before_its_turn`.
- In the app: see VERIFICATION.md, runs of 2026-10-30.

## Consequences

- A correction sent while the Lead works reaches it when that run ends, not in its middle: a long Solo work run is not
  turned by it (the user can pause the project to stop it).
- The run's own message queue in the agent loop stays (a vendor CLI may still leave one there; it goes to the queue).

## Amendment (2026-10-31, code audit M9)

A turn's taken messages are marked taken, not delivered, and delivered when the turn ends: the in-memory guard puts
them back on a failed turn, but a process that died in the turn left them read and lost. At start, messages still
taken go back to the queue (`ledger::return_taken`, called from reconcile); test
`a_message_taken_by_a_turn_the_process_died_in_is_queued_again`.
