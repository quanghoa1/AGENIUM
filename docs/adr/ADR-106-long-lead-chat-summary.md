# ADR-106: A long chat with the Lead is seen as a summary plus its recent messages

Status: Accepted (2026-10-26). Evidence: `orchestration/orchestrator/tests/runtime.rs::a_long_chat_is_summarised_once_and_kept`.
Source: pi's compaction entries (`docs/reference/PI-COMPARISON.md`).

## Context

Each chat turn with the Lead (or a Solo agent) sent the project's whole conversation. A long-running project's chat
outgrows the context, and every turn pays for all of it.

## Decision

1. The stored conversation stays whole (append-only), as memory learning and the app read it.
2. When a turn's history is estimated above 50 000 tokens (four characters a token), the older part is summarised
   once with the Lead's API model (a CLI Lead: the `[cli]` fallback model) under fixed headings: Goal, Agreements and
   decisions, Preferences the user stated, Open questions, State of the work. The last ~15 000 tokens stay as they
   are; the cut is always at a message the user wrote, never between a tool call and its result.
3. The turn sees `[Earlier conversation, summarised]` and the summary, then the recent messages. The summary and where
   it ends are kept in the Lead's database; the next summary updates this one instead of starting over.
4. A summary that fails, is empty or was cut by the output limit is not used: the turn sends the history whole and the
   next turn tries again.
