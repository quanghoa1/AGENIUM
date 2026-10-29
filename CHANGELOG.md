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
- Projects end: only the user closes a done project (never while one of its team's runs is going), and its team goes
  free, whole, for another project; reopening offers the team that worked on it, now or once it is free (ADR-130). A
  follow-up of a closed ticket is a new ticket linked to it (`follows`).
- A project's ticket prefix is made from its name (`Demo shop` → `DS-1`) unless you give one, and no two projects
  share one; projects are shown by the name you gave them.
- The planner drafts plan v1 before the Lead plans (ADR-116).
- A worker whose quota runs out in the middle of a ticket loses no work and keeps its ticket: the worktree and the
  conversation stay, the user gives it another model (another vendor's API model or CLI, which gets the conversation
  so far), which frees it at once, and it goes on (ADR-131, ADR-132).
- Agenium never moves an agent's work to another model by itself: a CLI that cannot run sets the agent aside and says
  why; a **Resume** button on the agent's card, its page and the Needs you item puts it back to work (ADR-132).
- An agent out of quota goes back to work with the same model once the quota should be back: at the vendor's window
  reset when Agenium can read it, otherwise tried again from 30 minutes on (ADR-135).
- The user's stop reaches a running command and a wait for approval at once (it waited for the command's timeout, up
  to 10 minutes); an approval request a stopped run gave up leaves Needs you (ADR-131).

### Models
- API providers: Anthropic, Gemini, OpenAI-compatible and OpenAI Responses wires; 30+ vendor presets (OpenAI,
  DeepSeek, OpenRouter, OpenCode, xAI, Mistral, Groq, Azure OpenAI, Bedrock, Cloudflare …); local LM Studio / Ollama
  with detected capabilities.
- Custom OpenAI-compatible endpoints: extra headers (secret ones in the vault), per-model settings and effort levels,
  extra request fields and the effort field, under rules checked on save and on every request (ADR-115).
- Vendor CLIs driven headless with every tool call decided by Agenium: Claude Code, Codex, Gemini (Antigravity),
  Grok, OpenCode; login and usability detected; an API model of your choice draws CLI agents' lessons.
- Each vendor CLI resumes its own session of a ticket across the ticket's runs (a question answered, a review
  reopened, a quota used up): it goes on with its own context and is sent only what is new; a new ticket starts a new
  session (ADR-134).
- One CLI account can run several sessions at once: logins are kept in step while runs go on, so a refreshed token
  reaches the other runs and the user's own CLI at once, and a turn refused for a token another run had just used
  runs again with the new one (ADR-134, ADR-135). Codex runs use the user's own login file (a link, not a copy), so
  its sessions share one refresh token as on the user's machine (ADR-136); Claude Code and Grok runs get the login
  without its refresh token while the access token lasts, so they cannot use it up (ADR-137).
- A command longer than a CLI's own tool wait goes on as a job the run waits for; its end comes to the agent as a
  message (ADR-133).
- Skills are written in the app with their references, scripts and assets: fields for the name, description and tags,
  files added in place or uploaded, one save that checks the whole skill (ADR-138).
- Paths crafted to slip past the checks are caught: every path a CLI's tool names, links out of the write scope or the
  worktree, hard links (no read or write through one), dotfiles of the CLI's scratch home (path review, ADR-56,
  ADR-135).
- Vendor CLIs run in their YOLO mode and work only through Agenium's own tools (files, apply_patch, shell, background
  commands): nothing asks the user and every change keeps the ticket's write scope (ADR-116, ADR-117, ADR-129).
- A machine without a working sandbox can run agents without one by the user's choice (Settings; Codex included).
- Vendor CLIs held to account again (ADR-56): Claude Code's hook path is quoted (a path with a space let its tools run
  without Agenium's decision); each CLI waits up to an hour for one of Agenium's tools (Claude Code gave up on an
  80-second command and the model ran it again); a dead OpenCode server or a CLI gone with its output open ends the
  turn instead of waiting 45 minutes; a used-up subscription is recognised by the CLI's own words and a lost login
  says to log in again; a refreshed login is handed back whole, one run at a time.
- A tool call a model gets slightly wrong costs no step where its meaning is clear: a worker's report or question
  without its ticket is about its run's ticket, a report's criteria under other names (`ac_results`, `id`) are read
  as `acceptance` and `ac_id`, and a review's verdict sent as `action` as `decision`; a CLI is told only the tools
  its run offers.
- Effort levels per model by the vendor's own names, prompt caching across vendors, context clearing and compaction,
  retries with back-off, quota and offline states.

### Desktop app
- Overview, chat with the Lead, Kanban board, tickets in a popup over any view with Monaco side-by-side diffs, live
  office view, run logs with an xterm terminal, memory browser, team organiser, provider and skill/MCP settings, usage
  and audit trail. English and Vietnamese.
- Usage: tokens and cost per day by vendor (stacked charts with hover and a table view), rankings by vendor and model,
  the vendors' prices (an API's per million tokens, a CLI's subscription and its windows), per agent.
- Runs say what memory did: the memories a work run was reminded of, and what a memory run learned (added, updated,
  retired, with the old text) and from which ticket.
- An agent's skills say where they come from: **Updated** (its own copy of a system skill, the system's unchanged),
  **Learned** (written from its own lessons), its role's, or the system's.
- The team starting and a project done are told (with the way there; a done project also as a desktop notification);
  the Lead's chat leads to the live office, the tickets and the runs, and its ticket keys open the ticket.
- First start prepares the local embedding model (bundled with the installer) before the app opens.
- Strict Content Security Policy: only the app's own code runs in the window; no remote content.

### Safety
- Agent commands run in an OS sandbox (Windows MXC containers, Linux bubblewrap, macOS Seatbelt): writes only in the
  ticket's worktree. Without a working sandbox, commands wait for your approval.
- Settings describe the three command modes at once in plain words (run in the sandbox, ask me every time, sandbox
  only and never ask) with what each does on this computer right now; a mode's description used to show only once
  it was chosen.
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
- MCP servers (stdio, Streamable HTTP, legacy SSE, OAuth), bound per agent with their own credentials: two agents
  reach one server with different keys and so different rights (an admin's, a member's), API and CLI agents alike;
  an own key pasted bare keeps the system key's `Bearer ` (it reached the server without it and was refused).
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
- zstd raised to 0.14.3 for three advisories of 2026 (RUSTSEC-2026-0391, -0398, -0403), found by cargo-deny in the
  local CI; the backup and agent export formats are unchanged.

### Known limits
- Windows: the agent sandbox needs MXC process containers, which stable Windows 11 releases (23H2 tested) do not
  provide yet; there every agent command waits for your approval unless you allow running without a sandbox in
  Settings. Git must be installed.
- Anthropic, Gemini and OpenAI APIs are contract-tested, not yet called with a real key; ADR-115's extra request
  fields apply to OpenAI-compatible endpoints only; Devin's CLI cannot be driven.
- Installers are not code-signed and there is no auto-update yet; no macOS build yet.
