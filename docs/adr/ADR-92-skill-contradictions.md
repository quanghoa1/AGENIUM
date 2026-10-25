# ADR-92: Keeping skills from contradicting each other

Status: Accepted (2026-10-25; owner: "avoid skill contradictions"). Closes the known limit of ADR-91 and applies to
every way a skill changes: an agent's new skill (ADR-91), an agent's update of its overlay (ADR-73), a user's import.

## Context

Skills contradict each other mostly in two ways: two skills about the **same subject** drift apart, or two different
skills give **opposite instructions** about a shared practice (one says always disable a flaky test, another says
never). Detecting contradiction in general is semantic; the deterministic checks below are deliberately narrow and
precise, and the judge adds a semantic read when it is on — so the system holds with and without it.

## Decision

Four layers, cheapest and most precise first:

1. **One subject, one skill** (new skills; deterministic). A new skill whose subject (description + Job) is too close to
   an existing skill's must improve that skill with `propose_skill_update` — an explicit diff of one skill — instead of
   living beside it. Two signals, either one triggers:
   - by **content words** (hashing embedder over the subject with generic verbs and function words removed), always
     computed, threshold **0.40**;
   - by **meaning** (the local bge-small model, when installed), threshold **0.77**.

   Calibrated with `cargo run --release -p agenium-eval --example skill_similarity -- <models>`, which calls the gate's
   own scorer, on the 12 library skills against 6 paraphrased and 7 different candidates: different subjects scored at
   most **0.211** by words and **0.714** by meaning (no false positive at either threshold); the gate caught **2/6**
   paraphrases by words and **4/6** by meaning. A first version compared raw words and flagged a changelog skill as
   the same subject as doc-writing (0.45) because short subjects share "write" and "change"; removing generic words
   fixed it (0.09). Recall is modest by design — this layer must not refuse legitimate skills.
2. **No explicit opposite directive** (new skills, updates, imports; deterministic). Sentences with a positive marker
   (always, must, make sure, ensure) are compared with sentences carrying a negative one (never, do not, must not,
   avoid, no need to) in every other skill the agent uses, **references included**; a conflict is two shared content
   stems covering a third of the shorter clause. The 12 library skills with their references produce **no** conflict.
   - New skill: refused, naming both sentences.
   - Update: refused only for a conflict the change **introduces** — an inconsistency already in the user's skills does
     not block an unrelated edit.
   - User import: shown as advice with the shape notes; the user decides.
3. **The judge, when on** (`[judge].skill_quality`). Before anything is written, the new skill or the would-be updated
   skill is read against the 3 closest skills by subject, one Noul each ("do they give contradictory instructions for
   the same situation?"); a confident yes (≥ 0.7) refuses. Off, failing or out of quota: nothing is added and layers 1,
   2 and 4 decide.
4. **Experience** (ADR-91): a skill whose lesson misleads reopened work loses confidence and is flagged for the user.

Also hardened: an **untrusted lesson** (from a page, a file or a tool output) can no longer change a skill through an
update either — the rule ADR-91 set for new skills.

## Consequences

- `orchestration/orchestrator/src/skill_conflict.rs`: `subject`, `subject_words`, `subject_scores`, `same_subject`,
  `directive_conflicts`, `skill_guidance`, `closest`, `App::{known_skills, judged_contradictions,
  library_skill_conflicts}`; thresholds `SAME_SUBJECT_WORDS`, `SAME_SUBJECT_MEANING`.
- `judge.rs`: `App::skill_contradiction`. `skill_create.rs`: subject and directive gates. `skill_overlay.rs`:
  `preview_skill_update`, the introduced-conflict gate, the untrusted-lesson check. `runtime/bridge.rs`: the judge
  pre-check for both tools. Desktop `import_skill`: conflict advice.
- `quality/eval/examples/skill_similarity.rs`: the calibration.
- Tests: `skill_conflict::tests` (library consistent, opposite directives caught, unrelated ones not, a paraphrased
  subject caught); `team.rs::skills_cannot_contradict_each_other` (same subject, opposite directive, introduced vs
  pre-existing conflict in updates, untrusted lesson, import advice); `team.rs::the_judge_reads_contradictions_only_when_on`.
- Limits: paraphrased subjects below the thresholds and implicit contradictions without marker words pass the
  deterministic layers; they stay contained to the agent (ADR-91) and are caught by the judge when on or by the
  experience flag. A larger labelled calibration set would raise recall safely.
