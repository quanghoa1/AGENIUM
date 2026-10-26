# ADR-98: The Lead learns from the user's chat — once it goes quiet, facts and preferences only

Status: Accepted (2026-10-26). Evidence: `orchestration/orchestrator/tests/runtime.rs`
(`the_chat_with_the_lead_is_learned_once_it_goes_quiet`, `a_failed_chat_extraction_waits_before_the_next_try`) and
`runtime::learn::tests`. Amends decision 2 (when the memory pipeline runs) and ADR-85 (the judge's gate applies here too).

## Context

The memory pipeline ran only when a ticket became Closed or Reopened. What the user tells the Lead in the chat — "answer
me in Vietnamese", "we deploy with `make ship`", "use pnpm" — was kept in the chat history of that project and nowhere
else: the next project, or the same project after the history is cut, started without it. A chat has no end event
like a closed ticket, and extracting after every turn would cost a model call per message.

## Decision

1. **When.** The scheduler's round (`Runtime::tick`) learns from a project's chat once it has been quiet for
   `[memory] chat_idle_minutes` (default 20; 0 = never). The chat reply is never delayed by it. `learn_from_chat(project,
   force: true)` learns now (for tools and tests).
2. **What.** Only the part of the chat not learned from yet: a watermark in the Lead's database (message count, and
   the time of the last turn learned), so each part is sent once and a round with nothing new reads two values and
   never loads the conversation. The project state Agenium appends to each user message is left out of the
   transcript.
3. **Who and how.** The Lead, with the same pipeline as tickets (extract → ADD/UPDATE/DELETE, entities, scope), its own
   model or, for a CLI Lead, the `[cli]` API fallback model. The episode has no signals, so only facts and preferences
   are kept, never lessons (decision 35; agent learning stays strictly gated). The user's own messages are never
   stored as untrusted: the extraction rules now say so, as they say for Agenium's own word (ADR-96).
4. **Cost and failure.** The judge's "nothing worth remembering" (ADR-85) skips the extraction and marks the part as
   learned. A failed extraction leaves the part unlearned and is not tried again before another quiet period, so a
   provider outage costs one call per period, not one per round.

## Consequences

- Preferences and project facts stated in the chat reach every later work order of the Lead (and, through recall,
  its tickets), without the user repeating them.
- One extra model call per quiet chat; none while the user is still talking.
- Not run against a real model yet (scripted model only); the extraction prompt is the one measured in
  `docs/memory-eval.md`, with one sentence added about the user's messages.
