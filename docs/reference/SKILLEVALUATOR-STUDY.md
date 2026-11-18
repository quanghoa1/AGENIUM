# NVIDIA SkillEvaluator, read against Agenium's skill checks (2026-11-18)

What [NVIDIA/SkillEvaluator](https://github.com/NVIDIA/SkillEvaluator) does, what Agenium already does as well or
better, and what was taken. Learned from, not reused: no code or package of it is in Agenium.

## What it does

An experimental Python CLI (Apache-2.0) that evaluates skills people wrote (`SKILL.md` plus `scripts/`,
`references/`, `assets/`), in three tiers:

1. **Static checks:** the format, a 0–100 quality score from fixed deductions over four weighted areas, five skill
   shapes with checks of their own, security scanners (secrets, Semgrep, Unicode smuggling, PII, licences), how each
   agent CLI loads a skill, and an optional LLM rubric.
2. **Overlap:** repeated content inside a skill (embedding clusters, then a model says duplicate, detail, or related),
   and closeness between skills in bands of similarity. Missing evidence is "incomplete", never "clean".
3. **Live A/B:** the same cases in Docker with the skill and without it; signals from the trajectory and from an LLM
   judge; pass@k with Wilson intervals, an exact McNemar test, a paired bootstrap, and results marked provisional below
   a dataset size. Activation is tested in four kinds of case (explicit, implicit, negative, edge). Its own grader is
   checked against labelled trajectories (its composite score passed every one of 15 known failures).

It states its limits: no agreement study with people, weights by convention, no control for "any extra
instruction".

## Agenium already

- Learns skills under gates (a lesson held in two closed tickets, caps, the user's approval, the anti-drift flag):
  SkillEvaluator has no learning.
- Finds opposite directives between skills, and blocks only the ones a change brings; SkillEvaluator finds overlap only.
- Calibrated its similarity bars on labelled paraphrases with a local model (ADR-92).
- Measures a skill on real tickets at no cost, instead of paid synthetic runs (ADR-152).
- Lists criteria with notes instead of one composite score, and never scores "not measured" as a failure.

## Taken (ADR-152 amendment of 2026-11-18)

1. **The effect is judged with intervals.** One reopened ticket of five against none of five flagged a skill. Now the
   95% interval of the difference (Newcombe's, from Wilson intervals) must lie above zero, a wide interval is said to be
   provisional, and the comparison is said to be uncontrolled.
2. **Labelled cases** for the scorecard's text criteria (20 skills) and the contradiction check (6 pairs), in the
   repository, so CI measures them. They found a fault at once: every numbered list passed as "concrete steps",
   because the "1." of the list counted as a number.
3. **Hidden characters** (bidi controls, zero-width characters, tag characters) are refused or warned on wherever a
   skill's text is checked, and the injection phrases no longer flag a skill that merely names "the system prompt".
4. **A skill switched on and never loaded:** after its agents closed 10 tickets without loading it, the scorecard
   says so (its description may not say when it applies).

## Not taken

- Paid A/B runs of a skill in the product (ADR-152 chose real work), synthetic datasets, skill-interaction runs.
- A composite score with grades: it hides failures, as SkillEvaluator's own baseline shows.
- A required LLM rubric or classification: the judge stays optional (ADR-90).
- External security scanners, and a check of how each CLI loads a skill (Agenium gives skills through `load_skill`
  to every backend).
- A developer-only routing eval (explicit, implicit, negative and edge cases) is possible later: it costs model calls.
