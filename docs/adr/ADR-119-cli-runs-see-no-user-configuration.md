# ADR-119: A vendor CLI run by Agenium sees none of the user's configuration

Status: Accepted (2026-10-27; owner: "các cli khi chạy với agenium không dính tới config của hệ thống và người dùng …
mọi thứ đó đều phải bỏ hết đi để dùng những thứ của agenium (tất nhiên là không xóa của người dùng)"). Extends ADR-56,
ADR-68, ADR-108.

## Context

Each driver already gave its CLI a per-run home: `CLAUDE_CONFIG_DIR`, `CODEX_HOME`, `GROK_HOME`, opencode's `XDG_*`,
agy's `HOME`, holding only Agenium's settings, its MCP server and hook, and a copy of the user's login (handed back
if the CLI refreshes it). The environment is an allowlist (no vendor keys or `CLAUDE_*`/`CODEX_*` variables). A probe
of what each CLI actually loads found two channels left:

- **Claude's claude.ai account.** The connectors of the user's claude.ai account (Gmail, Google Drive, Calendar,
  Slack, Notion …: 19 on the owner's account) and its skills (`anthropic-skills:*`) come with the login, not the
  configuration folder, so a fresh `CLAUDE_CONFIG_DIR` still listed them.
- **The user's home.** Every CLI but agy still had the user's `HOME`/`USERPROFILE`, where shared locations such as
  `~/.agents/skills` are read by some CLIs; Claude Code also kept state in the user's `~/.claude.json`.

## Decision

1. **Claude:** `ENABLE_CLAUDEAI_MCP_SERVERS=false` and `--disable-slash-commands` (no skills or slash commands; the
   agent's skills reach it through Agenium's `load_skill`). Claude Code's own `@builtin` plugins and built-in
   subagent types stay: they are the CLI, not the user's configuration (a team member is not given the Agent tool).
2. **Every CLI:** `HOME` and `USERPROFILE` point at the per-run home unless the driver set them
   (`run::isolated_home`). The user's files are never written or removed; their own CLI keeps using them.
3. **Probe:** `cargo run -p agenium-cli-backend --example isolation_probe -- <scratch> [--turn]` builds each
   installed CLI's per-run home as a run does and asks the CLI, without a model turn, which MCP servers (and plugins,
   extensions) it would load; `--turn` reads Claude's `init` event (MCP servers, tools, plugins, skills, agents, slash
   commands) from one short Haiku turn. The user's own environment is not probed (listing MCP servers starts them).

Measured 2026-10-27 on Windows: codex, agy, grok — only `agenium`; claude — MCP servers `["agenium"]`, 9 tools (the
ones Agenium allows, no foreign MCP tool), skills `[]`, slash commands `[]`, plugins only `@builtin`; one golden task
through each of claude, codex, grok, agy and opencode with the isolated home: 5/5 PASS (logins still work); the user's
`~/.claude.json`, `~/.codex/config.toml`, `~/.grok/config.toml`, `~/.gemini/settings.json` and opencode config unchanged by
a run. opencode's own listing commands do not run in this version; its isolation rests on the `XDG_*` and home
variables (ADR-68) and the golden task.

## Consequences

- `agenium_cli_backend::run::isolated_home`; the Claude driver's flag and variable; `examples/isolation_probe.rs`.
- Machine-wide policy (an organisation's managed settings for a CLI) still applies: it is the administrator's, not
  the user's, and a CLI enforces it whatever the home.
