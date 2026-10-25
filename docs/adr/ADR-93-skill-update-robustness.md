# ADR-93: Skill updates an agent can actually land — whole body, real errors, content fallback

Status: Accepted (2026-10-25; found by the learning-loop runs on six real models, `docs/learning-loop-eval.md`).

## Context

In the first pass, 3 of 6 `propose_skill_update` attempts failed to apply. Two problems:

1. The error came from the `-p0` fallback (`b/SKILL.md: No such file or directory`) and hid the reason of the `-p1`
   attempt the `a/`/`b/` diff was meant for — gpt-6.1-sol gave up.
2. The cause, visible in the patch Sol sent in pass 2: a hunk with **no context lines**, numbered from the skill body as
   the agent had read it (line 7; the real line is 14). git relocates hunks through their context, so it cannot place
   one without any.

Also found while building the scenario: the ADR-91 hint told agents to write skills "only after the ticket's work is
done", but `submit_report` ends a work run and review runs do not offer the tools — read literally, never.

## Decision

- **Whole body instead of a diff.** `propose_skill_update` takes either `patch` or `content` — the complete new body;
  the current frontmatter is kept (a frontmatter the agent pastes in is dropped, so name and description cannot change
  this way). Exactly one of the two. Every gate (lesson, injection, secrets, contradiction, validation) runs on the
  result either way.
- **The real error.** A diff with `a/`/`b/` paths reports its `-p1` failure, plus a hint: copy context lines exactly as
  `load_skill` shows them, or send `content`.
- **Content fallback.** When git cannot apply a diff, each hunk's old lines are looked up as whole lines; if every hunk's
  old text occurs **exactly once**, it is replaced. Ambiguous, partial-line or empty matches still refuse.
- **Line endings** are pinned to LF in the apply step, as in the rebase helpers (ADR-86).
- **Timing hint** (ADR-91): write a skill down "when the ticket's work is done and before you call submit_report (never
  instead of the work)".

## Consequences

- `foundation/core`: `ProposeSkillUpdate.content` (optional; `patch` now defaults to empty); the tool description names
  both ways.
- `skill_overlay.rs`: `updated_text`, `frontmatter`, `apply_by_content`; `apply_patch` reports the meaningful error;
  `apply_skill_update` and `preview_skill_update` share `updated_text`.
- Tests: `skill_overlay::tests::{an_update_can_send_the_whole_body_and_keeps_the_frontmatter,
  a_mismatched_diff_reports_the_real_reason, a_zero_context_hunk_with_a_wrong_line_number_applies_by_content}` — the last
  one uses Sol's exact patch.
- Evidence: pass 2 on the same six models — Sol saw the real reason and retried successfully; 12/12 runs passed.
- `quality/eval/src/learning.rs` keeps what a model sent in a refused skill call, so the next failure is diagnosable.
