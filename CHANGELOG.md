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
- Vendor CLIs run in their YOLO mode and work only through Agenium's own tools (files, apply_patch, shell, background
  commands): nothing asks the user and every change keeps the ticket's write scope (ADR-116, ADR-117, ADR-129).
- A machine without a working sandbox can run agents without one by the user's choice (Settings; Codex included).
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
- Vendor CLIs work only through Agenium's tools (ADR-129): their own shells are off, their own file and search
  tools reach only their scratch folder, so every vendor works under the same sandbox, command gate and write scope
  as an API agent. The vendored Codex sandbox follows upstream f021168.

### Memory, skills, tools
- Per-agent memory with a local embedding model (bge-small), keyword and knowledge-graph search, scoped facts, lessons
  with decay, consolidation; strictly gated learning of new skills and lessons.
- Memory stays fast past 100 000 facts per agent: an approximate (HNSW) vector index takes over from exact search,
  built in the background, checked on every start and rebuilt when it cannot be trusted (ADR-50, ADR-125).
- `SKILL.md` skills (13 built in), skill overlays and agent-created skills behind evidence gates.
- MCP servers (stdio, Streamable HTTP, legacy SSE, OAuth), bound per agent with their own credentials.
- `view_image` for vision models; `fetch` through an allowlist.
- Background commands for every agent: `shell` with `background` starts a server or watcher, `shell_output` reads it
  while it runs, `shell_stop` ends it; all stop with the run (ADR-129).
- A Solo agent's CLI subagents are shown as such: the run page and the Live tab label and indent each subagent's
  steps under the step that started it (ADR-129).

### Optional judge
- TypeSafe Jev for small decisions that save the main agents' tokens (memory gate, skill hint, review assist, skill
  quality). Used once a Jev key is added (every use on by default), fail-open: without a usable key Agenium works as
  it does without the judge. See [docs/JEV.md](docs/JEV.md).
- A report that says done while nothing checked the work after its last change can be sent back once (done check,
  ADR-126); a short rate limit no longer turns the judge off, calls give up after 5 s and reuse one connection.
- The judge can observe only (shadow mode) to measure a use before turning it on; it keeps each ask's billed usage,
  reuses answers it already gave, asks the review's questions together with the done check, and can leave out the
  parts of a long test/build log a ticket does not need (ADR-127).
- `find_code` for coding agents: ask where something happens and get the files and the functions that do it (ADR-128).
- Several named Jev keys that take turns; a key out of quota is set aside until you re-enable it (Agenium tells you
  which), a failing key is retried after 30–60 minutes; every judge use is on by default and Agenium works on without
  the judge whenever no key is usable (ADR-128).

### Operations
- Backups (hourly incremental, daily full, retention), restore into a new root, migrations with a backup first,
  support bundle without secrets, Markdown project reports, cost per run/ticket/agent.

### Known limits
- Windows: the agent sandbox needs MXC process containers, which stable Windows 11 releases (23H2 tested) do not
  provide yet; there every agent command waits for your approval unless you allow running without a sandbox in
  Settings. Git must be installed.
- Anthropic, Gemini and OpenAI APIs are contract-tested, not yet called with a real key; ADR-115's extra request
  fields apply to OpenAI-compatible endpoints only; Devin's CLI cannot be driven.
- Installers are not code-signed and there is no auto-update yet; no macOS build yet.
