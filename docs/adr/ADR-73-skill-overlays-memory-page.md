# ADR-73: Skill overlays from validated lessons; the Memory page

Status: Accepted (2026-10-25, M4.5, decision 35; owner: "add a Memory tab to the UI").

## Decision

1. **`propose_skill_update`** is answered in the run (like `load_skill`): the agent names a role or library skill,
   a unified diff against its current SKILL.md, the reason, and the lesson it writes down (`lesson #<id>`, shown with
   every lesson in the work order). Checked: the lesson is live and held in at least two contexts (ADR-72); the
   patch applies (`git apply`); the result is a valid skill; nothing looks like a secret; no injected instruction is
   added (ignore-previous-instructions phrases, piping a download into a shell). Then it is written copy-on-write to
   `<root>/agents/<id>/skills/<name>/` (staged and swapped in), recorded in the agent's `skill_overlay` row (base
   version, cumulative diff from the base, changelog with reason, lesson and its contexts) and in the audit trail.
   No user approval and no notification per change (decision 35). The skills index marks it `[updated]`; other
   agents keep the base.
2. **The user** reverts an overlay (the agent uses the base again) or promotes it (the base becomes this version for
   every agent with the skill).
3. **Memory page** (nav "Memory"): per agent, lessons / facts / preferences with cue, scope, the contexts a lesson
   held in ("may update a skill" from two), confidence, decayed weight (faded ones on request), source, evidence and
   last use; search; retire an entry; "Tidy now" (decay and consolidation); the skill updates with their changelog and
   diff, Revert and Promote.

## Evidence

- `team.rs::a_validated_lesson_updates_the_agents_overlay_of_a_skill` (one context refused, injection refused,
  overlay used and marked, base untouched, diff and changelog, revert, promote).
- Real app on the test root: the Memory page lists researcher-2's 13 lessons and 8 facts with scope and weights.
