# ADR-61: Agent pool, custom teams, skill library, providers entered by the user

Status: Accepted (2026-10-24, owner's request after reviewing the desktop app: "create agents on their own, pick
each one's model, give them a role, skills and MCP; add them to a team; the system only suggests the Lead"; "a list
of the system's skills and MCP servers, add/remove skills; each agent switches them on or off"; "add providers by
API key or local model; CLIs are detected")

## Context

The architecture page already says it ("Agent registry": *agents are long-lived entities the user creates first*;
"Tạo team": *pick existing agents into the Lead slot and the Worker slots*). The implementation took a shortcut:
an agent row needed a team (`agent.team NOT NULL`), so agents only came out of "team from template" or "Solo for a
project", and the template fixed the roster. Skills were per role or per agent folder, with no system list, and the
agent's skill binding *restricted* those folders. Providers could only be added from the CLI.

## Decision

1. **Pool.** `core_003` rebuilds the `agent` table with `team` nullable: an agent in no team is in the pool. Decision
   3 still holds as "at most one team at a time". The migration runner now turns foreign keys off around migrations
   (SQLite's table-rebuild procedure) and refuses a migration that leaves `PRAGMA foreign_key_check` rows; the
   previous state of `foreign_keys` is restored. `AgentCreated.team` is optional (old events still parse); new events
   `AgentTeamChanged`, `TeamProjectChanged`, `TeamDissolved`.
2. **Roles.** A role is a folder `<root>/roles/<id>/` (ROLE.md, role.toml with `allowed_paths`, `planner`, and now
   `lead` — a hint, never a rule — and default `tags`). Template roles are *presets*: listed before they exist and
   written when the first agent takes one. The user writes their own roles (`put_role`); a role in use is not deleted.
3. **Agents** are created into the pool (`create_agent`): name, role, tags (default: the role's), their own model
   (API provider or a CLI program + model), the library skills and MCP servers they enable. Role, model, skills and
   MCP change later; switching one skill/server is one call (`toggle_agent_skill`, `toggle_agent_mcp`).
4. **Teams** are composed from pool agents (`compose_team`): a name, the Lead, workers, optionally a project (one team
   per project, decision 25) or later (`set_team_project`, refused while the old project has open tickets). Rules:
   at least one worker (or a Solo: one agent, no members), at most one planner, a planner never leads, no agent twice,
   no agent from another team. Membership changes: `add_team_member`; `remove_team_member` refuses the Lead, an agent
   holding open tickets or working, and the last worker; `set_lead_of_team` swaps the Lead (the old Lead becomes a
   worker); `dissolve_team` sends everyone back to the pool (refused while the project has open tickets).
   "Team from template" stays as a shortcut that creates agents and the team in one step.
5. **Lead suggestion** (`suggest_lead`): role.toml `lead` (+4), a role named `lead`/`*-lead` (+3), tags
   `lead`/`planning`/`review` (+1 each), an enabled skill whose name or description mentions leading a team (+2);
   planners are skipped; nothing scores = no suggestion. The user picks the Lead; the suggestion is shown with its
   reasons.
6. **Skill library.** `<root>/skills/` holds the system's skills (`Owner::Library`, `Origin::Library`). The agent's
   `skills` field now lists the library skills it **enables** (opt-in); its role's skills and its own skills are
   always on (an own skill with the name of a role or library skill is its overlay; a role skill wins over a library
   skill of the same name). This replaces the earlier meaning ("binding restricts role/own skills"), which no UI
   used. Removing a library skill still enabled by agents needs `force` (they lose it); same for MCP servers.
7. **Providers** entered by the user (`put_provider`, `remove_provider`): the key goes to the vault as
   `provider/<id>` (never config.toml), editing without a key keeps the stored one, Anthropic/Gemini need a key,
   OpenAI-compatible endpoints need a base URL; removal is refused while an API agent or the CLI fallback uses the
   provider. `ProviderSource::forget` drops a cached client after a change. Local servers are discovered on the
   well-known ports (`agenium_llm::local::discover`: Ollama 11434, LM Studio 1234, `GET <base>/models`). CLIs keep
   being detected (`detect_all`).
8. **Setup and `AGENIUM_HOME`.** `agenium setup` now sets up the root `AGENIUM_HOME` points to, and neither the CLI
   nor the desktop app remembers an `AGENIUM_HOME` root in the machine-wide bootstrap file (only a root chosen with
   `--root` in the wizard is remembered). Found while testing: `setup` ignored `AGENIUM_HOME` and created
   `~/.agenium`, and an earlier `--root` test had left a bootstrap file pointing at a temporary folder (both removed).

## Consequences

- CLI: `agent create`, `role list|put|remove`, `team list|compose|add|remove|assign|dissolve` (compose without
  `--lead` uses the suggestion and prints why), `skill library`, `skill import|remove --library [--force]`,
  `provider remove|local`. Tests: `orchestrator/tests/registry.rs` (5), the v1 → v3 migration test, the rebuild
  migration test, `local_servers_are_discovered`.
- `xtask schemas` publishes every SQL migration of the store (it listed them by name).
- Open: the desktop forms for all of this (next), propose_role (Lead proposes a role, the user approves).
