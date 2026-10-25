# PROGRESS — Agenium (fresh implementation)

Last full verification: 2026-10-24 (Asia/Saigon) on Windows 11 Pro (10 cores, 32 GB), Rust 1.99.1
`x86_64-pc-windows-gnu`, git 2.45. After M3: fmt PASS, clippy `-D warnings` PASS, `cargo test --workspace` PASS (**210 passed**, 0 failed, 2
helpers ignored; run without `AGENIUM_TEST_MODELS`, the local embedding-model tests were last run with it after M1.5), `xtask schemas --check` PASS, scripted golden tasks **5/5** PASS. Decisions and deviations of this implementation: [`adr/ADR-52-fresh-implementation.md`](adr/ADR-52-fresh-implementation.md).
Front-ends: the headless CLI `agenium` and the desktop app `apps/desktop` (Tauri 2, [ADR-60](adr/ADR-60-desktop-app.md)).

## Milestones

### M0 — scaffolding (backend): DONE
| Item | Status | Evidence |
|---|---|---|
| M0-1 workspace, LICENSE/NOTICE, local CI | DONE | `Cargo.toml`, `devtools/scripts/ci-local.{ps1,sh}`; cargo-deny not installed (not run) |
| M0-2 config (`resolve_root`, layout, `config.toml`) | DONE | `agenium-config` tests (3 priority branches, layout, round trip) |
| M0-3 setup wizard backend + system requirements | DONE (backend) | `setup.rs`, `sysreq.rs`; `agenium setup`, `agenium doctor` |
| M0-4 core: ids, ticket, state machine, `dep_satisfied` | DONE | `agenium-core`: permission table test + property tests (no illegal transition, terminal states, dep rule) |
| M0-5 store: SQLite, migrations + pre-migrate backup, event log, idempotency | DONE | `agenium-store` tests; **20x hard-kill crash test** |
| M0-6 secrets (hybrid vault, keyring timeout, redaction, secret scan) | DONE | `agenium-secrets` tests; real Windows Credential Manager probe |
| M0-7 vcs (git CLI): worktree/branch per ticket, auto-commit with secret scan, rebase + ff merge queue, remote test | DONE | `agenium-vcs` tests (bare-repo remote; real GitHub/GitLab NOT RUN) |
| M0-8 project (`agenium.toml`, layout, git init) | DONE | `App::create_project`, `agenium project create` |
| M0-9 bus + resync from the log | DONE | `agenium-bus` tests |
| M0-10 reconcile DB <-> git | DONE | 8 injection points heal (`orchestrator/tests/reconcile.rs`) |
| M0-11 backup full/incremental, retention 24/7/4, restore into a new root | DONE | store + orchestrator backup tests (content hashes) |

### M1 — Solo agent: DONE except the UI parts
| Item | Status | Evidence |
|---|---|---|
| M1-1 Provider trait + Anthropic, Gemini, OpenAI-compatible adapters | DONE | contract tests on a local HTTP server; **real run on OpenCode Go** (OpenAI-compatible, DeepSeek v4.1 Flash); Anthropic and Gemini real APIs NOT RUN (no keys) |
| M1-1b Local / custom endpoint capability flags (ADR-31) | DONE: tools / vision / context length detected from LM Studio (`/api/v0/models`) and Ollama (`/api/show`), hand-set `[[providers.model_caps]]` win; `provider models` shows them; a run on a model without tool calling is refused with the reason; the known context window drives the checkpoint. (No "network allowlist" entry needed: Agenium does not filter its own provider traffic; agent commands have no network.) | `agent/llm/src/local.rs`, `adapters.rs` local tests, `runtime.rs::model_without_tool_calling_is_refused` |
| M1-1c OpenAI Responses wire (OpenCode `gpt-*`, `grok-*`, `muse-spark*`) | DONE (contract tests on a local server; not run against the real relay: the owner's key is for DeepSeek models only) | `agent/llm/src/responses.rs`, `adapters.rs::opencode_router_picks_the_wire_per_model`, `responses_failure_is_a_stream_error` |
| M1-2 retry / Offline / QuotaExhausted | DONE | retry unit tests, 529/429/drop/402 contract tests, runtime test |
| M1-3 prompt caching order, effort mapping, prices | DONE | breakpoints asserted in contract tests; built-in Anthropic prices + `pricing.toml` |
| M1-4 work tools | DONE | `agenium-tools/tests/tools.rs` |
| M1-5 protocol tools (24, schemas generated), 3 invalid calls stop, nudge | DONE | core + backend loop tests, `schemas/tools/` |
| M1-6 loop, `max_steps`, loop detection | DONE | `agenium-backend/tests/loop.rs` |
| M1-7 approval mode before the sandbox (YOLO fail-closed) | DONE | `RunGate`, CLI terminal approver, denied-command runtime test |
| M1-8 chat console / agent view | CLI only (UI later) | `agenium chat`, live text + tool calls on the terminal |
| M1-9 tokens + estimated cost per run/ticket | DONE | runtime test (usage on ticket), price table test |
| M1-10 eval: 5 golden tasks + runner + report | DONE | scripted 5/5; **real model 5/5** (DeepSeek v4.1 Flash via OpenCode Go, `quality/eval/results/`) |
| M1-11 memory hook + `fact_usage` | DONE (hashing embedder stand-in) | `agenium-memory` test |
| **Exit**: Solo agent: ticket -> worktree -> report -> self-review -> merge -> close | **MET** with a scripted model, and end-to-end through the CLI against a fake OpenAI-compatible HTTP server | `orchestrator/tests/runtime.rs`; CLI smoke (see below) |

CLI end-to-end smoke (2026-10-23): `setup -> project create -> provider add (openai_compat, local fake server) -> agent solo
-> chat "Hãy thêm file chào hỏi" -> run` produced spec v1, plan v1, AGN-1, a work run, a review run, merge into `main`
(`AGN-1: Write the hello file`) and Closed.

### M1.5 — Agent registry: DONE except the UI forms (M15-6) and parts of the graph benchmark plan
| Item | Status | Evidence |
|---|---|---|
| M15-1 agent profile, model switch keeps memory/MCP/skills | DONE | `runtime.rs::vendor_switch_keeps_memory_skills_and_conversation` |
| M15-2 local embedder + memory pipeline + hybrid retrieval | DONE | `bge-small-en-v1.5` via candle (pure Rust; ONNX Runtime has no GNU build); `agenium-memory/tests/{local_model,pipeline}.rs`; trigger test `runtime.rs::closing_a_ticket_teaches_the_assignee`; real model (below) |
| M15-3 McpHub stdio + Streamable HTTP, credential binding | DONE (no OAuth, no legacy HTTP+SSE) | `agenium-mcp/tests/mcp.rs` (test server binary + HTTP stub), `runtime.rs::mcp_tools_reach_the_agent_with_its_credential`, `mcp_settings.rs`; manual: `@modelcontextprotocol/server-everything` 2.0.0 |
| M15-4 SkillRegistry, `load_skill`, skills index | DONE (overlay updates = M4.5) | `agenium-skills/tests/skills.rs`, `runtime.rs::skills_are_indexed_loaded_and_readable` |
| M15-5 export/import JSONL, delete agent | DONE | `orchestrator/tests/agent_io.rs` (byte-identical memory round trip, tamper/secret refusal, delete rules), core_002 migration test |
| M15-6 Settings forms (MCP, skill) | CLI equivalents only (UI later) | `agenium mcp add/import/test`, `agenium skill import/validate/show` |
| M15-7 graph memory core (agent_002 migration, entity/alias/edge/fact_entity, merge log, tombstones) | DONE | `agenium-store/tests/graph.rs` (v1 -> v2 migration with data, identity/aliases, merge rewiring + log, plan uses `edge_out`/`edge_in`, property test vs reference BFS), backup round trip, export/import, **20 hard kills** during graph writes: integrity, FKs, degree cache, merge chains consistent |
| M15-8 extraction + entity resolution | DONE (bands not calibrated) | `agenium-memory/src/{pipeline,resolve}.rs`, unit + pipeline tests; labelled 2 000-pair precision set NOT RUN |
| M15-9 graph channel + RRF + re-rank + scope hard filter | DONE | `agenium-memory/src/retrieve.rs`, GM-3 (graph finds what text misses), GM-2 (scope leak 0) |
| M15-10 benchmark | PARTLY: traversal latency at 100 k / 1 M edges RUN (targets met with the hub cap) | `docs/reference/STORE-SCALE.md` §8.1; QA golden set, concurrency, cold cache NOT RUN |
| M15-11 golden scenarios GM-1..5, GM-7 | DONE (scripted model) | `agenium-memory/tests/graph_memory.rs`; "second ticket >= 20 % fewer steps" seen on the real model in the skill/MCP run (11 -> 5), not as a GM task |

### M2.5 — CLI backend: ACCEPTANCE MET with codex and agy ([ADR-56](adr/ADR-56-cli-backend-headless.md))
| Item | Status | Evidence |
|---|---|---|
| Detect CLIs in PATH: version, login state, minimum version | DONE | `agent/cli-backend/src/detect.rs`, `agenium doctor` |
| Headless JSON-stream drivers | DONE: codex (`exec --json`, resume per turn), agy (stream-json in/out) | `codex.rs`, `agy.rs`, `run.rs` |
| Per-run home, the user's CLI configuration not used | DONE (codex: `CODEX_HOME`; agy: `USERPROFILE`/`HOME`) | `tests/stream_run.rs`, unit tests |
| YOLO mode, permissions decided by Agenium | DONE: codex inside Agenium's sandbox (MXC/bubblewrap/Seatbelt, live stdio); agy via its `PreToolUse` hook -> Agenium's gate (fail-closed) | `security/sandbox/src/service.rs`, `tests/contained.rs`, `tests/stream_run.rs` |
| Protocol tools through Agenium's MCP server | DONE | `agent/mcp/src/server.rs`, `tests/server.rs` |
| Limits / mandatory report from the event stream | DONE | `tests/stream_run.rs` |
| Golden tasks through a CLI | **codex gpt-6-luna 5/5**, **agy gemini-3.8-flash-low 5/5**, **claude haiku 5/5** | `quality/eval/results/20261024T055002Z-cli-codex-gpt-6-luna.json`, `20261024T060626Z-cli-agy-gemini-3.8-flash-low.json` |
| API fallback when the CLI cannot run | DONE (`[cli] fallback_provider/fallback_model`) | `orchestrator/tests/runtime.rs::cli_agent_falls_back_to_the_api_when_configured` |
| Contract per minimum version | DONE (stream parsing tested on lines captured from codex 0.163.0 / agy 1.4.4; older = not used) | `codex.rs`, `agy.rs` tests |
| claude driver | DONE (hook control, per-run `CLAUDE_CONFIG_DIR`) | `agent/cli-backend/src/claude.rs`, `quality/eval/results/20261024T065834Z-cli-claude-haiku.json` |
| opencode driver (OpenCode's free models) | DONE: `opencode serve` driven through its HTTP API, every permission decided by Agenium; **5/5** golden tasks with `deepseek-v4.1-flash-free` | [ADR-68](adr/ADR-68-opencode-cli-free-models.md), `quality/eval/results/20261025T050257Z-cli-opencode-deepseek-v4.1-flash-free.json` |

Embedding model measured here (release build, `agent/memory/tests/local_model.rs`, own 60-fact fixture
`tests/data/retrieval.json` because the S6 pair set is not in the pack): load 0.09 s; EN recall@1 0.733, **recall@5
0.967** (S6 pass >= 0.85); Vietnamese query -> English fact recall@5 0.30 (S6 measured 0.167: memory stays English,
ADR-40); **46 facts/s single, 77 facts/s batched** (S6 pass >= 20; fastembed/ONNX in S6 was ~5x faster), query
embedding + ranking ~22 ms. Debug builds are ~2x slower.

### M3 — Ticket system: DONE except the UI (Kanban drag and drop, S7 live path)
Most of the ticket system was built with M1 (the Solo exit criterion needs it); M3 adds the user-side views,
overrides and audit, and closes the gaps found with the CLI backends.
| Item | Status | Evidence |
|---|---|---|
| M3-1 Discovery -> RequirementSpec (locked on approval) -> Plan with versions/approval, `revise_plan`; no ticket outside the plan | DONE | `lifecycle.rs`, `team.rs::team_spec_and_plan_wait_for_the_user`, `plan_revision_follows_the_newest_approved_spec`; CLI `agenium plan` (spec + plan versions, what waits for approval and the exact `approve` command, progress per plan item), `agenium approve spec|plan` |
| M3-2 permission matrix of the state machine, user override | DONE | core table-driven permission test; overrides: `user_set_priority`, reassignment of an Assigned ticket by the user only (Lead cannot), user review/close/cancel/answer through the same intents (`Actor::User`, recorded as `user`) -> `views.rs` tests; CLI `ticket priority|assign|review|close|cancel` |
| M3-3 every protocol tool + validation rules, structured forms, template per ticket kind | DONE | 24 tools, `schemas/tools/`; **every JSON example of the architecture page parses** (`protocol.rs::design_document_examples_parse`); report templates coding/pentest/research (`templates.rs`) |
| M3-4 scheduler unlocks on `deps_satisfied` (Closed, or Resolved + approved (+ merged)) | DONE | `lifecycle.rs::dependencies_unlock_on_approve_and_merge_not_close`; priority then age (`sched.rs`) |
| M3-5 Kanban, ticket view, audit trail | DONE headless: `App::board` (columns, blockers, open questions, flags), `ticket show`, `App::audit` = committed events (actor by name, originating intent) + refused protocol calls from the run logs; export `ticket audit --format md|jsonl --out`. **500 tickets: board in ~7 ms** (debug). UI = later | `orchestrator/tests/views.rs` |
| Review in one run (approval reports the real merge result + next step; review runs nudged; failed git ops replayed every round) | DONE: claude haiku 5/5 with 2 runs per task (was 3) | `quality/eval/results/20261024T090226Z-cli-claude-haiku.json`, `runtime.rs` nudge regression, `reconcile.rs::failed_merge_is_retried_without_a_restart` |

### M4 — Team mode: IN PROGRESS ([ADR-57](adr/ADR-57-team-mode-lead-inbox.md))
| Item | Status | Evidence |
|---|---|---|
| Lead inbox (blocking questions, permissions inside the allowlist, plan items without tickets, tickets without a usable worker, non-blocking questions) + Lead inbox run each round | DONE | `orchestrator/src/inbox.rs`, `team.rs::lead_inbox_order_sla_and_escalation`; CLI `agenium inbox` |
| SLA of blocking questions -> System comment + NeedsUser (once); Lead escalation (`answer_question.escalate`) | DONE | same test |
| Team templates: coding, pentest, research, content (TOML, user overrides in `<root>/templates`), create a team from a template (agents, ROLE.md, role scope, report template) | DONE ([ADR-58](adr/ADR-58-team-templates-and-write-scope.md)) | `team_templates.rs` tests; CLI `agenium team templates|template|import|create|show` (smoke-tested on a real root) |
| Write scope: ticket `scope_paths` AND role allowed paths, widened by path grants; enforced by the file tools and at `submit_report` (covers shell and CLI backends) | DONE | `paths.rs::write_scope`, `team.rs::team_from_template_and_write_scope` |
| Stall-nudge (decision 28): a ticket whose runs keep stopping gets, after `stall_minutes`, up to `stall_nudges` more runs with a System nudge, then goes to the user once (replaces the immediate NeedsUser) | DONE | `runtime.rs::quota_exhaustion_parks_the_agent_and_repeated_failures_escalate` |
| Planner (owner's decision): a template role that drafts plan v1 after the spec is approved (`submit_plan_draft`, read-only run); the Lead reviews the draft and creates the plan; never takes tickets. Without a planner the approved spec is a Planning item for the Lead | DONE ([ADR-59](adr/ADR-59-planner-role.md)) | `team.rs::planner_drafts_and_the_lead_creates_the_plan`, `runtime.rs::planner_drafts_then_the_lead_creates_the_plan` |
| Subagents only for Solo (owner's rule); claude tool allowlist, agy/codex withheld tools | DONE (ADR-56 addendum) | `claude.rs`/`agy.rs` tests; real `fix-config-key` PASS on claude, codex, agy |
| **Team golden task with a real model** (DeepSeek v4.1 Flash): Vietnamese request -> Lead spec (user approves) -> planner draft -> Lead plan (user approves) -> tickets assigned to backend and frontend, frontend depends on backend -> work, reviews (1 reopen), merges -> both Closed | **PASS** (1 run, 16 agent runs, ~9 min; 12 command denials without an approver; the frontend asked/permission-parked 5 times) | `quality/eval/results/20261024T105841Z-team-deepseek-v4.1-flash.json`; `cargo run -p agenium-eval -- --team ...` |
| Dependent tickets start from the main that holds their dependencies (worktree fast-forward before the first run, new base recorded) | DONE (found by the real team run) | `vcs.rs::a_dependent_worktree_catches_up_with_main_before_its_work` |
| Team end to end: approved plan -> Lead creates/assigns tickets -> worker question answered by the Lead -> dependent ticket starts after approve + merge -> both closed | DONE (scripted model) | `runtime.rs::team_lead_inbox_takes_a_plan_to_closed_tickets` |

### UI-1 — Desktop app (from the AGENIUM-UX mockup): FIRST CUT ([ADR-60](adr/ADR-60-desktop-app.md))
| Item | Status | Evidence |
|---|---|---|
| Tauri 2 shell in its own workspace, builds with the GNU toolchain; IPC commands over App/Runtime; live events, run output and command approvals as Tauri events; autopilot; kill switch | DONE | `apps/desktop/src/{main,state,api}.rs` |
| UI (Vite + React + Tailwind, mockup's look, light/dark, vi/en): overview, needs you (actionable), tickets (board, plan, Lead inbox), ticket view (AC, timeline, inline answers, runs, audit, overrides), runs (steps, live), projects, agents (model switch), teams (from template / solo), providers & CLIs, templates, audit, usage, settings, setup, command palette | DONE | `apps/desktop/ui/src/` |
| Real app checked through the WebView2 DevTools port: all read commands on a real root; Vietnamese request in the Lead console -> DeepSeek reply -> spec approval in Needs you via live events | DONE (manual, scripted CDP) | ADR-60 §5 |
| `App::needs_you` (derived "Needs you" list), `runs::recent` | DONE | `team.rs::needs_you_lists_what_waits_for_the_user` |
| UI-2: safe Markdown renderer (chat, tickets, comments, runs), ticket changes (diff of the live branch or of the merged commit), board drag-and-drop for the user's transitions (assign / close / cancel; reopen goes to the ticket for per-AC feedback; empty columns collapse), shell steps as terminal blocks, agent page (model, Make Lead, runs, memory facts, skills & MCP), selective desktop notifications (NeedsUser), full vi/en strings | DONE | `ui/src/components/{Markdown,DiffView}.tsx`, `views/*`; real app screenshots; drag tested once through CDP (assign), the rest by hand |
| M4: the user puts another member in the Lead slot (`App::set_team_lead`; CLI `team lead`; UI "Make Lead") | DONE | `team.rs::the_user_puts_another_member_in_the_lead_slot` |
| Monaco editor / xterm terminal (lightweight viewers instead), skills/MCP forms, installer, CI UI tests | NOT DONE | |

### P5 — Agent pool, custom teams, Live tab (owner's review of the app): DONE ([ADR-61](adr/ADR-61-agent-pool-and-custom-teams.md))
| Item | Status | Evidence |
|---|---|---|
| Agents created on their own into the pool (role, own model API/CLI, library skills, MCP); `core_003` makes `agent.team` nullable (table rebuilt; migrations run with foreign keys off and must pass `foreign_key_check`) | DONE | `registry.rs::agents_start_in_the_pool_with_their_own_model`, `core.rs::v1_database_with_data_migrates_to_the_latest`, `migrate.rs::a_table_rebuild_keeps_references_and_broken_keys_are_refused` |
| Roles: template presets + the user's own (ROLE.md, scope, planner, lead hint, tags) | DONE | `registry.rs::roles_are_template_presets_or_the_users_own` |
| Custom teams from pool agents, Lead suggested with reasons (user decides), membership, Lead swap, project assignment, dissolve | DONE | `registry.rs::teams_are_composed_from_pool_agents_and_the_lead_is_only_suggested` |
| System skill library (`<root>/skills`) with per-agent switches; MCP switches; forced removal unbinds | DONE | `registry.rs::library_skills_and_mcp_servers_are_switched_per_agent`, `skills.rs` |
| Providers added/edited/removed by the user (key in the vault), local servers discovered (Ollama, LM Studio) | DONE | `registry.rs::providers_keep_their_keys_in_the_vault`, `adapters.rs::local_servers_are_discovered` |
| CLI parity (`agent create`, `role`, `team compose/...`, `skill library`, `provider remove/local`) | DONE (smoke-tested on a fresh root) | ADR-61 |
| `setup` honours `AGENIUM_HOME` and does not remember it machine-wide | DONE | ADR-61 §8 |
| Desktop: providers & CLIs, skills & MCP library, create agent, compose/manage teams | DONE | ADR-61 Consequences; real app on a test root (pool agents created, suggestion "role meant to lead, tags, skill lead-team") |
| Desktop: Live tab (rooms, pixel office, room log, inspector with thinking / tool-skill-MCP calls / role-based chat), window maximized + F11 | DONE | ADR-60 "Live tab"; watched a real team round on DeepSeek: Lead created and assigned tickets, a researcher worked, calls classified with args/output/ms |
| Finding from that round: a plan item whose role is the Lead's (research-lead) cannot become a ticket (the Lead takes no tickets in team mode); the Lead cancelled it | DONE | ADR-64 §3: plans refuse items for the Lead's own role; research template has a synthesist |
| Language: no user-language setting; Lead/Solo answer in the language the user writes in | DONE | ADR-40 amendment |
| Per-agent MCP keys + Lead tool set_member_mcp_key; projects by folder name or repo; no machine paths in the UI; Live: desks for agents with a task, lounge for free ones, names, plan tab, CLI-like chat with expandable thinking | DONE | ADR-62; real app on the test root |
| Team runs from the agreed spec to the end (no autopilot switch, no "run a round"), stop/resume per project, Lead approves plans of an agreed spec | DONE | ADR-63 |
| Commands run in the sandbox without asking (policy `sandboxed`, default), approvals per project, workers get the requirements they serve, skeptical review, ROLE.md sections + advice, specialist role library | DONE | ADR-64 |
| `propose_role`: the Lead proposes a library or new role, the user approves (agent created, joins the team) or declines | DONE | [ADR-69](adr/ADR-69-propose-role.md); real Lead proposed `api-tester` on the test root |
| Scheduler aging; outside edits to a ticket worktree undone before the next run (patch kept, user told) | DONE | [ADR-70](adr/ADR-70-aging-and-worktree-guard.md) |
| Template wizard (new, from a team, import/export TOML, role editor) | DONE | [ADR-71](adr/ADR-71-template-wizard.md) |
| M4.5: lessons need a cue and a signal, rules/hints by scope, validated contexts, decay and consolidation | DONE | [ADR-72](adr/ADR-72-lessons-decay-consolidation.md) |
| M4.5: `propose_skill_update` overlays (checked, audited, revert/promote); Memory page | DONE | [ADR-73](adr/ADR-73-skill-overlays-memory-page.md) |
| Skill overlays rebase onto an updated base (3-way merge; clean auto-applies, conflict left for the user) | DONE | [ADR-86](adr/ADR-86-overlay-rebase.md) |
| Cross-agent knowledge sharing (GM-6): controlled shared pool of generalized lessons | DONE | [ADR-87](adr/ADR-87-shared-lesson-pool.md) |
| Skill shape + authoring checks (reused by agent self-upgrade) + seeded 12-skill library with references/scripts (agency-agents methodologies, our words) | DONE | [ADR-88](adr/ADR-88-skill-shape-and-library.md) |
| Vendor badge: the AI behind each agent's model shown in the inspector and pixel office | DONE | [ADR-89](adr/ADR-89-vendor-badge.md) |
| Optional Jev skill-quality read (one job / actionable / proof / no stale code) on top of the structural check | DONE | [ADR-90](adr/ADR-90-jev-skill-quality.md) |
| Agent-created skills (`propose_new_skill`): strict gates, contained, offered only with a qualifying lesson, anti-drift flag; skill-writing tools now reachable in work runs | DONE | [ADR-91](adr/ADR-91-agent-created-skills.md) |
| Skills cannot contradict each other: one subject one skill (calibrated), no explicit opposite directive (new/update/import), judge read when on, experience flag | DONE | [ADR-92](adr/ADR-92-skill-contradictions.md) |
| Learning loop on real models: 6 models x 2 passes (DeepSeek, Claude Haiku/Sonnet/Opus, gpt-6-luna, gpt-6.1-sol), 12/12 pass, 0 violations; update robustness fixed | DONE | [learning-loop-eval](learning-loop-eval.md), [ADR-93](adr/ADR-93-skill-update-robustness.md) |
| Shared project guidance from the repo's AGENTS.md (no vendor CLAUDE.md) | DONE | `views.rs::project_guidance`, `team.rs::agents_md_is_the_shared_project_guidance` |
| Model list by backend (API vs each CLI) | DONE | [ADR-74](adr/ADR-74-models-by-backend.md) |
| TypeSafe "Jev" evaluated and wired as an optional judge (off by default) | DONE | [ADR-75](adr/ADR-75-typesafe-jev-judge.md) |
| Judge is fail-open (two modes), disables on key/quota failure, re-enable in Settings; agent export/import on desktop | DONE | [ADR-81](adr/ADR-81-judge-two-modes.md) |
| Quick skill pick: the judge suggests the skill that fits a fresh ticket (a hint, fail-open, off by default) | DONE | [ADR-82](adr/ADR-82-quick-skill-pick.md) |
| Memory gate: the judge skips extraction on routine tickets when confident nothing is worth remembering (fail-open, off by default); [with/without comparison](jev-comparison.md) | DONE | [ADR-85](adr/ADR-85-memory-gate.md) |
| Network posture: containment is the boundary; sandboxed commands may use the network by default (fail-closed); no egress proxy; every agent reads URLs through one audited `fetch` (allowlist, out-of-sandbox) | DONE | [ADR-83](adr/ADR-83-no-egress-proxy.md), [ADR-84](adr/ADR-84-network-posture-and-unified-fetch.md) |
| M5: run-log compaction after 90 days | DONE | [ADR-76](adr/ADR-76-run-log-compaction.md) |
| M5: support bundle (diagnostics without secret values) | DONE | [ADR-77](adr/ADR-77-support-bundle.md) |
| M5: Markdown project report | DONE | [ADR-78](adr/ADR-78-markdown-report.md) |
| M5: pre-merge checks per template, run in the sandbox | DONE | [ADR-79](adr/ADR-79-pre-merge-checks.md) |
| M5: push the merged main on Closed when the project has a remote | DONE | [ADR-80](adr/ADR-80-push-on-close.md) |

## Spike S2 — Windows sandbox (run 2026-10-24, first run ever)
Report: [`spike-reports/S2/REPORT.md`](spike-reports/S2/REPORT.md), evidence in `spike-reports/S2/evidence/`. Codex CLI
0.163 `codex sandbox` with the MXC backend and an Agenium permission profile: 0 escapes in the attack matrix (writes,
reads, junction/symlink/UNC/long paths, TCP/UDP/DNS/localhost, persistence), 20/20 clean tree kills, valid work runs
(git read, python, node with `--preserve-symlinks-main`, cargo), build overhead +1.7 % (+0.27 s per command), no admin.
Codex default permissions (reads the whole disk) and the `unelevated` backend (network and detached processes escape)
fail. Caveats: Windows build 26300 (Insider) only.
**Implemented (ADR-53):** the owner chose to use Codex's sandbox natively, not through the Codex CLI. `agenium-sandbox`
now has a `WindowsMxc` backend on Microsoft's `mxc-sdk` (MIT) with the probe and policy shape adapted from Codex
(Apache-2.0, see NOTICE): every agent command runs in a container (worktree writable; platform, toolchain, git and
skill folders readable; network denied; clean environment), shell = PowerShell (Git Bash's MSYS runtime cannot start in
the container). All 172 workspace tests pass with commands inside MXC; the suite takes ~575 s instead of ~290 s
(PowerShell start ~0.8 s per command). MCP stdio servers are not containerized yet.

## Real-model verification (2026-10-23, OpenCode Go subscription, `deepseek-v4.1-flash`, effort medium)
- CLI end to end: Vietnamese chat -> English spec/plan/ticket, self-assignment -> work run (write, verify with read/grep;
  the one shell command was denied without an approver) -> submit_report -> self-review -> merge -> close.
- Golden tasks: **5/5 PASS**, 2 runs each, 8-13 steps, 75k-125k tokens, 26-46 s; the only tool errors were
  `command_denied` (the model tried to verify with shell).
- Found and fixed with the real model: the report template's extension format was not in the work order (the model
  needed 2 retries on `coding@1`), and a chat turn that hit exhausted credit left the agent Idle.
- M1.5 end to end (skills + MCP + memory, Vietnamese requests): agent `bea` with skill `sum-report` and MCP server
  `everything` (tools `get-sum`, `echo`). Ticket 1: chat used `mcp__everything__get-sum`; the work run did
  `load_skill` -> MCP sum -> `write_file` -> report (11 steps), then self-review, merge, close; the pipeline extracted
  8 facts (2.4k tokens in). Ticket 2 (same agent): 8 facts recalled into the work order, 5 work steps (no denied
  shell attempts this time: it used the stored lesson); pipeline: 3 extracted -> 2 ADD, 1 NOOP. Golden tasks after
  the OpenCode router change: 5/5 again (only `command_denied` errors).
- Graph memory with the real model (2026-10-24, ticket 3 of the same agent): the agent database written in M1.5
  migrated v1 -> v2 with its facts on open; extraction returned scope, entities and relations: 5 facts -> 2 ADD, 1
  UPDATE, 2 NOOP; 5 entities and 2 edges (evidence = the new facts); 1 relation refused (named an entity the fact did
  not list); the 8 memories shown to the work run were marked `helped` on close (6 work steps).
- Found and fixed with the real model in M1.5: a plan revision after a new spec version was validated against the
  old spec (`revise_plan` now follows the newest approved spec; regression test in `lifecycle.rs`). Adding a plan
  item is a scope change and waits for the user even in Solo mode (by design).
- OpenCode specifics: Zen pay-as-you-go (`/zen/v1`) vs Go subscription (`/zen/go/v1`); Go requires the
  `x-opencode-session` affinity header (sent = conversation cache key); free models are restricted to OpenCode's own
  app (403 FreeTierError); unfunded Zen accounts return 402 (-> QuotaExhausted, verified).

## Not done / not run (honest list)
- Real Anthropic, Gemini, OpenAI, OpenRouter, DeepSeek-direct and local endpoints (only OpenCode Go was run; the
  owner allows DeepSeek models only). The OpenCode Anthropic and Responses wires are contract-tested, not run.
- Graph memory: adjacency cache; memory extraction after chat sessions (only Closed/Reopened tickets trigger it now).
  (The labelled resolution set — 2 152 mentions — and the 112-question multi-hop set are built and run:
  docs/memory-eval.md, ADR-94. Memory reuse ran on DeepSeek, n = 5; not yet on the CLI models.) (Consolidation/forgetting, lessons with cue/decay, skill
  overlays, sharing between agents and agent-created skills are done: ADR-73, 86, 87, 91, 92.) The skill-writing
  tools ran on six real models (docs/learning-loop-eval.md); no model tried to get a duplicate or contradicting skill
  past the ADR-92 gates, so those are verified by tests only.
- MCP: OAuth, the legacy HTTP+SSE transport, protocol revision 2026-07-28 (the client negotiates 2025-06-18; the
  reference server accepted it), MCP resources/prompts.
- ANN sidecar (usearch builds on this machine; not wired), event-log retention archive.
- M4: the Lead "interrupt" (ADR-57: rounds instead). (Pre-merge checks per template: done, ADR-79.)
- UI: Monaco editor / xterm terminal (lightweight viewers instead), mini-map and room editor of the Live office, CI UI
  tests. CLI backend: codex, agy, claude and opencode (free models, ADR-68) driven; devin not drivable (ADR-56).
- Sandbox: the containment matrix against Seatbelt on macOS (Linux bubblewrap: run 2026-10-26 on a real Ubuntu 25.10
  VM, 0 escapes, ADR-95; Windows MXC: ADR-53/S2). An AppArmor profile shipped with a Linux installer (until then,
  Ubuntu 24.04+ hosts run sandboxed commands without network unless the user allows user namespaces, ADR-95).
- Context compaction beyond the checkpoint-and-continue approach.
- cargo-deny, GitHub workflows. **Real Ubuntu 25.10 (VMware VM, 2026-10-26): 296 passed, 0 failed** after the ADR-95
  fixes — the first run found that no sandboxed command could start with the network on. **macOS: first run 2026-10-25** (virtual M4 Pro): build OK, 289 passed / 1 failed /
  2 ignored — the failure was a real path-confinement defect (a worktree reached through `/private/var`), fixed and
  reproduced on Windows (8.3 short names); benchmarks in STORE-SCALE §10. Linux: first run 2026-10-24 on WSL2 Ubuntu 24.04 (kernel
  6.18, cargo 1.98.1): `cargo build --workspace` and `cargo test --workspace` PASS (no sandbox backend yet, so commands
  run after approval as designed). Since then a bubblewrap backend through `mxc-sdk` (ADR-54) is active on that distro;
  the sandbox tests pass inside it. **2026-10-25: the current code's full suite ran on WSL2 Ubuntu (real kernel,
  bubblewrap present): 264 passed, 0 failed (2 local-model tests ignored).** The S3 containment matrix was run in the
  spike (`spike-reports/S3`); running it against the shipping bubblewrap backend, and the macOS backend, are NOT RUN.

## Next steps (proposed order, 2026-10-25)
M4, M4.5 and the self-contained M5 items are done (ADR-64..93). What remains needs an owner decision or real
infrastructure, not mechanical work:
- **egress proxy** — DROPPED ([ADR-83](adr/ADR-83-no-egress-proxy.md), [ADR-84](adr/ADR-84-network-posture-and-unified-fetch.md)):
  containment is the boundary, sandboxed commands may use the network by default (fail-closed), reads go through one
  audited `fetch`; no proxy/allowlist layer.
- **cross-agent knowledge sharing** GM-6 — DONE ([ADR-87](adr/ADR-87-shared-lesson-pool.md)): a controlled shared pool of generalized lessons (promoted only after >=2 distinct contexts, non-project-scoped; recalled below own memory). The base-update 3-way rebase of skill overlays is also DONE ([ADR-86](adr/ADR-86-overlay-rebase.md)).
2. **installer + signed updater** — deferred (core not finished yet); packaging/signing infrastructure per OS.
3. **M2 on Linux/macOS** — needs those machines (sandbox attack suite, macOS Seatbelt backend); graph benchmark rest.
