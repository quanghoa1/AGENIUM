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

## Amendment (2026-11-04): hooks run Agenium's way, and nothing stays once a plugin is gone

The owner: a plugin without its hooks does not work as its author meant (superpowers and ponytail both put their rules
in at `SessionStart`); run them Agenium's way, and leave nothing of a removed plugin in Agenium.

1. **Taken**: the command hooks of `SessionStart`, `UserPromptSubmit` and `PreToolUse` (from `hooks/hooks.json` or the
   files or object the manifest's `hooks` names), written in `agenium-plugin.json` as Claude Code's `hooks` object;
   their folder (`hooks/`) is copied. Not run, and said so: `SubagentStart` (Agenium's agents are not subagents; every
   run starts with `SessionStart`), `PostToolUse`, `Stop`, `SubagentStop`, `PreCompact` (their text would stay in the
   stored conversation), hooks that are not commands, interface `modules`.
2. **Run** (`plugins/hooks.rs`): each command in Agenium's sandbox, only for the agents that have the plugin on: the
   plugin's folder read-only, its `data/` the only folder it writes, no network, at most 60 s, Claude Code's input
   (`hook_event_name`, `session_id`, `cwd`, `source`/`prompt`/`tool_name`+`tool_input`) on stdin, `CLAUDE_PLUGIN_ROOT`,
   `CLAUDE_PLUGIN_DATA` and `CLAUDE_PROJECT_DIR` set and put in the command; its PowerShell form where the sandbox runs
   PowerShell. Without a sandbox a hook is not run. Exit 2 or a `deny`/`ask` decision refuses a tool call (`ask`
   refuses too for now and tells the agent to use `request_permission`); a hook never allows what Agenium refuses.
3. **Nothing stored**: what `SessionStart` says joins the run's instructions and what `UserPromptSubmit` says joins that
   turn's instructions, rebuilt at every run, never in a stored message. Switching a plugin off or removing it ends
   the agent's kept CLI sessions (a resumed session would carry what the hooks said). Removal deletes the plugin's
   folder (skills, hook scripts, the data its hooks wrote) and its record; a stopped install's `.fetch-`/`.new-`/`.old-`
   folders are swept.
4. **Shown**: the chat shows each hook a turn ran as the plugin's, folded (`Plugin ponytail: its SessionStart hook`,
   red when it failed), from a record kept apart from the conversation (`plugin-hooks:<project>`) that no model
   reads; a call of a plugin's skill or server reads `Plugin::superpowers::brainstorming`.
5. A plugin's skill may name an example path that does not exist (superpowers' `scripts/tool.sh`); only a reference
   leaving the plugin's `skills/` is refused.

Verified 2026-11-04 in the app with a Devin Solo: ponytail's `SessionStart` (node) ran in the sandbox and the agent
quoted "PONYTAIL MODE ACTIVE"; after removal nothing of ponytail was left in Agenium's root and the next turn had no
ponytail text from Agenium. superpowers' `SessionStart` fails on Windows: it starts Git Bash, which cannot start in the
MXC container (S2); its skills work. The user's own CLI plugins (installed in Devin or Claude Code themselves) are the
CLI's, not Agenium's, and still load in its runs.

## Amendment (2026-11-16): a private GitHub repository, with a token the user pastes

The original decision read public GitHub only ("no private repositories with a token"). A user's own plugin kept in a
private repository could not be installed. The owner: let the user choose Public or Private; for Private, paste the
`GITHUB_TOKEN` into a field, each time, and keep no key.

1. **The form** (Library → Plugin): Public / Private. Private shows a password field `GITHUB_TOKEN`, with how to make a
   read-only token (a fine-grained token with the repository under Repository access and Contents: Read-only). The token
   lives in the panel's state for that preview and install and is cleared after (install, cancel, close).
2. **The download** (`plugins/source.rs`, `download`): the same codeload URL (`/zip/<ref>`, so the archive still carries
   the commit as its comment), anonymous for Public, with `Authorization: token <t>` for Private. Refused without a
   token (GitHub answers 404 for a private repository), the error says to choose Private and paste a token; refused
   with one, that it may be mistyped, expired or not allowed to read the repository. The token is never in a message.
3. **Nothing keeps the token**: not the install record, `agenium-plugin.json`, the config, the vault or a log; the
   desktop commands take it as an argument for the one call. The install record keeps only `private: true`, so an
   update of that plugin asks for a token again (a dialog) before it reads the vendor. A folder source ignores it.
4. Not done, on purpose: reading the token from git's credential helper or an environment variable (tried first, then
   dropped: every install ran the helper, which on Windows can open Git Credential Manager's sign-in window, and an
   environment variable must be set before the app starts).

Verified 2026-11-16: the private `quanghoa1/agenium-pentest` downloaded with a pasted token (commit `794cbe2`); without
one, the "choose Private" message; `obra/superpowers` anonymously; `download` tests with a stand-in server (public, private
with the token, none or a wrong one, the token not in the message); the registry plugin tests; the form in the UI.

## Amendment (2026-11-17): skills one by one, learned into, rebased on the vendor's update

The owner: a plugin's skills show on the Skills page under the plugin's name; an agent can be given one skill of a
plugin, or the whole plugin; agents may learn into a plugin's skill and upgrade it like a skill the user imported; the
vendor's update still goes through, and the user or a helper then brings the agent's learned changes onto the new
version, without losing them. The vendor's copy stays as the vendor wrote it (the user does not edit it either).

1. **One skill alone**: an entry `plugin:<plugin>/<skill>` (`agenium_skills::plugin_skill_of`) loads that skill with
   origin `plugin`, and **no MCP server or hook of the plugin** (those follow only `plugin:<plugin>`; the user adds a
   server the skill needs). Checked like the others (`check_skills`: the plugin installed, the skill in it). The
   library listing (`skill_library`) adds `plugin_skills` (name, plugin, `entry`, who uses it); the Skills page shows
   them read-only under the plugin's name; an agent's Skills tab has them one by one (on, greyed, when the whole plugin
   is on).
2. **Learned into, in the agent's own copy**: an agent's skill of a plugin skill's name is its overlay of it (was
   dropped as read-only), checked by the plugin's rules (a sibling skill's file, an example path). `base_skill` finds a
   plugin's skill the agent has on (whole or alone), so `propose_skill_update` works on it under the same gates as a
   library skill (a trusted lesson held in two closed tickets, a reason, no injected instruction or secret, no new
   contradiction). The overlay view names its base `plugin:<name>`; promoting it over the vendor's copy is refused.
3. **The vendor's update**: installed as before; then (`refresh_plugin_users`) every agent using the plugin has its
   copies rebased (the 3-way merge of ADR-86: the old base rebuilt from the recorded change, the new one, the copy),
   keeping what it learned; a copy that does not merge cleanly stays as it was, with a Needs-you; a skill the vendor
   dropped is taken off the agents given it alone (their own copy stays as their skill). The install answers what it
   did (`refreshed`: rebased, conflicts, dropped) and the plugins page says so.
4. **Merged with help**: on a stale copy, the Memory page's "Ask the helper to merge" has the skill writer's model draft
   the copy on the new base keeping what the agent learned (`Runtime::draft_overlay_merge`, nothing written); the user
   reads it, may edit it, and accepts it (`App::accept_overlay_merge`: a valid skill, no injected instruction, no
   secret; recorded against the new base). Library skills' copies get the same help.

Tests: `agent/skills/tests/skills.rs` (one skill alone, the agent's copy as overlay, a copy that refers to a sibling
skill), `orchestrator/tests/team/plugin_skills.rs` (one skill without MCP, the learning gates, promote refused, v2
rebased with the lesson kept and a dropped skill taken off, v3 in conflict, an injected merge refused, the merge
accepted, removal), `helper_merge.rs` unit test; the UI smoke installs a plugin through the form, finds its skills on
the Skills page and gives one alone to an agent.


## Amendment (2026-11-17): the token is kept for updates

Owner: a fine-grained token is shown once, when it is made, so pasting it again at every update means making a new
one. Point 3 above is replaced:

- An install from a private repository keeps the token it read with in Agenium's secret store (the OS keyring, or the
  encrypted vault where there is none, as the API keys), under `plugin-github/<owner>/<repo>` (lower case): one token
  per repository, shared by the plugins of a marketplace repository. It is still in no install record, config,
  message or log, and it is registered with the redactor when read.
- An update (and a new preview of that repository) reads it with the token pasted, else the kept one. The update
  dialog asks only when none is kept, or when GitHub refuses the kept one (expired or revoked): the error is shown and
  the dialog opens, and the new token replaces the kept one once the update is installed.
- Removing the repository's last plugin deletes its token.

Test: `plugins::token::tests::a_repositorys_token_is_kept_used_replaced_and_forgotten`.
