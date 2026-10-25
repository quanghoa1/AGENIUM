# ADR-86: Skill overlays rebase onto an updated base (3-way merge)

Status: Accepted (2026-10-25; owner: "continue the remaining core items"). Completes the M5 item left open in
[ADR-73](ADR-73-skill-overlays-memory-page.md).

## Context

A skill overlay (ADR-73, decision 35) is an agent's copy-on-write copy of a skill's `SKILL.md`, built on the base as it
was when the lesson applied. The overlay record keeps the `base_version` and the cumulative unified diff `base ->
overlay`. When the **base** later changes — the user edits it, imports a new library version, or another agent's overlay
is promoted — the overlay is stale: it carries neither the base's improvements, and the Memory page's "diff against the
base" now mixes base changes with the agent's.

## Decision

Add a **3-way rebase** of stale overlays onto the current base.

- **Detect staleness without a schema change.** The base the overlay was built on is reconstructed by *reverse-applying*
  the stored cumulative patch to the overlay (`git apply --reverse`); if that reconstructed base differs from the
  current base, the overlay is stale. `OverlayView.stale` exposes this to the Memory page.
- **Rebase** = `git merge-file` (no repository needed): start from the new base and fold in the agent's `old_base ->
  overlay` change.
  - **Clean merge**: validate the result as a skill, swap the overlay in, and record a `rebased_onto` changelog entry
    with the new base version.
  - **Conflict** (overlapping edits, or the result fails skill validation): the overlay is **left unchanged**, the
    merged text with `<<<<<<<` markers is returned as a preview, and one Needs-you asks the user to resolve it (revert
    or re-propose). The agent keeps working on its current overlay meanwhile.
- Line endings are normalised to LF and `core.autocrlf`/`core.eol` are pinned off in these git calls, so Windows
  checkout rules do not turn every overlay stale or corrupt a merge.

`App::rebase_skill_overlays(agent)` rebases all of an agent's overlays and returns a `RebaseOutcome` per skill
(`up-to-date` / `rebased` / `conflict` / `no-base`). Desktop: the Memory page shows a "base changed" chip and a "Rebase
onto base" button on a stale overlay (`memory_action` action `rebase`).

## Consequences

- `orchestration/orchestrator/src/skill_overlay.rs`: `unapply_patch`, `merge_three`, `rebase_skill_overlays`,
  `OverlayView.stale`, `RebaseOutcome`.
- Desktop: `memory_action` `rebase`; Memory view chip + button; `OverlayView.stale` in TS types; vi/en strings.
- Tests: `skill_overlay::tests::{unapply_reconstructs_the_base_the_overlay_was_built_on,
  a_non_conflicting_base_change_rebases_cleanly, an_overlapping_base_change_reports_a_conflict}`.
- No store migration: the existing cumulative patch is enough to reconstruct the old base.
