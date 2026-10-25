# ADR-71: Team template wizard

Status: Accepted (2026-10-25, M4: "template wizard in Settings: roles, skills, model, report template; save,
import/export").

## Decision

1. **Templates page = wizard.** Built-in templates and the user's own are listed with their origin (built-in, yours,
   your copy of a built-in). A template is created from scratch (lead + worker with the ROLE.md section skeleton),
   drafted from a team as it is (`template_from_team`: one role per members' role, counted, the Lead's role as lead,
   ROLE.md text, tags, write scope, planner flag and each role's model as its default; template prefixes are
   dropped from the role ids), or imported from TOML; any template is edited (a built-in is saved as the user's
   copy with the same id) and exported as TOML.
2. **Editor.** Identity (id, name, version, report template, description) and the roles: id, kind (lead / worker /
   planner), count, tags, default model, write scope, ROLE.md text, optionally started from a library or existing
   role. Saving validates on the backend (one lead with count 1, at least one worker, at most one planner, slug ids,
   1-10 agents per role, scopes inside the worktree, `provider/model` defaults).
3. **Storage.** `<root>/templates/<id>.toml`; removing a user copy brings the built-in back; built-ins cannot be
   removed. Teams made from a template keep their role folders when the template changes.
4. **Not built (yet):** per-template allowlist and pre-merge checks (projects own their allowlist; pre-merge checks
   are M5), MCP bindings (agents own them, ADR-61).

## Evidence

- `team.rs::a_team_becomes_a_template_that_makes_new_teams`; real app: the Research team drafted as `research-plus`
  (3 roles, models, ROLE.md), saved and exported as TOML.
