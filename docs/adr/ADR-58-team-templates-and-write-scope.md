# ADR-58: Team templates (TOML) and the write scope of workers

Status: Accepted (2026-10-24, fresh implementation, M4)

## Context

The architecture page ("Template team & Role", "Chuyên môn & phạm vi") specifies built-in team templates (Coding,
Pentest, Research, Content) as versioned YAML/JSON files, roles with tags, ROLE.md, skills, MCP, default model,
allowed paths and count, and an orchestrator that refuses a worker's action outside its scope unless a permission is
granted. Until M4 a team could only be created through the API with hand-written agent specs (the CLI had no team
command), and neither a role's allowed paths nor a ticket's `scope_paths` limited writes.

## Decision

1. **Format: TOML**, not YAML. The workspace already uses `toml` for `config.toml` and `agenium.toml`; TOML is just
   as diff-friendly, and adding a YAML crate (the common one is unmaintained) buys nothing. Built-in templates live in
   `orchestration/orchestrator/templates/*.toml` (compiled in); user templates in `<root>/templates/<id>.toml`, where
   the same id overrides a built-in. Validation: one Lead (count 1), at least one worker, slug ids, `1..=10` agents
   per role, `provider/model` defaults, allowed paths inside the worktree.
2. **Creating a team from a template** makes one agent per role slot, named `<prefix>-<role>[-n]` (agent names are
   unique in the root), with role `<template>-<role>`. The role folder `<root>/roles/<template>-<role>/` gets
   `ROLE.md` (the template's prompt) and `role.toml` (allowed paths) when missing; afterwards the user owns them.
   The project's default report template becomes the template's. Skills named by the template but not installed in
   the role folder are reported, not fetched.
3. **Write scope** = the ticket's `scope_paths` AND the role's allowed paths (an empty list does not restrict);
   approved path permissions of the ticket and `path:` entries of the project allowlist widen it. Enforced twice:
   - by the file tools of API runs (`ToolCtx::write_scope`, immediate structured error `out_of_scope`), and
   - at `submit_report` against every file changed on the ticket branch, committed or not — this covers shell
     commands and CLI backends, whose writes the file tools never see. The report is refused with the file list.
   Tool output under `artifacts/tool-output/` is exempt. Solo agents and tickets without scope are unrestricted.
4. Per-role MCP bindings, network allowlists and default models in templates are parsed (default model) or left to
   the existing per-agent settings (MCP: `agenium agent mcp`; network: project allowlist) for now.

## Consequences

- `agenium team templates|template|import|create|show` make team mode usable from the CLI.
- A worker that edits outside its scope learns it at the first write (API) or at submit (any backend) and can ask
  for the path through `request_permission`, which the Lead (inside the allowlist) or the user decides.
- Tests: `team_templates.rs` unit tests (built-ins valid and round-trip, validation), `paths.rs::write_scope`,
  `team.rs::team_from_template_and_write_scope`.
