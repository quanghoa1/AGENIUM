# ADR-75: TypeSafe "Jev" as an optional judge (not an agent model)

Status: Accepted (2026-10-25). The owner asked to evaluate Jev / TypeSafe for Agenium and to reduce agent tokens
where sensible. Evaluated live with the owner's test key on 2026-10-25 (key test-only, never stored).

## What Jev is

TypeSafe's "System One" model **Jev** is not a chat model and not an image generator: it answers **typed questions**
about supplied state (text, application data, images) with **calibrated probabilities**. Question types: Noul
(probability a statement holds), Choice (one option among criteria, with per-option probabilities), Score (a level),
Box (locate something in an image; `jev-1.14.0` only). One request batches many questions.

Verified against `POST https://api.typesafe.ai/v1/systemone` (`Authorization: Bearer`, images need
`TypeSafe-Danger-Zone-Images: true`): a 320x320 red PNG -> Noul "solid red" = 0.98 (576 input tokens); a text report
with 3 questions (2 Noul + 1 Choice) answered in one call for 384 input tokens, output free (~$0.042 / 1M input).

So Jev **cannot** be an agent's provider (no chat, no tool calling), nor generate deliverable images or Live-office
art. It **can** be a cheap, calibrated second opinion.

## Decision

1. **Client, not provider.** `agenium_llm::systemone::{Judge, Judging, Question, Answer}` sends one batched request
   and returns answers by id. Off unless `[judge]` is configured (`base_url`, `model`, `api_key_ref` in the vault);
   Agenium never routes an agent's turn to it.
2. **One wired use, opt-in: review evidence read** (`[judge] review_assist = true`). Before a review run, Agenium
   asks Jev one batched Noul per acceptance criterion over the worker's report ("does `report` give evidence this is
   met?") and prepends a short calibrated "independent evidence read (verify it yourself)" to the Lead's review task.
   This is text-only (no danger-zone header), cheap, and gives the Lead's expensive model a first opinion instead of
   forming one from nothing. A missing key or a failed call is non-fatal: the review proceeds on the Lead's own read.
3. **Not wired (recorded for later):** screenshot checks in review (Box + Noul; needs the experimental images
   header and screenshot downscale to <=1568 px), role/skill routing (Choice), lesson-signal classification.
   Secrets can leak through screenshots (the redactor only covers text), so image use stays off until chosen.

## Consequences

- `agent/llm/src/systemone.rs` (+ `Http::post_json`), `agent/llm/tests/systemone.rs`;
  `foundation/config` `[judge]`; `agenium_orchestrator::judge` (`App::judge`, `review_evidence`), wired in the
  review run. All off by default.
- The skill `typesafe-ai` (github.com/typesafe-ai/skills) is a valid SKILL.md folder Agenium could import; it is not
  installed and its global-install hints are ignored.
