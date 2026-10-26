# Verification map (2026-10-26)

What each part of Agenium does, where it is implemented, what tests hold it, and what ran on real models. "Real" means
a real model or vendor CLI, not the scripted provider. The last section lists what has not been verified.

## Agent loop (API backend) — `agent/loop`

| Behaviour | Tests | Real runs |
|---|---|---|
| Tool loop ends only through a protocol tool (`submit_report`, `ask_question`, ...) | `loop.rs::completes_on_submit_and_carries_call_ids` | golden tasks 5/5 on DeepSeek; team task |
| Nudge: a text-only answer gets a nudge, the run stops after `nudge_limit` (NoProgress) | `text_without_tools_is_nudged_then_stops` | nudges counted in the team task (`discipline.counts.nudge`) |
| Loop detection (same call 3 times), invalid calls in a row, max steps → checkpoint | `identical_call_three_times_...`, `three_invalid_calls_...`, `max_steps_takes_a_checkpoint_without_tools` | — |
| Retry: transient errors back off (base/cap/attempts/total), `retry-after` respected up to a cap, quota → QuotaExhausted, wordless 400 on a huge request → overflow | `adapters.rs` retry tests, `provider_errors_map_to_agent_states`, `a_wordless_400_is_an_overflow_only_for_a_huge_request` | OpenRouter 429 → Offline; OpenCode 402 → QuotaExhausted; real wordless 400 overflow (ADR-103) |
| Context: old results cleared at 60 %, compaction at 85 % (twice, then checkpoint), overflow recovery | `old_tool_results_are_cleared...`, `a_full_context_is_summarised...`, `after_two_compactions...`, `an_overflow_...` | two compactions in one run on OpenRouter (ADR-100/103) |
| Steering: the user's message reaches the running Lead before the next request | `messages_sent_during_the_run_reach_the_next_request` | `real_checks steer` on DeepSeek and OpenRouter |
| Lenient arguments, read-only calls in parallel | `read_only_calls_run_together_and_arguments_are_repaired` | `real_checks` |
| Every nudge, delivered message, compaction and refused CLI call is in the run's log (`control`), every retry (`retry`) | `stream_run.rs` (CLI), team task counts | team task |

## CLI backend — `agent/cli-backend`

| Behaviour | Tests | Real runs |
|---|---|---|
| Per-run home, YOLO inside Agenium, permission hook decides every tool call; MCP server for protocol tools | `stream_run.rs::agenium_decides_every_tool_call...` | golden tasks: codex, agy, claude, opencode, grok (results/) |
| Mandatory report: text-only turns nudged, then NoProgress; loop and step limits | `text_only_turns_are_nudged_then_stopped`, `repeated_step_is_a_loop`, `step_limit_stops_with_a_checkpoint` | — |
| Messaging a running CLI Lead (ADR-110) | `the_user_messages_a_running_cli_lead` | Claude Haiku, Grok, Codex (`cli_steer`), runtime on Claude and Grok (`cli_lead_steer`) |
| API fallback when the CLI cannot run | `runtime.rs::cli_agent_falls_back_to_the_api_when_configured` | — |

## Orchestration: tickets as the only channel for work — `orchestration/orchestrator`

| Behaviour | Tests | Real runs |
|---|---|---|
| Lifecycle: spec → plan → tickets → work → report → review → merge → close; reopen with per-AC feedback; escalation after 3 reopens; dependencies unlock on approve + merge | `lifecycle.rs` (9 tests) | solo end to end on DeepSeek; team task |
| Roles and ownership: only the assignee works on a ticket, only the Lead plans and assigns, workers never chat | `team.rs::roles_and_ownership_are_enforced`, `runtime.rs::the_user_messages_a_running_lead_and_never_a_worker` | team task `discipline` |
| Evidence on the ticket: the report has one result per acceptance criterion with evidence; the review approves or reopens with feedback per AC; questions, answers, permissions, handoffs, system notices are ticket comments | `lifecycle.rs`, `team.rs` | team task `discipline` (report per AC, approving review before merge) |
| Questions: blocking questions park the run and resume on the answer; SLA → the user; Lead inbox order | `blocking_question_parks_and_answer_resumes`, `lead_inbox_order_sla_and_escalation` | team task (worker questions answered by the Lead) |
| Stall nudge: a ticket whose runs keep stopping gets more runs with a System nudge, then goes to the user | `quota_exhaustion_parks_the_agent_and_repeated_failures_escalate` | — |
| Write scope, worktrees, outside edits undone, merge conflicts reopen to the same worker | `team_from_template_and_write_scope`, `outside_edits_...`, `merge_conflict_*` | — |
| Planner drafts, Lead plans; Lead proposes roles; templates | `planner_*`, `the_lead_proposes_a_role...` | team task (planner drafted, Lead planned) |

## Memory — `agent/memory`, `foundation/store`

| Behaviour | Tests | Real runs |
|---|---|---|
| Extraction after a ticket (facts, lessons with cue and signal), add/update/delete/noop with guards, scope, graph | `pipeline.rs`, `graph_memory.rs` GM-1..7 | DeepSeek (M1.5); memory-eval.md §1–2 benchmarks |
| Recall into the work order; rule vs hint by scope; reuse signals (helped / hurt) | `retrieve.rs`, `gm2_scope_trap` | memory reuse: first merge with memory on 3/3 on Haiku, Sonnet, Opus, gpt-6-luna, gpt-6.1-sol; 0/3 off (memory-eval.md §4) |
| Lessons validated per context, decay, consolidation, shared pool | `validation_decay_and_consolidation`, `pooled_recall_...` | learning chain (below) |
| Learning from the Lead chat, long chat summary | `the_chat_with_the_lead_is_learned...`, `a_long_chat_is_summarised...` | `real_checks chat` on DeepSeek |

## Skills — `agent/skills`, `orchestration/orchestrator/src/skill_*`

| Behaviour | Tests | Real runs |
|---|---|---|
| Library, per-agent switches, role skills, load on demand | `skills_are_indexed_loaded_and_readable`, `registry.rs` | M1.5 run used `load_skill` |
| Skill update overlays from a lesson held in 2 contexts (checked, audited, revert/promote, rebase) | `a_validated_lesson_updates_the_agents_overlay_of_a_skill`, ADR-86 tests | learning loop 12/12 on 6 models (seeded lessons) |
| New skills only through every gate; no contradictions; drift flag | `an_agent_creates_a_skill_only_through_every_gate`, `skills_cannot_contradict_each_other`, `a_created_skill_is_flagged...` | learning loop |

## MCP — `agent/mcp`

| Behaviour | Tests | Real runs |
|---|---|---|
| stdio, Streamable HTTP, legacy SSE, OAuth, resources and prompts as tools, per-agent keys | `mcp.rs`, `legacy_sse.rs`, OAuth tests | official `server-everything` (16 tools, 7 resources, 4 prompts); M1.5 run used an MCP tool |

## Providers — `agent/llm`

| Behaviour | Tests | Real runs |
|---|---|---|
| Anthropic, Gemini, OpenAI-compatible, Responses, OpenCode relay; 32 presets | `adapters.rs` (contract tests), `every_vendor_preset_builds_an_adapter` | OpenCode Go (DeepSeek), OpenRouter (free models); the rest are contract-tested only |

## Runs of 2026-10-26 on the current code

**Team task** (`agenium-eval --team`; coding template: Lead, planner, backend, frontend; the harness plays the user and
writes to the team while it works). The protocol is graded on every run (`discipline` in the result file):

| Team | Result | Runs / steps | Protocol | Counts |
|---|---|---|---|---|
| all on DeepSeek v4.1 Flash (API) | PASS (`results/team/20261026T104420Z-…`) | 7 / 38, 0 reopens, 4.7 min | every work run on a ticket assigned to its agent; chat and inbox runs only the Lead's; both tickets closed with the worker's report (evidence per AC) and an approving review before the merge | message delivered to the running Lead (it answered with the tag); 0 retries, 0 nudges |
| Lead and planner on DeepSeek, workers on Claude Haiku (CLI), commands approved | PASS (`…-workers-claude-haiku.json`) | 7 / 35, 0 reopens, 3.4 min | same | 1 nudge (a CLI turn ended without a protocol tool; the worker recovered), message delivered to the Lead |

Two earlier runs that day did not pass, for reasons in the harness or the setup, not in the protocol:
- The first API run had all its work merged and no protocol fault. The Lead cancelled two superseded tickets, and the
  harness then wanted every ticket Closed. It now accepts a ticket the Lead cancelled with a reason, and records each
  ticket with its close reason.
- In the first mixed run, nobody approved commands. A hooked CLI's shell commands run on the host, so they need
  approval. The Haiku workers reported honestly: Partial or NotMet, with what they could not run. The Lead reopened
  twice, verified the work itself in its sandbox, and approved. The harness now accepts a criterion the approving
  review vouches for.

**Learning chain** (`--memory-reuse on --chain`, DeepSeek, `results/learning-chain/20261026T102757Z-…`): ticket 1 failed
its pre-merge check once (a rule no one told the agent) → the lesson was extracted → tickets 2, 3 and 4, in folders
ticket 1 never touched, passed their first merge with the lesson recalled → the lesson held in more contexts → the
skill-writing tools opened → in ticket 3 the agent proposed a new skill, refused by the contradiction gate → in ticket 4
the agent created the skill `docs-team-document-with-owner-line`. Two defects found and fixed:
- **A lesson counted the ticket it was learned on as a validating context.** It now needs fresh contexts, so the tools
  open one ticket later.
- **The contradiction gate refused a sound skill.** "Make sure … starts with the Owner line, so the check passes" was
  read as the opposite of "do not mark a check passed you did not test". The gate now drops purpose and reason
  clauses, and the shared words must cover a third of both clauses (regression test
  `a_shared_reason_is_not_an_opposite_directive`).

Also from these runs:
- Grep explains that patterns match UTF-8 text when a model sends byte patterns (`(?-u)ï»¿`).
- Nudges, delivered messages, compactions and refused CLI calls are now in each run's log (`control`), next to retries.

**Claude through Anthropic's API** (2026-10-26, a $5 test key; `results/claude-api/`). Every check passed:

| Check | Model | Result |
|---|---|---|
| `real_checks`: compaction, steering, overflow wording, MCP behind OAuth, BOM/CRLF edit, chat learning, dangling tool call | Haiku 5.5 | 7/7 PASS |
| golden tasks | Haiku 5.5 (high), Sonnet 5.5 (medium), Opus 5.5 (medium) | 5/5 PASS each |
| team task | Haiku 5.5 | PASS (after the review rule below) |
| learning chain | Haiku 5.5 | PASS; tickets 2 and 4 passed their first merge with the lesson recalled |
| CLI agents at effort low (ADR-114) | Claude Code `haiku`, Codex `gpt-6-luna`, Antigravity `gemini-3.8-flash` | `hello-file` PASS |
| the agent form offers each model's own effort levels (ADR-114) | Codex, Claude Code, Antigravity, Grok, DeepSeek API | 7/7 in the desktop app |

These runs found five defects, all fixed:
- **The Lead approved a criterion the worker had reported Partial**, without a word on it. An approval now needs
  `ac_feedback` saying how the reviewer verified each criterion the report left Partial or NotMet; otherwise the
  engine refuses it and the Lead verifies or reopens (`approving_a_partial_criterion_needs_the_reviewers_word`).
- **A lesson became a plain fact.** A clean ticket that refined a lesson yields only facts, so its update turned the
  lesson into a fact and reset what it had earned. An update of a lesson now keeps its kind, its cue, its trust and
  the contexts it held in (`an_update_of_a_lesson_keeps_what_it_earned`).
- **A project file rewrote a proven lesson.** It read "nothing enforces this", which made the lesson untrusted, and
  retired its twin. The next ticket was not reminded and failed its merge. External content (web, MCP, files) now
  neither rewrites nor retires trusted memory; it is stored beside it as untrusted.
- **Cached tokens were billed twice when earlier runs were priced again** (after a price download). The run's
  input count includes its cached input; only the rest is billed at the input price.
- **Effort was one ladder for every model**, while models have two to six levels of their own (DeepSeek low/high/
  max, Gemini 3 Pro low/high, Codex up to `ultra`). The app also offered `xhigh`, which the core refused, and CLI
  agents never received the effort. Each model now has its own levels (ADR-114).

Also observed, not changed:
- The hashing embedder (no local model installed) recalls by shared words only. A ticket for `handbook/` was not
  reminded of a lesson about `deliverables/ or reports/`. With the local model (bge-small) it was.
- Haiku did not generalise the lesson to folders the check message did not name (DeepSeek did), and it did not write
  a skill at ticket 4 although the tools were open.

## Prompt cache (ADR-113)

The golden tasks on DeepSeek went from 77.7 % to 90.2 % of input tokens served from the prompt cache. The learning
chain went from 80.3 % to 90.3 %. Uncached input per run fell by 43–49 %. Each run logs a `cache_miss` when a step
loses its cached prefix. The probe `cache_probe.rs` shows what a provider keeps.

## Not verified yet

- Vendors other than OpenCode Go, OpenRouter and Anthropic on the API: contract tests only (no keys). The
  prompt-cache rules were measured on DeepSeek and Claude; OpenAI, Gemini and xAI are contract-tested.
- The team task at n = 1 per configuration; variance across repeated runs is not measured.
- Stall nudges and the SLA escalation to the user: scripted tests only (no real run stalled long enough).
- Skill update overlays from an experienced lesson (the chain created a new skill; the update path ran in the learning
  loop with seeded lessons).
