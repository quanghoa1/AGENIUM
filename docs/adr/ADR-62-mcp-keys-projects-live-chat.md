# ADR-62: Per-agent MCP keys, projects by folder name or repository, Live tab as a chat

Status: Accepted (2026-10-25, owner's requests after the demo: "an agent's MCP key: its own key when filled in,
otherwise the system key; the Lead requests keys for members and sets them, the user can set them too"; "MCP keys
need not be hidden or in the vault, keep them in the app and show them; logs mask them"; "the project folder is a
folder name or a repo, the app decides where it lives"; "no machine paths in the app"; "idle agents sit in the
lounge, agents with a task stay at the desk"; "the plan on the right as a task list"; "no separate live thinking:
the conversation is a CLI-like chat with expandable thinking")

## Decision

1. **MCP keys.** A server's `env` / `headers` values are its *system key*, kept as written in `config.toml` (the
   earlier refusal of literal secrets and the move of pasted JSON secrets to the vault are dropped for MCP;
   `${secret:<name>}` references still work). An agent may have its *own key* per entry (`Binding::keys`, stored in
   the agent row): it replaces the entry's placeholder (keeping `Bearer `) or the whole value; without one the
   agent uses the system key. `${cred:KEY}` placeholders resolve to the agent's own key (literal, or the vault
   secret its `credentials` names). The app shows both values (agent page "MCP keys", MCP library); the log
   redactor learns every key value an agent's connections use (`App::mask_mcp_keys`, also the bare value of
   `Scheme value`), so run logs never show them. A changed key reconnects: the hub keeps an in-memory fingerprint
   of each resolved connection.
2. **Lead tool `set_member_mcp_key`** (Lead group): `{agent, server, key, value|null}` for a member of its team (or
   itself) that already uses the server (enabling a server stays the user's decision). `get_team` lists each
   member's servers, key names and whether it has its own key (never values). Typical use: the Lead holds the
   system key, obtains a member key through the server, and sets it for the member.
3. **Projects.** `CreateProject` takes `folder` (a folder name under `<root>/projects`, never a path) or `repo` (a
   git URL or source cloned there; the main branch is the clone's HEAD). The desktop field "folder or repository"
   decides by the text; the clone runs off the UI thread. The CLI keeps `--path` for existing repositories.
4. **No machine paths in the UI**: the root (sidebar, settings, setup), project paths (projects, board, command
   palette) and the working folder of command approvals are gone.
5. **Live tab.** The office seats an agent at its desk while it has a task (an open ticket, a run, or for a Lead a
   team with open tickets), even when the task waits; free agents go to the lounge (sofa, armchairs, then standing
   places), one place each. Chairs sit in the middle of the desk; every character carries its name; delegation
   lines carry the ticket key. The right side has two tabs: the agent (model and effort, Tool/Skill/MCP calls, the
   conversation) and the room's plan as a task list (playing = in progress, tick = done, pause = pending). The
   conversation reads like a CLI chat: the turn in progress shows its reasoning in an expandable block (open while
   the model thinks, folded once it writes), the tool being called and the streamed text; only the Lead or a Solo
   agent takes input, a worker is read-only. The separate "live thinking" block is gone. The ticket board's columns
   have equal widths.

## Consequences

- Tests: `own_keys_replace_the_system_key_per_entry` (mcp), `pasted_json_keeps_keys_and_agents_get_their_own`,
  `lead_sets_a_member_mcp_key`, `projects_from_a_folder_name_or_a_repository`; schemas: `set_member_mcp_key`,
  `get_team`.
- Keys in tool-call arguments (the Lead's `set_member_mcp_key`) are masked in the run log because the value is
  registered before the call is logged; a key the Lead read from an MCP output earlier in the run is masked only if
  its pattern is a known secret format.
