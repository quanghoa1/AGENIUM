# PROGRESS — Agenium (fresh implementation)

Last full verification: 2026-10-23 (Asia/Saigon) on Windows 11 Pro (10 cores, 32 GB), Rust 1.99.1
`x86_64-pc-windows-gnu`, git 2.45. Command: `.\scripts\ci-local.ps1` -> fmt PASS, clippy `-D warnings` PASS,
`cargo test --workspace` PASS (**156 tests** incl. the local-model tests with `AGENIUM_TEST_MODELS` set, 1 helper
ignored), `xtask schemas --check` PASS, scripted golden tasks **5/5** PASS. Decisions and deviations of this implementation: [`adr/ADR-52-fresh-implementation.md`](adr/ADR-52-fresh-implementation.md).
The UI is out of scope for now; the headless CLI `agenium` is the front-end.

## Milestones

### M0 — scaffolding (backend): DONE
| Item | Status | Evidence |
|---|---|---|
| M0-1 workspace, LICENSE/NOTICE, local CI | DONE | `Cargo.toml`, `scripts/ci-local.{ps1,sh}`; cargo-deny not installed (not run) |
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
| M1-2 retry / Offline / QuotaExhausted | DONE | retry unit tests, 529/429/drop/402 contract tests, runtime test |
| M1-3 prompt caching order, effort mapping, prices | DONE | breakpoints asserted in contract tests; built-in Anthropic prices + `pricing.toml` |
| M1-4 work tools | DONE | `agenium-tools/tests/tools.rs` |
| M1-5 protocol tools (24, schemas generated), 3 invalid calls stop, nudge | DONE | core + backend loop tests, `schemas/tools/` |
| M1-6 loop, `max_steps`, loop detection | DONE | `agenium-backend/tests/loop.rs` |
| M1-7 approval mode before the sandbox (YOLO fail-closed) | DONE | `RunGate`, CLI terminal approver, denied-command runtime test |
| M1-8 chat console / agent view | CLI only (UI later) | `agenium chat`, live text + tool calls on the terminal |
| M1-9 tokens + estimated cost per run/ticket | DONE | runtime test (usage on ticket), price table test |
| M1-10 eval: 5 golden tasks + runner + report | DONE | scripted 5/5; **real model 5/5** (DeepSeek v4.1 Flash via OpenCode Go, `eval/results/`) |
| M1-11 memory hook + `fact_usage` | DONE (hashing embedder stand-in) | `agenium-memory` test |
| **Exit**: Solo agent: ticket -> worktree -> report -> self-review -> merge -> close | **MET** with a scripted model, and end-to-end through the CLI against a fake OpenAI-compatible HTTP server | `orchestrator/tests/runtime.rs`; CLI smoke (see below) |

CLI end-to-end smoke (2026-10-23): `setup -> project create -> provider add (openai_compat, local fake server) -> agent solo
-> chat "Hãy thêm file chào hỏi" -> run` produced spec v1, plan v1, AGN-1, a work run, a review run, merge into `main`
(`AGN-1: Write the hello file`) and Closed.

### M1.5 — Agent registry: DONE except graph memory (M15-7..11) and the UI forms (M15-6)
| Item | Status | Evidence |
|---|---|---|
| M15-1 agent profile, model switch keeps memory/MCP/skills | DONE | `runtime.rs::vendor_switch_keeps_memory_skills_and_conversation` |
| M15-2 local embedder + memory pipeline + hybrid retrieval | DONE | `bge-small-en-v1.5` via candle (pure Rust; ONNX Runtime has no GNU build); `agenium-memory/tests/{local_model,pipeline}.rs`; trigger test `runtime.rs::closing_a_ticket_teaches_the_assignee`; real model (below) |
| M15-3 McpHub stdio + Streamable HTTP, credential binding | DONE (no OAuth, no legacy HTTP+SSE) | `agenium-mcp/tests/mcp.rs` (test server binary + HTTP stub), `runtime.rs::mcp_tools_reach_the_agent_with_its_credential`, `mcp_settings.rs`; manual: `@modelcontextprotocol/server-everything` 2.0.0 |
| M15-4 SkillRegistry, `load_skill`, skills index | DONE (overlay updates = M4.5) | `agenium-skills/tests/skills.rs`, `runtime.rs::skills_are_indexed_loaded_and_readable` |
| M15-5 export/import JSONL, delete agent | DONE | `orchestrator/tests/agent_io.rs` (byte-identical memory round trip, tamper/secret refusal, delete rules), core_002 migration test |
| M15-6 Settings forms (MCP, skill) | CLI equivalents only (UI later) | `agenium mcp add/import/test`, `agenium skill import/validate/show` |
| M15-7..11 graph memory (ADR-51) | NOT STARTED | next |

Embedding model measured here (release build, `crates/agenium-memory/tests/local_model.rs`, own 60-fact fixture
`tests/data/retrieval.json` because the S6 pair set is not in the pack): load 0.09 s; EN recall@1 0.733, **recall@5
0.967** (S6 pass >= 0.85); Vietnamese query -> English fact recall@5 0.30 (S6 measured 0.167: memory stays English,
ADR-40); **46 facts/s single, 77 facts/s batched** (S6 pass >= 20; fastembed/ONNX in S6 was ~5x faster), query
embedding + ranking ~22 ms. Debug builds are ~2x slower.

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
- Found and fixed with the real model in M1.5: a plan revision after a new spec version was validated against the
  old spec (`revise_plan` now follows the newest approved spec; regression test in `lifecycle.rs`). Adding a plan
  item is a scope change and waits for the user even in Solo mode (by design).
- OpenCode specifics: Zen pay-as-you-go (`/zen/v1`) vs Go subscription (`/zen/go/v1`); Go requires the
  `x-opencode-session` affinity header (sent = conversation cache key); free models are restricted to OpenCode's own
  app (403 FreeTierError); unfunded Zen accounts return 402 (-> QuotaExhausted, verified).

## Not done / not run (honest list)
- Real Anthropic, Gemini, OpenAI, OpenRouter, DeepSeek-direct and local endpoints (only OpenCode Go was run). OpenCode
  models that are Responses-only (gpt-*, grok-*) or Anthropic-wire (claude-*, minimax-*, qwen*) are not routed yet.
- Graph memory (ADR-51, M15-7..11), lessons with scope/decay and skill overlay updates (M4.5), memory extraction
  after chat sessions (only Closed/Reopened tickets trigger it now).
- MCP: OAuth, the legacy HTTP+SSE transport, protocol revision 2026-07-28 (the client negotiates 2025-06-18; the
  reference server accepted it), MCP resources/prompts.
- ANN sidecar (usearch builds on this machine; not wired), event-log retention archive.
- Team templates, inbox/SLA/watchdog, Kanban/UI, CLI backend (M2.5–M4; `agy`, `devin`, `codex` are installed here
  for the CLI backend tests).
- Sandbox backends: Linux bubblewrap/Landlock, macOS Seatbelt (M2). Windows has none in v1 (ADR-39).
- Context compaction beyond the checkpoint-and-continue approach.
- cargo-deny, GitHub workflows, Linux/macOS runs of this codebase.

## Next steps (proposed order)
1. M1.5 rest: graph memory core (ADR-51: agent_003 migration, entity/edge store, resolution, graph channel in hybrid
   retrieval, benchmark).
2. M3 completion: Discovery flow with user approvals in the CLI, Kanban-equivalent CLI views, audit export.
3. M4: team templates, Lead inbox + SLA, watchdog/stall-nudge, handoffs in runs.
4. M2 on Linux/macOS machines (sandbox backends + attack suite) — needs those machines.
