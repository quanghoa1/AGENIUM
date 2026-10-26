# ADR-104: The user can message a running Lead or Solo agent — never a worker

Status: Accepted (2026-10-26). Evidence: `agent/loop/tests/loop.rs::messages_sent_during_the_run_reach_the_next_request`,
`orchestration/orchestrator/tests/runtime.rs::the_user_messages_a_running_lead_and_never_a_worker`. Source: pi's steering
queue (`docs/reference/PI-COMPARISON.md`), shaped by the owner's rule.

## Context

A message to the Lead while it was working started a second, concurrent Lead run that knew nothing of the first.
pi lets the user steer a running agent: the message is read at its next turn. The owner's rule (2026-10-26): the
user talks only with the Lead and with a Solo agent; workers get their work only through tickets and the Lead's plan.

## Decision

1. While the project's Lead (a Solo agent is its own team's Lead) runs on the API backend, its run has a message
   queue. `Runtime::steer` puts a message there; the loop reads the queue before each model request and adds each
   message as `[Message from the user while you work] …`. `Runtime::chat` tries this first, so the app and the CLI
   need no change: they get a `Delivered` answer instead of a new turn.
2. **Workers have no queue.** Only the agent that is the project team's Lead is ever registered; a message never
   reaches a worker. A change for a worker goes through the Lead (ticket, plan).
3. **Nothing is lost.** The queue is removed under the same lock `steer` uses; a message that arrived after the last
   step is kept and opens the project's next chat turn (the next `chat` call or the scheduler's round).
4. A Lead running through a vendor CLI has no queue yet: the message starts a turn as before.
