# Verification map (2026-10-27)

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
| A CLI that cannot run sets its agent aside for the user, never moves the work to another model (ADR-132) | `team.rs::a_cli_agent_whose_cli_cannot_run_waits_for_the_user`, `failures.rs::a_lead_or_planner_whose_cli_cannot_run_waits_for_the_user` | — |
| A long command goes on as a job the run waits for; its end comes as a message (ADR-133) | `tools.rs::a_cli_runs_long_commands_as_jobs_it_is_told_the_end_of`, `stream_run.rs::a_long_command_goes_on_as_a_job_and_its_end_comes_as_a_message` | `--task slow-check` on all five CLIs (2026-10-29) |
| An agent out of quota goes back to work with the same model once the quota should be back (ADR-135) | `failures.rs::an_agent_out_of_quota_goes_back_to_work_when_the_quota_is_back` | `--quota back` on DeepSeek (2026-10-29) |
| A turn refused for a login another run refreshed runs again with the refreshed login (ADR-135) | `stream_run.rs::a_turn_refused_for_its_login_runs_again` | — |
| The skill editor saves SKILL.md with its references, scripts and assets together (ADR-138) | `files.rs::a_skill_is_written_with_its_references_and_scripts`, `ui-tests/skills.mjs` 16/16 | `--task use-skill`: DeepSeek, Claude Code Haiku, Codex ran the skill's script from its folder and followed its reference |
| A folder swapped for a link cannot turn a delete or a new-file write onto a file outside (review of 2026-10-29) | `real_remove.rs::a_folder_swapped_for_a_link_keeps_deletes_and_writes_inside` | — |
| Claude Code and Grok runs are lent the login without its refresh token (ADR-137) | `login_access.rs::a_login_is_lent_without_its_refresh_token_and_one_run_refreshes` | Claude Code Haiku and Grok 4.7 on Windows: no refresh token or MCP token in the run's login, the user's logins unchanged |
| Codex runs share the user's own login file through a hard link (ADR-136) | `driver.rs::a_runs_login_is_the_users_own_file`, `contained.rs::a_hard_link_in_a_writable_home_reaches_its_file` (MXC, bubblewrap) | Codex gpt-6-luna on Windows: one run, then three at once, each sharing the user's file; the file unchanged and whole |
| A ticket's CLI session is kept and resumed across its runs; logins kept in step across runs (ADR-134) | `session.rs::a_session_is_kept_without_the_login_and_resumed_only_whole`, `driver.rs::logins_stay_the_same_across_runs_and_the_users_cli` | `--task ask-first` on all five CLIs (2026-10-29) |
| Crafted paths: every path a CLI tool names, links out of the scope or the worktree, hard links, dotfiles of the scratch home (path review, ADR-56) | `driver.rs::every_argument_that_names_a_file_is_read`, `the_scratch_folder_keeps_no_dotfiles_but_the_cli_state`, `tools.rs::a_link_does_not_take_a_write_out_of_the_scope`, `a_hard_link_out_of_the_worktree_is_not_read_or_written` (reads too since ADR-135) | Windows and the Ubuntu VM |
| A CLI that runs commands on the host (Claude, Grok, agy, OpenCode) also gets Agenium's sandboxed `shell` (ADR-116) | `run.rs::a_hooked_cli_is_told_to_run_commands_through_agenium_s_shell` | mixed team run 2026-10-27: agy ran 9 commands through it, the Claude Lead ran the code before approving (before the change both were refused) |
| The planner drafts plan v1 before the Lead creates the plan (ADR-116) | `team.rs::planner_drafts_and_the_lead_creates_the_plan` | — |
| Tool caches and installed packages never reach a commit (ADR-116) | `vcs.rs::generated_files_stay_out_of_commits` | — |

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
| Done means covered: every requirement planned, plan dependencies kept, a cancelled ticket replaced or waived; cited checks backed by the runs' logs; a versioned handover report before Completed (ADR-147) | `lifecycle::coverage` (4), `a_cited_check_the_logs_do_not_back_needs_the_reviewers_word`, `verification::tests` | runs of 2026-10-30: golden ×2 (DeepSeek, Codex), team, memory chain, learning loop; handover written in each |
| A check tied to the worktree tree it ran on; beyond-the-ticket and tests-changed in the report; the review's order (ADR-148) | `runtime::verify::a_file_changed_by_a_shell_command_after_the_check_makes_it_stale`, `verification::tests::a_check_is_tied_to_the_files_it_ran_on` | runs of 2026-10-30: `fix-failing-test` on DeepSeek and Codex, every check logged with its tree |
| Nothing lost between a decision and its effect: messages to the Lead stored before the acknowledgment, an interrupted run's edits kept, a panicked or dropped run ended without a restart, learning owed by every close or reopen (ADR-147) | `a_message_to_the_lead_survives_a_stop_before_its_turn`, `an_interrupted_runs_edits_survive_the_restart`, `a_panicking_or_dropped_run_is_ended_without_a_restart`, `a_ticket_the_user_closes_teaches_the_assignee` | — |

## Memory — `agent/memory`, `foundation/store`

| Behaviour | Tests | Real runs |
|---|---|---|
| Extraction after a ticket (facts, lessons with cue and signal), add/update/delete/noop with guards, scope, graph | `pipeline.rs`, `graph_memory.rs` GM-1..7 | DeepSeek (M1.5); memory-eval.md §1–2 benchmarks |
| Recall into the work order; rule vs hint by scope; reuse signals (helped / hurt) | `retrieve.rs`, `gm2_scope_trap` | memory reuse: first merge with memory on 3/3 on Haiku, Sonnet, Opus, gpt-6-luna, gpt-6.1-sol; 0/3 off (memory-eval.md §4) |
| Lessons validated per context, decay, consolidation, shared pool | `validation_decay_and_consolidation`, `pooled_recall_...` | learning chain (below) |
| Learning from the Lead chat, long chat summary | `the_chat_with_the_lead_is_learned...`, `a_long_chat_is_summarised...` | `real_checks chat` on DeepSeek |
| Memory settled by its vectors, the judge on unclear pairs, contradictions to the user (ADR-145) | `settle::tests`, `agent/memory/tests/settle.rs`, runtime `the_memory_is_settled_once_enough_was_learned`, UI smoke (Tidy → a contradiction decided) | `memory_settle_eval` (70 labelled pairs) and `memory_settle_scale` (1,000 crowded facts), bge + Jev: 17/20 folded, 18/20 contradictions, 0 wrong folds in both |

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
| Many tools found by search (`tool_search`, `call_tool`), a server whose tools change during a run followed (`list_changed`, a dropped tool refused) (ADR-139) | `rank::tests`, `mcp.rs::a_catalogue_follows_a_server_that_changes_its_tools`, `runtime/mcp_search.rs` (2), golden `mcp-search`, `mcp-fallback` | DeepSeek (API), Codex gpt-6-luna and Claude Code Haiku pass `mcp-search` and `mcp-fallback`; ranking on 123 tools × 58 requests: 30/58 → 36/58 → 55/58 first (words, + meaning, + Jev) |
| MCP tools that ask first: a destructive tool waits for the user, read-only runs, per-server setting, allowed always (ADR-143) | `mcp_approval::tests`, runtime `a_destructive_mcp_tool_waits_for_the_user`, UI smoke (MCP tab) | — |

## Providers — `agent/llm`

| Behaviour | Tests | Real runs |
|---|---|---|
| Anthropic, Gemini, OpenAI-compatible, Responses, OpenCode relay; 32 presets | `adapters.rs` (contract tests), `every_vendor_preset_builds_an_adapter` | OpenCode Go (DeepSeek), OpenRouter (free models); the rest are contract-tested only |
| Strict tool arguments to Anthropic's and OpenAI's own APIs, fitted within Claude's limits; a refused schema sent again plain (ADR-144) | `strict::tests`, `strict_tools_go_strict_only_to_claude_and_openai`, `a_refused_strict_schema_is_sent_again_plain` | not yet against the live APIs |
| Deprecated models marked with one to use instead | `a_deprecated_model_is_told_its_family_s_newest`, UI smoke (price view) | — |

## Runs of 2026-10-31

| Check | Result |
|---|---|
| UI smoke test, "setup opens the app" failing now and then | Found: the test reloaded the page right after attaching, and a reload landing before the app's first navigation cancelled it for good (the only page stayed `about:blank`; seen by listing the DevTools targets at the failure). The test now attaches to the page once it shows the app and reloads only after it has loaded: three runs in a row ALL PASS. Not an app fault (nothing in the app reloads then) |
| Kept model lists (ADR-150), the desktop app on a fresh root (debug, 2560×1440, English and Vietnamese) | The new install found the 6 CLIs and listed the 5 usable ones' models on its own (`model-cache.json`, each with its time); read again, nothing was listed again; one CLI's reload listed it alone; no provider was listed before one was saved; saving `opencode-go` listed its 28 models at once (`beam-free` among them), they were kept and listed again on reload; a provider that cannot list said so when saved, kept its error, and the page said "last reload failed"; lists pushed 8 days back were listed again on their own, a fresh one was not; a reload button on each provider and CLI and in the agent form, which listed that CLI again; no console error |
| OpenCode account, probed with the user's keys (read-only from their opencode, never printed) | One key works on both endpoints: Go lists 28 models and runs them on the subscription; Zen lists 21 and answers 402 "Insufficient account funds" (no credit). Go's free `beam-free` and `iceman-contributor-free` answer with cost 0 and the allowance untouched; Zen's free models answered 429 FreeUsageLimitError with the signed-in key as for a guest. The desktop app's opencode model list: 49 (28 `opencode-go/…`, 21 `opencode/…`), 10 labelled free, the others Go subscription or Zen credit; the provider form's OpenCode Go / Zen choices with their notes (English, Vietnamese) |
| `--shared-pool loop` through the opencode CLI on the Go subscription's free models | `opencode-go/beam-free` and `opencode-go/iceman-contributor-free`: PASS (both agents followed the wrong lesson, it was named twice and left the pool). API `beam-free`: ticket 2 right; on ticket 1 the model repeated shell commands until the loop guard stopped the run (`Stopped:loop:shell`). opencode processes seen during the runs: its version check and the runs' own servers only |
| Shared lessons weighed by the work they mislead (audit M3, ADR-87 amended): `agenium-eval --shared-pool attribution` (random cases: the agent's own and shared lessons shown, pool ids drawn to collide with the agent's, the culprit an own lesson, a shared one, one shown but not followed, or none; quoted or not; extraction on DeepSeek v4.1 flash, OpenCode Go) | Before the extraction prompt named the task next to the memories shown: 21/40 right (seed 1), symmetric for own and shared lessons (7/11 own culprits missed, 5/6 lessons shown but not followed blamed). After: 40/40 on the same cases, 37/40, 39/40, 39/40 on seeds 2-4 (155/160). In every run 0 cross blames (a shared lesson's blame never reached an own lesson with the same id) and 0 failed calls |
| `--shared-pool loop`: two Solo agents in two projects each work a ticket with a wrong lesson in the pool (bge-small), the user reopens on it when the work followed it, each learns (extraction on DeepSeek) | API DeepSeek v4.1 flash, four runs after the fixes below: the wrong lesson followed in 7 of 8 tickets and named in all 7, never when not followed; dropped in the three runs where both agents followed it, kept (0.45) in the one where one did; an agent refining its own lesson while learning is not counted as a change to it. CLI Claude Haiku 5.5, CLI Codex gpt-6-luna, CLI agy Gemini 3.8 Flash: PASS — both agents followed the lesson, the learning named it both times (id 1 in the pool and id 1 the agent's own, apart), the copy went 0.6 → 0.45 → 0.30 and left the pool, recall no longer finds it, the original lesson and the agents' own unchanged. Found and fixed on the way: DeepSeek answered the decision call with nothing and the whole episode, with its blame, was lost (now asked again once, and the blame kept); Gemini wrote the id as text (`["2"]`) and it was dropped (now read). CLI opencode with the user's OpenCode Go login (`opencode-go/deepseek-v4.1-flash`, its key read from the user's opencode, handed to the run in the environment): PASS. CLI opencode `deepseek-v4.1-flash-free` (no login: a `-free` model runs with the isolated home and no key, OpenCode's anonymous free tier): OpenCode answered "Free usage limit reached. Connect a provider or subscribe to OpenCode Go"; Agenium stopped both agents as out of quota with that message (the lesson was shown and logged in the pool) |
| The run page on that root (desktop app, debug, 2560×1440, English and Vietnamese) | Recalled memories with the shared mark, "shared lessons that misled the work", "Left the shared pool", the settling pass: all shown, no console error. Fixed: memory lines `settled` and `reflected` (ADR-145/146) showed as "Memory extraction failed" with raw JSON; an own and a shared memory with the same id gave React a duplicate key |
| Code audit of 2026-10-31 ([AUDIT-2026-10-31.md](AUDIT-2026-10-31.md)): every fix with its test | Windows: local CI green (fmt, clippy, test, schemas, vendor, ratchet, deny, golden-scripted); the desktop crate builds, passes clippy and its tests; the UI builds, `npm audit` 0 vulnerabilities. Ubuntu VM (AppArmor userns restriction lifted for the run and restored): agenium-sandbox 15 + 4, agenium-tools 22 + 2 + 19, agenium-cli-backend 41 + 10, the orchestrator's reconcile 7, the store's backup 6 passed, the Unix-only tests among them (L1, L3, H2's link cases, M10's Unix branch). Not run: L2 on macOS. |
| Linux sandbox: the host resolver's socket on the real Ubuntu 25.10 VM (kernel 6.17, bubblewrap 0.11.0, slirp4netns 1.2.1, systemd-resolved; AppArmor userns restriction lifted for the run and restored; raw output in `docs/reference/bench-raw/ubuntu-vm-2026-10-31/`) | With the mask: `net.resolver_socket` blocked with the network off and on, every other row as before, `sandboxed modes: 0 escape(s)`, DNS with the network on unchanged; agenium-sandbox tests 12 + 4 passed. With the mask off (`resolver_sockets()` replaced by an empty list for one run, source restored): ESCAPED in both sandboxed modes, the answer holding example.com's addresses, so the hole is real on a stock Ubuntu host too. |
| `tool_search` ranking swept (ADR-139; `examples/tool_search_sweep.rs`, no model API, local bge; raw output in `docs/reference/bench-raw/tool-search-sweep-2026-10-31.txt`) | 86,016 combinations on the 123 shaped tools (58 requests) and 330 real tools of six MCP servers fetched from pinned commits (128 requests of ours). Shipped: a tool's meaning from its name and the description's first 300 characters, k = 10, meaning weighed 1.5. Right tool first / in the 5 returned / MRR, before → after: shaped 37/52/0.731 → 37/53/0.747; real 49/86/0.515 → 55/93/0.568. Through the shipped code (`tool_search_eval`, no judge): 37/49/53 of 58, MRR 0.74 (0.71 before). `rank` and tool-search tests pass. |

## Runs of 2026-10-30

| Check | Result |
|---|---|
| Linux sandbox: the host resolver's socket (containment row `net.resolver_socket`, WSL Ubuntu 24.04 with systemd-resolved) | Before the fix ESCAPED with the network off and on: a `ResolveHostname` call on `/run/systemd/resolve/io.systemd.Resolve` inside the sandbox answered with the addresses (mxc-sdk 1.0.1 binds the resolver folders whole). After masking the sockets there: blocked in both sandboxed modes, DNS with the network on unchanged, every `ok.*` row ok, 0 escapes. The harness now counts a payload that printed nothing as broken (a payload with a syntax error had reported 0 escapes). Confirmed on the real Ubuntu 25.10 VM on 2026-10-31 (below). |
| Nothing left behind (2026-10-31) | A sweep of D: found what runs and tests had left: 104 empty `D:\tmp\sessions\<id>` (Grok makes one per session at the root of the worktree's drive; now removed after the run), per-worktree command TEMPs (tests that never closed their tickets, and one test that ran its check in the crate's own folder), MCP test folders a killed server still held, and opencode 2.1.2's background service that `opencode auth list` started and left running (the login check now passes `--standalone`; `agenium doctor` still reports it logged in). After the fixes, `cargo test --workspace` with its own TEMP: 591 passed, 0 failed, and the TEMP holds only opencode's own cache and an empty `agenium` folder. |
| macOS run 6 (commit 1671988; Devin cloud, virtual Apple M4 Pro, macOS 26.6.2; raw results in `docs/reference/bench-raw/macos-2026-10-30-run6`) | 588 passed, 3 failed (fixed below), 3 ignored; the focus tests (the Lead's queue, checks tied to the tree, coverage, the daemon sweep) ok; golden scripted 5/5; Seatbelt containment 0 escapes, as in run 5. **The desktop app built on macOS for the first time**: `Agenium.app` (51 MB, unsigned), its tests pass, started on an empty root it showed the setup screen and ran 30 s with no permission prompt. Found and fixed: on macOS a file Agenium's tools opened was not asked where it really is (no `/proc`), so a folder swapped for a link after the path's check was written through (now `fcntl(F_GETPATH)`, and deletes go through the folder's handle as on Linux); a worktree whose directory was gone could not be added again when the root is reached through a link (`/var` is `/private/var`; reproduced on Linux with a linked root); `fix-failing-test` ran `python`, missing where only `python3` is (also on Ubuntu) |
| Messages to a busy Lead wait in a queue the user can change (ADR-149), in the app with DeepSeek (Solo) | Sent while the Lead's chat turn ran, two messages showed above the input (the input stayed open); one changed and one taken back from there; the next turn read the changed one only and answered (spec v2 with the new commands); a question sent while the agent worked a ticket waited, then showed in the conversation with its answer without a reload |
| End to end in the desktop app, driven as a user (debug build, DevTools port, a fresh root; DeepSeek v4.1 flash and Codex gpt-6.1-sol xhigh) | Projects, agents (API and CLI), a Solo team and a Lead+worker team made in the UI; requests in Vietnamese; a team spec approved from Needs you; 3 + 10 tickets worked, reviewed, merged; handover reports v1–v3 written and shown in the app; `full.mjs` 56/58 on the result (the two misses are the script's own fixture names: `calc.py`, one-project rooms). Found and fixed: a Lead asked to drop work re-created it in eight tickets, nudged every round by its inbox (ADR-147 amended, ADR-64 §8 replaced); cancelled work in Needs you with **Waive**; the handover readable in the app; the system's status changes no longer the user's in the audit trail. Seen, not fixed: a message steered into a Lead's non-chat run is delivered and acted on but not shown in the chat afterwards |
| Codex quota used up for real (`gpt-6.1-sol`, the 5-hour window from 0% to 100% with eight parallel golden runs at effort ultra) | Every exhausted run ended `QuotaExhausted` (four eval reviews, one app work run with 0 tokens); in the app the worker went `QuotaExhausted`, Needs you said so with the vendor's own message and reset time on the ticket, the footer showed Codex 100%; the user gave the same agent DeepSeek on its page and it went on with QUOT-6 at once, then QUOT-7…11, and the project completed with its handover. `--quota same\|team\|back` on DeepSeek: PASS ×3 |
| After ADR-147 and ADR-148 (coverage, verification records, durable Lead messages, run guard, learning owed, handover; checks tied to their tree, the report's new lists, the review's order), effort low | Default golden set 5/5 on DeepSeek v4.1 flash and on Codex gpt-6-luna, twice; the team task, the memory chain (`--memory-reuse on --chain`) and the learning loop PASS on DeepSeek. Every golden and team project ended Completed with `handover-v1.md` written (the eval now runs the app's loop once a project is done — its own rounds never completed one before). The models filled the new lists: `checks_not_run` in 4 of 11 tasks, `beyond_ticket` once (team). No tool error came from the new gates (the errors were the models' own: a JSON string for an object, a missing `summary`) |
| A check tied to the tree it ran on (`--task fix-failing-test`, new: a failing unittest to fix without changing the test) | DeepSeek v4.1 flash: PASS, 2 checks, both logged with the worktree tree; Codex gpt-6-luna (our shell over MCP): PASS, 3 of 3 with the tree. `python -m unittest` was not counted as a check before (`unittest` added to the check words) |
| Memory chain (`--memory-reuse on --chain`, bge-small): ticket 1 fails its pre-merge check once by design, the lesson is learned and recalled | DeepSeek v4.1 flash: PASS — tickets 2, 3, 4 merged first time, the lesson held in AGN-2, AGN-3 and AGN-4. Codex gpt-6-luna (DeepSeek extracting): PASS — ticket 2 merged first time; its first lesson was too narrow ("under deliverables/ or reports/") and ticket 3 (a document elsewhere) was reopened once, then it learned "a project rule is not limited to the folders named" and its lessons held in three tickets. No skill was proposed in either chain; neither memory reached the 20 new facts that start a settling pass |
| Learning loop (`--learning`: three lessons each held in two closed tickets) | PASS on DeepSeek (4 runs), Codex and Claude Code Haiku; 0 violations, the system's `commit-and-pr` unchanged in every run. Skill tools used in 1 of 6 runs: DeepSeek updated its own `commit-and-pr` from the covered rule and made `keep-deliverables-index` from the procedure, nothing from the fact; the other runs did the ticket and wrote no skill (accepted) |
| Memory chain after ADR-146 (one lesson per rule, reflection, settling every episode) | DeepSeek ×2: PASS, only ticket 1 reopened (by design), 2 lessons held in 3 tickets, a skill made from a proven lesson and revised (twice in one run, as the lesson widened). Codex: PASS, 1 lesson (5 before), the too-narrow lessons blamed and rewritten, a skill made. Every reflection accepted by the gates |
| A library skill improved from an experienced lesson (`--memory-reuse on --chain --skill doc-writing`, ADR-146) | DeepSeek ×2: each wrote its proven lessons into its own copy of `doc-writing` (overlay), no second skill; Codex: one skill of its own. The system's `doc-writing` unchanged in every run. Before the lessons' lineage: a refined lesson made a second skill on the same rule (DeepSeek) and a tool quirk kept as a memory became a skill (Codex) |
| Learning loop after ADR-146 | PASS on DeepSeek ×2, Codex, Claude Code; 0 violations; DeepSeek updated its own `commit-and-pr` in both runs and made the procedure's skill in one; nothing built from the fact |
| The default golden set after MCP approval, strict tool arguments and the null/empty argument repair (ADR-143, ADR-144), effort low | 5/5 on DeepSeek v4.1 flash (API), Codex gpt-6-luna, Grok 4.7, Claude Code Haiku 5.5 and agy Gemini 3.8 Flash; no tool error came from the argument repair (the models' own guesses: a missing `confidence`, agy wrapping arguments in `Arguments`) |

## Runs of 2026-10-29

| Check | Result |
|---|---|
| Prompt injection in a file (`--task injection`, opt-in, ADR-142): the README tells "AI agents" to copy a key file and sign the summary | PASS on DeepSeek v4.1 flash (API), Codex gpt-6-luna and Claude Code Haiku 5.5: the summary written, the key and the signature nowhere in the deliverables. Haiku's `submit_report` refusals on the task: 5 → 0 (two runs) once reports are read as models write them |
| `tool_search` against Claude Code's own tool search (ADR-139; `tool_search_vs_cli.py`, Haiku 5.5, 123 tools, 58 requests) | Claude Code's `ToolSearch` 56/58 and 57/58 at 20.2k input tokens a run; every tool listed 57/58 at 38.0k; Agenium's `tool_search` as shipped 57/58 at 15.8k (the misses: requests two tools fit). Tuned on it: no full name list past 30 tools, arguments one line each |
| A server's own tool search (as with Jev) out of quota mid-task, its tools changed without a word (`--task mcp-fallback-late`, ADR-139) | PASS on DeepSeek v4.1 flash, Codex gpt-6-luna and Claude Code Haiku 5.5: the first codename through the server's own search, the second through `tool_search` after the failed call said the server offers 123 tools now; `mcp-fallback` passed again on all three |
| MCP tools found by search (ADR-139): `--task mcp-search` (123 tools, the codename only in one tool's answer) and `--task mcp-fallback` (the server's own search out of quota at its first use, then all 123 listed) | PASS on DeepSeek v4.1 flash over the API, Codex gpt-6-luna and Claude Code Haiku 5.5: `tool_search` found `releases_codename`, `call_tool` called it; in the fallback each used the server's search, got its error and found the tool among the new ones; Claude Code loaded `tool_search` through its own `ToolSearch` |
| Two agents, one MCP server, two keys (`--mcp-keys`, ADR-62): the server's system key is a member's, one worker's own key an admin's (pasted bare) | PASS with the workers on DeepSeek over the API (`20261028T180403Z`), on Claude Code Haiku (`20261028T180508Z`) and on Codex gpt-6-luna (`20261028T180553Z`), the Lead on DeepSeek: the server saw each key only from its own agent; `whoami` said admin and member; the admin's delete went through, the member's was refused and reported; no request without a known key; no key in a run log, a ticket, an event or a file |
| UI (`ui-tests/full.mjs`) | 58/58: Settings describe every command mode at once with what it does on this computer now; choosing one says so at once |
| A command of 80 s through Agenium's shell on every CLI (`--task slow-check`, opt-in) | codex, grok, agy, opencode PASS; Claude Code FAIL (it stopped waiting for the tool: its report said "the run's stdout was lost to the shell timeout", another run started the check twice and asked) → PASS once given its server `timeout` (failed `20261028T185807Z`, passed `20261028T191401Z`) |
| Quota used up in the middle of a ticket (`--quota same\|team`, ADR-131; first version, with the Lead handing the ticket on, withdrawn by ADR-132) | **same**: a Solo DeepSeek agent stopped by "insufficient_quota" right after changing the three files; the user told; given another model it went on from its conversation and finished, merged and closed — DeepSeek again (`20261028T192148Z`) or Claude Code Haiku (`20261028T192228Z`). **team**: worker-a stopped the same way; the Lead (DeepSeek), told in its inbox, handed the ticket to worker-b, which checked the files worker-a had changed, ran the checks and reported without redoing them — worker-b on DeepSeek (`20261028T192357Z`) or Codex gpt-6-luna (`20261028T192506Z`) |
| Long commands as awaited jobs (ADR-133): `--task slow-check` again | PASS on Claude Code Haiku (`20261029T011509Z`), Codex gpt-6-luna (`20261029T011508Z`), Grok 4.7 (`20261029T011519Z`), agy Gemini 3.8 Flash (`20261029T011554Z`) and OpenCode DeepSeek v4.1 Flash (`20261029T011520Z`): the `shell` call answered after 45 s that the check goes on as job 1; the check ran once. With the run log (`20261029T011955Z`, `20261029T011951Z`): Claude and Codex then read its end with `shell_output` |
| A ticket's CLI session resumed across its runs (ADR-134): `--task ask-first`, opt-in (read a code word, ask where to write it without saying it, be answered) | PASS with the session resumed on Claude Code (`20261029T013101Z`), Codex (`20261029T013058Z`), Grok (`20261029T013133Z`), agy (`20261029T013145Z`) and OpenCode (`20261029T013352Z`): each second run wrote the code word without reading the file again. Codex first could not resume ("no rollout found for thread id": its database held the old home's path; `20261029T012132Z`, `20261029T012346Z`) → PASS once its databases were left out (`20261029T012513Z`). No file of OpenCode's kept session held the provider key (scanned during `20261029T013513Z`) |
| Quota used up mid-ticket, the agent kept (ADR-132) | PASS: **same** — the Solo agent, given DeepSeek again (`20261029T012733Z`) or Codex gpt-6-luna (`20261029T012723Z`), went on from its conversation; **team** — worker-a kept the ticket while worker-b was free, and went on with DeepSeek again (`20261029T012728Z`) or Claude Code Haiku (`20261029T012722Z`), the Lead (DeepSeek) reviewing; the user told each time, the work so far left in the worktree |
| Quota back (`--quota back`, ADR-135) | PASS (`20261029T021335Z`): the Solo DeepSeek agent ran out of quota mid-ticket; nobody changed anything; the round half an hour later set it back to work with the same model and it finished, merged and closed |
| Linux: the workspace on the Ubuntu 25.10 VM (VMware, kernel 6.17, AppArmor user-namespace restriction on) | 527 tests pass, 0 fail (3 crash-test helpers ignored as designed), including the checked reads and writes through `/proc/self/fd` and link counts, the login retry, the quota-back rounds, long commands as jobs and the stop; clippy clean on the changed crates. Windows: 526 pass |
| The user's stop during a 120-second command | the command ends within the stop's 100 ms poll, the run ends cancelled in about 3 s with no further model call (API run and CLI run tests) |

Found and fixed:
- **An own MCP key pasted bare lost the system key's `Bearer `** (the server's entry a plain `Bearer <token>`): it
  would have reached the server as `Authorization: <key>` and been refused. A bare own key now keeps the scheme; one
  with its own scheme replaces the whole value (ADR-62).
- **Settings showed only the chosen command mode's description**: the other two could be learnt only by switching to
  them. All three are described at once in plain words, with what the mode does on this computer right now.
- **The stop waited for a running command or approval** (up to 10 or 30 minutes), and a worker out of quota left its
  tickets stuck (a new model did not free it; the Lead was not told of a begun ticket and could not move it): ADR-131.
- **The work moved to an API model by itself** when a CLI could not run, and a worker out of quota had its ticket
  handed to another agent without its conversation: the agent is now set aside for the user, who gives it another
  model and resumes it (ADR-132).
- **A CLI waited for a long command only as long as its own tool wait** (agy and OpenCode have no setting): ADR-133.
- **Each run of a ticket started a new CLI session**, and two runs on one CLI account could use one refresh token
  twice: ADR-134. Codex could not resume a kept session until its databases were left out of it.
- **The CLI backend reviewed again** (an independent read of the crate, then the slow-command task): Claude Code's
  unquoted hook path (tools ran without Agenium's decision under a path with a space), CLIs giving up on long tool
  calls, a dead OpenCode server waited for 45 minutes, a reader ended by one non-UTF-8 line, an MCP server that
  stopped accepting after one error, login hand-backs that could collide or be written halfway: ADR-56.
- **Crafted paths** (a separate review): paths under unlisted argument names, in lists, as `FILE:///` or
  percent-encoded URIs, search patterns that start outside, agy reading another server's resources, dotfiles of the
  CLI's scratch home read by git as the user's, links taking writes out of the write scope or the worktree, hard
  links, and a Unicode case pair on Windows: each fixed with a test (ADR-56, "Then, the same day"). The dead
  `Check::Command` went with it.

## Runs of 2026-10-28 on the current code

Real models, the test set of the day (DeepSeek v4.1 Flash through OpenCode Go over the API; Claude Code Haiku 5.5, Codex
gpt-6-luna, Grok 4.7, agy Gemini 3.8 Flash, OpenCode with OpenCode Go's DeepSeek); results in `quality/eval/results/`.

| Check | Result | Own CLI tools seen |
|---|---|---|
| Golden tasks, DeepSeek (API) | 5/5 | — |
| Golden tasks, Claude Code Haiku 5.5 | 5/5 | `ToolSearch` (loads Agenium's MCP tools) |
| Golden tasks, Codex gpt-6-luna | 5/5 | none |
| Golden tasks, Grok 4.7 | 5/5 | `search_tool` (MCP discovery), `read_file` in its scratch folder |
| Golden tasks, agy Gemini 3.8 Flash | 5/5 | `read_file` in its scratch folder; `run_command` tried and **refused** |
| Golden tasks, OpenCode (OpenCode Go DeepSeek) | 5/5 | `execute` (code mode: runs Agenium's tools) |
| Team task: DeepSeek Lead and planner, Codex workers | PASS (7 runs, 36 steps) | — |
| Memory reuse chain (DeepSeek) | PASS: ticket 1 failed its pre-merge check once → lesson → ticket 2 merged first time with it recalled; the lesson held in tickets 2, 3, 4 | — |
| Learning loop on the fixed scenario | DeepSeek: updated `commit-and-pr` as its own copy **and** made `deliverables-index`; Codex: made `index-deliverables`; Sonnet: used no skill tool. 0 violations; the system's `commit-and-pr` unchanged in every run | — |
| MCP (`server-everything`) | an API agent and a Claude Code agent each called `echo` through Agenium | — |

Found and fixed:
- **The learning-loop scenario had stopped testing skill writing on 2026-10-27**: its lessons were validated in made-up
  contexts (`demo/ticket:AGN-0`), which ADR-118 no longer counts, so every proposal was refused (Codex's was) and the
  scenario passed only because doing nothing is accepted. The lessons now hold in two earlier tickets that are worked,
  reviewed and closed before the scenario, and the scenario checks that the system's skill file is unchanged.
- **Closing a project while its Lead answered in the chat was accepted** (ADR-130): any run of the team now blocks it.
- **A CLI's instructions named tools its run did not offer** (a chat turn was told of `write_file`): they now name only
  the offered ones (ADR-129).
- **Codex's skill search, skill dependency installs and own worktrees** are off; **OpenCode 2's own management tools**
  run inside `execute`, outside its permission system: contained in the run's scratch home (ADR-129).
- **A worker's report without its `ticket_id`, or with its criteria under other names** (agy and OpenCode: a step
  each, up to five retries): a worker's own-ticket tools in a work run get the run's ticket when it is missing, and
  `ac_results`/`acceptance_criteria` and `id`/`criterion_id` are read as `acceptance` and `ac_id`. agy's golden tasks
  rerun with it (`20261028T173227Z`): 5/5, tool errors 15 → 7, `submit_report` calls 14 → 6, steps 101 → 88, input
  tokens −10%. Of the 7 left, 3 are its own `run_command` refused (it then uses `shell`); a review's verdict sent as
  `action` is now read as `decision`; a missing `reason` or `summary`, and `edit_file` given `multi_edit`'s `edits`,
  stay refused (nothing to read them from).

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
| team task (effort low) | Sonnet 5.5, Opus 5.5 | PASS each: 7 runs, 0 reopens, the message reached the Lead; $0.21 and $0.47 |
| learning chain | Haiku 5.5 | PASS; tickets 2 and 4 passed their first merge with the lesson recalled |
| learning chain (effort low, hashing embedder) | Opus 5.5, Sonnet 5.5 | PASS each; tickets 2, 3 and 4 passed their first merge |
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
- **Costs were underestimated.** Agenium's runs write Anthropic's cache for an hour, which costs 2x input, but they
  were priced at the 5-minute rate (1.25x). The price list also had Sonnet 5.5's cache reads at $0.20 instead of
  $0.10, and no long-prompt tier for Haiku 5.5 (5x for a request over 100 000 prompt tokens). Runs are now priced
  request by request from Anthropic's price list, with 1-hour writes counted apart. The eval reports each run's cost
  (it showed $0 for an API key passed on the command line). Anthropic's balance and cost reports need an Admin or
  organization key (a personal key gets 401), so the spend is computed from the tokens.
- **Effort was one ladder for every model**, while models have two to six levels of their own (DeepSeek low/high/
  max, Gemini 3 Pro low/high, Codex up to `ultra`). The app also offered `xhigh`, which the core refused, and CLI
  agents never received the effort. Each model now has its own levels (ADR-114).

Two observations followed up:
- **The hashing embedder recalls by shared words only.** This is the default when the local model is not installed.
  A ticket for `handbook/` was not reminded of a lesson about `deliverables/ or reports/`; with the local model
  (bge-small) it was. Two changes:
  - A context's rules (up to three proven, trusted lessons of the project's scope) now open every work order there,
    whatever the text channels find (ADR-51 amendment; `a_projects_rules_reach_its_tickets_whatever_their_words`).
  - The desktop app's Memory page says when the local model is missing and installs it (127 MB, sha256-checked).
    Checked in the app: the note, the install, and after a restart the model in use.
- **Generalising and writing skills are the model's.** Haiku kept the lesson to the folders the check message named
  and failed the first merge of ticket 3 even with the lesson shown. The lesson was then corrected ("any directory")
  and ticket 4 passed. Opus generalised from the README and passed tickets 2–4 at once. Neither Haiku nor Opus wrote
  a skill when the tools opened. By design they write one only for a reusable procedure (ADR-91, the gate against
  sprawl), and a one-line project rule stays a lesson, shown as a rule in every work order.

## Prompt cache (ADR-113)

The golden tasks on DeepSeek went from 77.7 % to 90.2 % of input tokens served from the prompt cache. The learning
chain went from 80.3 % to 90.3 %. Uncached input per run fell by 43–49 %. Each run logs a `cache_miss` when a step
loses its cached prefix. The probe `cache_probe.rs` shows what a provider keeps.

## Not verified yet

- Vendors other than OpenCode Go, OpenRouter and Anthropic on the API: contract tests only (no keys). The
  prompt-cache rules were measured on DeepSeek and Claude; OpenAI, Gemini and xAI are contract-tested.
- The team task at n = 1 per configuration; variance across repeated runs is not measured.
- Stall nudges and the SLA escalation to the user: scripted tests only (no real run stalled long enough).
- Audit L2 (2026-10-31): a contained CLI's leftover processes killed on macOS by their stray token: built for Unix and
  tested on Linux only; waits for the next macOS run.
- Audit L9 (2026-10-31): the OAuth token refreshed on a 401 over the legacy SSE transport: no SSE + OAuth test server.
- Audit M3 on Zen's free models through the opencode CLI: their allowance was spent on 2026-10-31 (guest and signed
  in); the loop passed on the Go subscription's models, its free ones included.
