# ADR-82: The judge can pick the skill that fits a ticket (quick skill pick)

Status: Accepted (2026-10-25; owner: "implement quick skill pick, and what else can Jev do" — a fast tool pick that
helps the agent decide without reading everything, in two modes: with Jev and without). Amended by ADR-128 (on by
default once a judge key is given) and ADR-141 (the judge reads each description whole).

## Decision

Before a **fresh** work run, the TypeSafe Jev judge (ADR-75) may pick the one skill that best fits the ticket, so the
worker loads the right one instead of reading every skill's `SKILL.md`. It is a **hint**, not a gate — the worker
still decides what to load.

- `App::skill_hint(ticket, skills)` asks Jev a single `Choice` over the agent's skills (name → description), plus a
  `none` option, against the ticket's title and description. The pick becomes one line prepended to the first user
  message: *"Suggested skill (TypeSafe Jev, a hint — load it only if it fits): `name`. Call load_skill ..."*.
- **Fail-open, two modes** (same rule as ADR-81). Returns nothing — and the run proceeds on the agent's own judgment —
  when: `[judge].skill_hint` is off (the default); the judge is off or disabled; there are fewer than two skills to
  choose between (no question is sent); the pick is `none`; the pick is below the `MET` (0.7) confidence; or the call
  fails. A key/quota failure disables the judge and raises a Needs-you, exactly like `review_evidence`.
- Only on a fresh run (not a continuation, which already carries its context).

## Why

Skills are SKILL.md folders; an agent with several skills otherwise spends tokens reading each one to decide which to
load. One cheap, calibrated `Choice` (hundreds of tokens) narrows that to a single suggestion. It never blocks work:
if Jev is off or wrong, the agent simply reads and chooses as before.

## Consequences

- `foundation/config`: `JudgeSection.skill_hint: bool` (off by default).
- `orchestration/orchestrator/src/judge.rs`: `App::skill_hint`; `runtime/mod.rs` computes it before the work-order
  builder (async) and prepends the hint to the first user message.
- Desktop: the Settings judge note names both uses (evidence read + skill pick); enabling is still via `[judge]` in
  config.toml. No new IPC.
- Tests: `team.rs::skill_hint_is_off_by_default_and_needs_two_skills` and
  `team.rs::skill_hint_prepends_the_judges_pick` (mock Jev). The `Choice` wire shape is covered by
  `agent/llm/tests/systemone.rs`.

## Other Jev uses still open (ADR-81)

A guardrail `Noul` on output before submit, and lesson-signal/cue classification for memory. Each would follow the
same opt-in, fail-open rule.
