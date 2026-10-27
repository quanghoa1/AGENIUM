# Changelog

All notable changes to Agenium. Dates are Asia/Saigon. Decisions behind each change are in [docs/adr](docs/adr/README.md).

## 0.1.0 — first release (unreleased)

The first public build: the desktop app for Windows and Linux (Ubuntu), and the `agenium` command.

### Teams and orchestration
- Solo agent, or a Lead with workers and an optional planner; tickets are the only channel for work, every state
  change is a typed, validated tool call recorded in an event log (crash-safe, resyncs every view).
- Spec → plan → tickets with acceptance criteria → work in a git worktree per ticket → review per criterion → merge
  through one queue. Reopens carry per-criterion feedback; repeated failures go to the user.
- Ticket dependencies (a ticket waits for what it needs to be merged), write scopes per role, pre-merge checks,
  optional push on close, aging and stall detection, Lead inbox, messaging a running Lead.
- Team templates (coding, research, content, pentest), 27 ready-made roles, agents composed into your own teams.
- The planner drafts plan v1 before the Lead plans (ADR-116).

### Models
- API providers: Anthropic, Gemini, OpenAI-compatible and OpenAI Responses wires; 30+ vendor presets (OpenAI,
  DeepSeek, OpenRouter, OpenCode, xAI, Mistral, Groq, Azure OpenAI, Bedrock, Cloudflare …); local LM Studio / Ollama
  with detected capabilities.
- Custom OpenAI-compatible endpoints: extra headers (secret ones in the vault), per-model settings and effort levels,
  extra request fields and the effort field, under rules checked on save and on every request (ADR-115).
- Vendor CLIs driven headless with every tool call decided by Agenium: Claude Code, Codex, Gemini (Antigravity),
  Grok, OpenCode; login and usability detected; a fallback API model when a CLI cannot run.
- CLIs that run commands on the host get Agenium's sandboxed `shell`, so they test their work without asking
  (ADR-116).
- Effort levels per model by the vendor's own names, prompt caching across vendors, context clearing and compaction,
  retries with back-off, quota and offline states.

### Desktop app
- Overview, chat with the Lead, Kanban board, ticket pages with Monaco side-by-side diffs, live office view, run logs
  with an xterm terminal, memory browser, team organiser, provider and skill/MCP settings, usage and audit trail.
  English and Vietnamese.
- First start prepares the local embedding model (bundled with the installer) before the app opens.
- Strict Content Security Policy: only the app's own code runs in the window; no remote content.

### Safety
- Agent commands run in an OS sandbox (Windows MXC containers, Linux bubblewrap, macOS Seatbelt): writes only in the
  ticket's worktree. Without a working sandbox, commands wait for your approval.
- Encrypted vault for keys (master key in the OS keyring); secrets never reach agents, logs or git; secret scan on
  every commit; redaction in logs.
- Tool caches and installed packages are kept out of commits (ADR-116).

### Memory, skills, tools
- Per-agent memory with a local embedding model (bge-small), keyword and knowledge-graph search, scoped facts, lessons
  with decay, consolidation; strictly gated learning of new skills and lessons.
- `SKILL.md` skills (13 built in), skill overlays and agent-created skills behind evidence gates.
- MCP servers (stdio, Streamable HTTP, legacy SSE, OAuth), bound per agent with their own credentials.
- `view_image` for vision models; `fetch` through an allowlist.

### Optional judge
- TypeSafe Jev for small decisions that save the main agents' tokens (memory gate, skill hint, review assist, skill
  quality). Off by default, fail-open. See [docs/JEV.md](docs/JEV.md).

### Operations
- Backups (hourly incremental, daily full, retention), restore into a new root, migrations with a backup first,
  support bundle without secrets, Markdown project reports, cost per run/ticket/agent.
