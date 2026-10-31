# ADR-87: Cross-agent knowledge sharing through a controlled pool (GM-6)

Status: Accepted (2026-10-25; owner chose "a controlled shared pool" over a project pack, to keep agents
project-independent: "knowledge distilled in project A should be reusable in B without overfitting or
over-engineering"). Implements GM-6 (ADR-51 §10).

## Context

Memory is per-agent and travels with the agent (agents are a pool, ADR-61), so a single agent already carries what it
learned from project A into project B. Portability is governed by a lesson's **scope**: recall hard-filters on scope, so
a lesson scoped `{project: A}` is dropped in project B (`ScopeMatch::Conflict`) while a lesson scoped
`{language: rust}` or `{}` surfaces anywhere. What was missing is sharing **between agents**: agent X's lesson could not
help agent Y.

A "project pack" was rejected: it ties knowledge to a project, the opposite of the goal. A "ticket note" is too local.

## Decision

Add a **controlled shared pool** of generalized lessons that every agent also recalls from, with two existing mechanisms
as the anti-overfit guard — no new concepts:

1. **Promotion gate.** After the memory pipeline runs, a lesson is copied into the pool only when it is a live, trusted
   `Lesson` that **held in ≥2 distinct contexts** (`validated.len() >= MIN_CONTEXTS`, the same bar as a skill overlay)
   **and is not scoped to one project**. The pool copy drops the project, keeps scope and cue, caps confidence at 0.6,
   and is deduped by text so repeated runs do not pile up copies. Project-specific or one-off lessons never leave the
   agent.
2. **Pooled recall.** Each work run recalls from the agent's own memory **and** the pool; pool hits are scaled below the
   agent's own (×0.6), deduped against what the agent already holds, flagged `shared`, and shown as *"Shared lesson
   (from another agent, verify first)"* — never a hard rule. Scope filtering still applies in the pool, so a lesson that
   somehow carried a project scope could not leak across projects anyway.

The pool is an ordinary agent memory database under a reserved **nil** agent id (`App::pool_memory`); real agents get
random ids, so it never collides. Shared hits are **not** logged as the recalling agent's usage (their fact ids belong
to the pool). All of this is gated by `[memory] share` (default on); off stops both promotion and pooled recall, and
Agenium is unchanged.

## Consequences

- `foundation/config`: `MemorySection.share` (default true).
- `agent/memory`: `Recalled.shared`; `MemoryRetriever::with_pool` merges pooled recall below own; `record` skips shared
  hits; `label` marks a shared lesson as verify-first.
- `orchestration/orchestrator`: `App::pool_memory`; `Runtime::share_lessons` (promotion gate) called after a successful
  learn; the work-run retriever is given the pool.
- Tests: `agent/memory` `pooled_recall_merges_shared_lessons_below_own` (merge, dedupe, shared flag, usage skip). The
  promotion predicate (lesson + ≥2 contexts + non-project) is exercised by the pipeline path.
- Not done: a desktop view of the shared pool (the per-agent Memory page is unchanged); promotion runs on the agent's
  model-independent metadata, so it costs no model tokens.

## Amendment (2026-10-31, code audit M3; owner's decision the same day)

The pool follows the lessons it copied: a copy names its lesson (`shared:<agent>:<fact id>`), and one whose lesson
was retired, refined into another or lost confidence is retired from the pool (`pool_changes`).

Other agents' work now weighs on a copy, only downwards (owner: "hướng 2"):

- The pool hits a run is shown are logged in the **pool's** own usage (`MemoryRetriever::record`), still not in the
  recalling agent's: its memory and its counts are unchanged.
- A reopened run's learning sees them among the memories shown, under ids of their own past the agent's (a pool id
  may be the same number as one of the agent's facts: the blame never lands on the wrong one). The ones it names as
  misleading are reported (`LearnReport::misleading_shared`), never applied to the agent's memory.
- The pool's copy loses confidence (`HURT_DELTA`) and counts the context (`<project>/ticket:<key>`,
  `AgentStore::misled_in`). Misleading in two contexts (`MIN_CONTEXTS`, the evidence rule every lesson gate uses), it
  is retired from the pool and its text marked (`pool-dropped:<text>`): no agent shares that text again.
- The lesson it was copied from stays as it is in its agent's memory; nothing another agent does raises a copy.
- The run page shows the shared lessons that misled the work and those that left the pool.

Code: `runtime/pool.rs` (sharing, weighing, `pool_changes`), `agent/memory` (pipeline, retriever),
`foundation/store` (`misled_in`). Tests: `a_shared_lesson_is_blamed_apart_from_the_agent_s_own`,
`pooled_recall_merges_shared_lessons_below_own`, `the_pool_follows_the_lessons_it_was_copied_from`,
`a_pool_lesson_that_misleads_in_two_contexts_leaves_the_pool` (each fails when its rule is broken: checked by
mutation). Real models: `agenium-eval --shared-pool attribution|loop` (docs/VERIFICATION.md). They led to three
changes in the extraction that serve the agent's own lessons as much: the instruction about `misleading` stands next to
the memories shown (21/40 → 40/40 on the same cases), an empty or non-JSON answer is asked for once more, and a step
failing after the reuse signals no longer loses them (`LearnReport::failed`); an id written as text is read.
