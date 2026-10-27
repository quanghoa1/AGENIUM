# ADR-118: A lesson counts a context only when it was in play; promotions need contexts the record backs; claims carry intervals

Status: Accepted (2026-10-27; owner: study Infini-AI-Lab/hermit for self-improvement, then "tự quyết 3 việc đó … sao cho
hợp lý"). Tightens ADR-87 (shared pool), ADR-91 (agent-created skills) and decision 35 (validated contexts). Learned
from hermit (Apache-2.0); no code or package of it is used.

## Context

Hermit evolves an agent's harness (instructions, skills, memory) around a frozen model. Agenium already has most of
its machinery, behind stricter gates. Two of its rules exposed gaps in ours:

- *"A deployed rule silences its own counter-evidence: silence after deployment is not confirmation"*
  (`materials/skills/failure-discipline/SKILL.md` §4). Our pipeline counted a validated context for **every** lesson
  shown to a run that closed cleanly (`agent/memory/src/pipeline.rs`): a hint from another context, or a project rule
  put before every run whatever the ticket, earned a context by a quiet close. Two such contexts unlock creating a skill
  (ADR-91) and the shared pool (ADR-87).
- Every knowledge-base entry carries `derived_from`, links to the examples it came from. Our contexts are strings
  nothing checked.
- Hermit reports results as paired differences with intervals, and most of its gains sit inside them. Ours (memory on
  vs off, n = 5) were medians and ranges.

## Decision

1. **In play, or it does not count.** Each recorded use says whether the lesson was shown as a rule of the work's
   context (its scope matched exactly: `fact_usage.as_rule`, agent migration 5). On a clean close a lesson counts the
   context only if it was shown as a rule **and** found by the work's own words (a text or graph channel), not only put
   before the run as a project rule. Confidence still rises a little for every memory shown on a clean close; only the
   validated contexts — what unlocks skills and the pool — are stricter. Contexts counted before this change stay: the
   old rows do not say how a lesson was shown, and erasing earned standing on a guess would be worse than keeping it.
2. **Contexts the record backs.** Creating a skill from a lesson, changing a skill with one, and sharing it to the pool
   count only contexts `<project>/ticket:<key>` that name a ticket of that project that exists and **closed**. A chat
   context, a ticket deleted or cancelled since, or an unknown project does not count. The refusal says how many
   contexts are on record.
3. **Claims carry intervals.** `agenium-eval --compare <dir>` reports two conditions of interleaved runs as the mean
   paired difference with a 95 % bootstrap interval (10 000 resamples, seeded). A difference whose interval crosses zero
   is reported as such, not as a finding. The memory-reuse runs were extended to n = 10 and reported this way
   (`docs/memory-eval.md` §3).

Not adopted: hermit's ungated in-place edits (Continual Harness) and gates turned into advice (MWM) break the owner's
strict-gate rule; learners that write runnable tools conflict with ADR-91; a validation-split gate on every skill edit
needs a task set and many runs per edit, and hermit's own skill-optimisation gains were within noise.

## Consequences

- `agenium_store`: `record_shown`, `lessons_in_play`, migration `agent_005.sql`.
- `agenium_memory`: `Memory::record` marks rule-shown lessons; the pipeline validates only lessons in play.
- `agenium_orchestrator`: `App::evidenced_contexts` used by `create_agent_skill`, `apply_skill_update` and
  `share_lessons`.
- `agenium_eval::stats` (`paired_bootstrap`, `compare`), `--compare`.
- Tests: `graph_memory.rs` (a hint and an injected rule do not validate), `team.rs::an_agent_creates_a_skill_only_through_every_gate`
  (contexts not on record do not count), `stats.rs`.
- Expect skills and pool lessons to form more slowly; that is the intent.
