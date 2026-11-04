# ADR-157: Plugins, whole from the vendor, in Claude Code's format

- Status: accepted (2026-11-04)
- Related: ADR-61 (library and per-agent switches), ADR-88 (skills), ADR-146/147 (an agent's skills and their gates),
  ADR-129 (vendor CLIs work through Agenium's tools)

## Context

Vendors publish sets of skills and MCP servers as plugins (`obra/superpowers`, `dietrichgebert/ponytail`), most in
Claude Code's layout: `.claude-plugin/plugin.json`, `skills/<name>/SKILL.md`, `.mcp.json`, often with hooks, slash
commands and subagents for one CLI. The owner's rules (2026-11-04): a plugin's skills are never updated or learned
into by an agent; there is no switch per skill or per server of a plugin, only a Plugin tab that switches the whole
set for an agent; an update takes everything from the vendor; and the format is Claude Code's, not one of Agenium's.

## Decision

1. **Source**: `owner/repo[@ref]` (GitHub) or a folder. GitHub is read as its archive
   (`codeload.github.com/<owner>/<repo>/zip/<ref>`, no API call, no rate limit); the commit is the archive's comment
   (`git archive` writes it). A repository with a marketplace that lists several plugins takes the one named.
2. **What is taken**: skills (`skills/` and the folders the manifest adds, each passing Agenium's skill checks; a
   plugin's skill may refer to a sibling skill's files inside the plugin's `skills/`, as Claude Code allows) and MCP
   servers (`.mcp.json` or the manifest's `mcpServers`, ids `<plugin>-<server>`). Not taken, and listed in the preview:
   hooks, commands, agents, `bin/`, servers that run code from the plugin's own folder (`${CLAUDE_PLUGIN_ROOT}`).
   Agenium runs no vendor code of a plugin beyond MCP servers the user saw.
3. **Preview, then install at the commit shown**: nothing is written by a preview; the install downloads that exact
   commit and refuses another. The new folder is built beside the old one and swapped.
4. **Stored in Claude Code's format, written by Agenium** (the owner: keep `agenium-plugin.json`, in Claude's format,
   not a copy of the vendor's files): `<root>/plugins/<name>/agenium-plugin.json` is a `plugin.json` of what was taken
   (`name`, the vendor's `version`, `description`, `author`, `homepage`, `repository`, `license`, `keywords`;
   `skills: "./skills/"`; the `mcpServers` taken, under their own names) beside `skills/<name>/`. Hooks, commands and
   the vendor's other files are not copied. Where each came from is in
   `<root>/plugins/installed_plugins.json`, shaped like Claude Code's (`version: 2`, `name@source` -> `installPath`,
   `version`, `installedAt`, `lastUpdated`, `gitCommitSha`, plus `source`, `ref`, `entry` and what was skipped).
5. **Whole per agent**: the agent's skill binding holds `plugin:<name>`; its skills load with origin `plugin` (never
   over a role or library skill of the same name, never replaced by an agent's own skill), its servers are bound with
   every tool at each run (API and CLI brains alike, through Agenium's MCP). The agent page's Plugin tab is the only
   switch.
6. **Read-only**: a plugin's skill is no role, library or created skill of the agent, so `propose_skill_update`,
   overlays and skill proposals refuse it; a new skill of its name is refused as existing.
7. **Update**: the ref it follows is fetched again; the preview shows the skills and servers added, changed (by a
   digest of their files) and removed, and the agents that have it on; applying replaces the whole set. **Remove**
   deletes the folder and the record and switches it off for every agent.

## Consequences

- One plugin repository serves every brain: the skills reach Claude, Codex, Antigravity, Devin and API agents the
  same way, through `load_skill`.
- A vendor's hook-based behaviour (ponytail's status band, superpowers' session hooks) is not reproduced; its skills
  are. Verified 2026-11-04 with both repositories: superpowers 7.0.2 (15 skills, hooks skipped) and ponytail 5.3.3
  (6 skills, hooks and commands skipped); a Devin agent with superpowers on listed and loaded `brainstorming`.
- shortcut: GitHub only (no GitLab or private repositories with a token); add a source when a vendor needs it.
