# ADR-141: A skill's description says what it does and when to use it

Status: Accepted (2026-10-29). Owner: "làm đi, cùng các việc bạn đề xuất sau khi đọc tài liệu của claude". From
Anthropic's skill authoring guide (`agents-and-tools/agent-skills/best-practices`). Amends ADR-88 (skill shape) and
ADR-82 (skill hint).

## Context

- An agent sees a skill's name and description only (tier 1, `agenium_skills`); it loads the body after it picked the
  skill. The guide's first rule for the description: what the skill does **and when to use it**, in the third person,
  with the words a request would use — the model picks among many skills from this text alone.
- ADR-88 put "When to use" in the body, read only after the pick. 13 of the 19 built-in skills' descriptions said what,
  not when ("Write a clear commit and PR …"). Nothing checked a description. The judge's skill hint (ADR-82) read the
  first 200 characters of each, which cut the "Use when" clause that comes last.

## Decision

1. `skill_shape::lint_description` advises (never refuses): no "Use when …" clause (or another "when"), a first- or
   second-person description ("I can …", "You …"), tags (`<…>`), fewer than 40 characters, a vendor's name in the
   skill's name. It runs with the body's checks wherever a user's skill is checked — import, save, the editor's
   Evaluate, the library's Evaluate (`App::lint_skill_md`, `App::library_skill_notes`).
2. The 13 built-in descriptions are rewritten: third person, what, then "Use when …" (`templates/skills/*/SKILL.md`);
   the six security skills already said when. A root that seeded them before keeps its own copies (seeding never
   overwrites, ADR-88); Evaluate shows the advice there.
3. The skill hint reads each description whole (up to the 1,024-character limit).

## Verified

- Tests: `a_description_says_what_and_when_in_the_third_person`; `a_built_in_skill_passes_its_own_checks` now checks
  every built-in description too.
