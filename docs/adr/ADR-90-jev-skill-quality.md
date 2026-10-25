# ADR-90: The judge can read a skill's quality against the methodology principles

Status: Accepted (2026-10-25; owner asked how skills are checked against the methodology logic and whether Jev evaluates
them). Extends the skill system (ADR-88) and the Jev judge (ADR-75/81).

## Context

The skill check shipped in ADR-88 (`lint_skill`) is **deterministic and structural**: it checks that the five sections
are present and non-empty, that the body is not too long, and that it is not a near-duplicate of another skill (word
overlap). It does **not** read whether the content actually follows the methodology logic — whether it is truly one job,
whether the steps are actionable, whether the Proof demands real evidence, whether it embeds code that will go stale. No
model was involved.

## Decision

Add an **optional, fail-open** Jev read of a skill's *quality*, on top of the always-on deterministic shape check.

- `App::skill_quality(body)` asks Jev four calibrated Nouls — **one job**, **actionable steps**, **proof demands
  evidence**, **no stale embedded code** — and returns one note per principle with the verdict and confidence
  (`"proof demands evidence: weak (41%)"`, thresholds 0.7 / 0.3, between is "weak").
- Gated by `[judge].skill_quality` (off by default). `None` when the judge is off/disabled, the flag is off, or the call
  fails; a key/quota failure disables the judge (ADR-81). The deterministic shape check always runs regardless, so skill
  checking never depends on Jev being on.
- Surfaced in both places a skill changes: when the **user imports** one (`import_skill` returns `quality` beside
  `shape_notes`, shown by the desktop) and when an **agent self-upgrades** one (`propose_skill_update`): the async bridge
  reads the upgraded overlay body and adds `quality` to the tool result the agent sees. Fail-open throughout — without
  the judge (or on any failure) the agent gets only the deterministic shape check, so the loop never depends on Jev.

This is **how** Jev evaluates a skill: as a second, semantic opinion. Per the measured A/B (`docs/jev-comparison.md`),
Jev is not more accurate than a capable cheap model on judge tasks, so this is an opt-in convenience (a cheap, calibrated
offload), not a correctness gate — the structural `lint_skill` remains the default, always-on check.

## Consequences

- `foundation/config`: `JudgeSection.skill_quality` (off by default).
- `orchestration/orchestrator`: `App::skill_quality`, `App::agent_overlay_body`; the `ProposeSkillUpdate` branch in
  `runtime/bridge.rs` adds the quality read after the (sync) apply.
- Desktop: `import_skill` is async and returns `quality`; `Capabilities` shows the notes; vi/en `library.quality`.
- Test: `team.rs::skill_quality_is_off_by_default_and_reads_each_principle` (mock Jev).
- Not done (by design): a Jev gate that blocks a low-quality skill — it stays advisory, and the deterministic shape
  check is always the default so nothing depends on Jev.
