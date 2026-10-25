# ADR-65: Agent names are role names, unique; the Activity page folds into the audit trail

Status: Accepted (2026-10-25, owner: "the Lead sees every ticket, a worker only its own; in the office show only the
agent's name, not the team: there is a team filter"; "Activity and Audit trail show the same information, drop
Activity"; "the user may not give two agents the same name")

## Decision

1. **Who sees which tickets (confirmed, unchanged).** The Lead's runs list every ticket of the project and carry the
   full text of the tickets its inbox is about; a worker's run carries only its own ticket, that ticket's comments
   and the requirements its plan item traces to, and the engine refuses a worker's call on another ticket.
2. **Agent names without the team.** A team made from a template names its agents after their roles (`lead`,
   `planner`, `researcher-1`), no longer `<project>-<role>`: the office shows one team at a time. An explicit prefix
   (CLI `--prefix`) still works.
3. **Names are unique, ignoring case.** Agents live in one pool across teams, so creating an agent (pool, Solo, team,
   import) with a name another agent has, or twice in one team, is refused with "an agent named ... already exists;
   choose another name". A template takes the next free number instead (`lead-2`). An agent can be renamed from
   its page (same rule; id, memory and history stay), so older `<project>-<role>` names can be brought in line.
4. **No Activity page.** It listed the raw event log of every project; the audit trail shows the same events for the
   current project with agent names, ticket links, refused calls, a filter and a Markdown export.

## Consequences

- `App::check_names_free`, `App::rename_agent` (IPC `rename_agent`, pencil next to the name); `create_team_from_template` numbers taken names; desktop and CLI default to no prefix.
- UI: the Activity nav item, route and strings are gone (`api.events` stays for the Live tab).
- Test: `team.rs::agent_names_are_role_names_and_unique`.
