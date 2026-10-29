# ADR index

ADR-1 … ADR-38 are summarised in [`../design/DECISIONS.md`](../design/DECISIONS.md) (highest priority on conflict).
ADR-39 … ADR-51 were written during the M0 trial; they are decisions and results, kept as written.
ADR-52 records this fresh implementation.

| ADR | Topic |
|---|---|
| [39](ADR-39-windows-deferred.md) | First release Linux + macOS; Windows deferred |
| [40](ADR-40-english-below-lead.md) | Any language with the user, English from the Lead down |
| [41](ADR-41-sandbox-vendoring-and-layering.md) | Codex sandbox vendoring and wrapper crate |
| [42](ADR-42-linux-sandbox-bwrap-landlock.md) | Linux sandbox: bubblewrap / Landlock >= 3, fail-closed |
| [43](ADR-43-macos-seatbelt-design.md) | macOS Seatbelt profile v2 |
| [44](ADR-44-vcs-git-cli.md) | VCS through the git CLI, journal-first |
| [45](ADR-45-journal-and-event-sink-in-core.md) | Event sink in core, broadcast bus |
| [46](ADR-46-backup-and-migration-design.md) | Backups and migrations |
| [47](ADR-47-embedder-trait.md) | Embedder trait |
| [48](ADR-48-xtask-and-ipc-types.md) | xtask, IPC types |
| [49](ADR-49-sqlite-primary-store.md) | SQLite primary store |
| [50](ADR-50-vector-index-scale-and-retention.md) | Vector index, 100k ANN threshold, retention |
| [51](ADR-51-graph-memory-on-sqlite.md) | Graph memory on SQLite (Proposed) |
| [52](ADR-52-fresh-implementation.md) | Fresh implementation; Windows verification; deviations |
| [53](ADR-53-windows-sandbox-mxc.md) | Windows sandbox backend: MXC process containers via `mxc-sdk` (amends ADR-39) |
| [54](ADR-54-linux-sandbox-bubblewrap-mxc-sdk.md) | Linux sandbox backend: bubblewrap via `mxc-sdk` (S3 matrix not run yet) |
| [55](ADR-55-macos-sandbox-seatbelt-mxc-sdk.md) | macOS sandbox backend: Seatbelt via `mxc-sdk` (experimental; containment on a Mac 2026-10-26: 0 escapes on the fifth run (c35ee29, 312 passed, 0 failed) after the unix-socket narrowing and the stray-process sweep) |
| [56](ADR-56-cli-backend-headless.md) | CliBackend: headless JSON stream in an Agenium-owned per-run home; agy driven |
| [57](ADR-57-team-mode-lead-inbox.md) | Team mode: Lead inbox (derived view), inbox runs per round, SLA, escalation flag |
| [58](ADR-58-team-templates-and-write-scope.md) | Team templates in TOML (4 built-in + user overrides); write scope = ticket scope AND role paths, checked at write and at submit |
| [59](ADR-59-planner-role.md) | Planner role: drafts plan v1 (submit_plan_draft), the Lead reviews and creates the plan; no subagents in team mode |
| [60](ADR-60-desktop-app.md) | Desktop app: Tauri 2 in its own workspace, thin IPC over the orchestrator, UI adapted from the AGENIUM-UX mockup |
| [61](ADR-61-agent-pool-and-custom-teams.md) | Agent pool (agents before teams), custom teams with a suggested Lead, system skill library with per-agent switches, providers from the UI |
| [62](ADR-62-mcp-keys-projects-live-chat.md) | Per-agent MCP keys (system key by default, Lead tool set_member_mcp_key, shown in the app, masked in logs), projects by folder name or repository, Live tab as a chat with a plan tab |
| [63](ADR-63-team-runs-until-done.md) | From the agreed spec to the end the team runs on its own (run state, team loop, stop/resume per project); the Lead approves plans of an agreed spec |
| [64](ADR-64-sandboxed-commands-roles-review.md) | Commands run in the sandbox without asking; no plan items for the Lead's role; workers get their requirements; skeptical review; ROLE.md sections and a specialist role library |
| [65](ADR-65-agent-names-audit-only.md) | Agent names are role names without the team, unique ignoring case; the Activity page folds into the audit trail |
| [66](ADR-66-model-prices-catalogue.md) | Model prices from the models.dev catalogue (cached, refreshed weekly, earlier runs priced again); FREE models marked; OpenCode free models need the OpenCode client |
| [67](ADR-67-vendor-usage-footer.md) | Vendor subscription usage in the footer (Claude from its run events, Codex app-server RPC, OpenCode Go endpoint; Antigravity listed without numbers); only honest channels |
| [68](ADR-68-opencode-cli-free-models.md) | opencode CLI backend for OpenCode free models: `opencode serve` driven over HTTP, every permission through Agenium's hook, fully isolated home |
| [69](ADR-69-propose-role.md) | The Lead proposes roles (library or new); the user approves (an agent joins the team) or declines; Needs you, CLI, Lead project state |
| [70](ADR-70-aging-and-worktree-guard.md) | Scheduler aging (a level per `aging_hours` waited); changes made to a worktree outside Agenium are undone before the next run, saved as a patch and reported |
| [71](ADR-71-template-wizard.md) | Template wizard: new, from a team, import/export TOML, edit roles (kind, count, tags, scope, model, ROLE.md), validated on save |
| [72](ADR-72-lessons-decay-consolidation.md) | Lessons need a cue and a signal; rules where the scope matches, hints elsewhere; validated contexts; 90-day decay and consolidation |
| [73](ADR-73-skill-overlays-memory-page.md) | Skill overlays from lessons held in two contexts (checked, copy-on-write, audited; revert/promote by the user); Memory page |
| [74](ADR-74-models-by-backend.md) | Model list follows the backend: API lists what the API serves (OpenCode free models only via the opencode CLI), each CLI lists what it runs |
| [75](ADR-75-typesafe-jev-judge.md) | TypeSafe "Jev" as an optional judge (typed calibrated questions, not a chat model): opt-in review evidence read; images deferred |
| [76](ADR-76-run-log-compaction.md) | Run logs older than 90 days are compacted (diagnostics dropped, refused protocol calls kept) on the maintenance tick |
| [77](ADR-77-support-bundle.md) | Support bundle: shareable diagnostics folder (versions, config without secret values, store counts, masked logs); CLI and Settings |
| [78](ADR-78-markdown-report.md) | Markdown project report (goals, plan progress, ticket table) from stored state; CLI `report`, Overview button |
| [79](ADR-79-pre-merge-checks.md) | Pre-merge check per template, run in the sandbox (desktop); fail-closed without a backend; wizard field |
| [80](ADR-80-push-on-close.md) | Push the merged main to origin on Closed when the project has a remote (best-effort, non-blocking); local-only otherwise |
| [81](ADR-81-judge-two-modes.md) | The judge is fail-open: Agenium works without it; a key/quota failure disables it and asks the user to re-enable; agent export/import on the desktop |
| [82](ADR-82-quick-skill-pick.md) | The judge can pick the skill that fits a ticket before a fresh run (a hint, fail-open, off by default); the worker still decides |
| [83](ADR-83-no-egress-proxy.md) | No egress proxy: the codex-derived sandbox is the containment boundary (network on/off), agents run YOLO; no proxy/allowlist layer |
| [84](ADR-84-network-posture-and-unified-fetch.md) | Network posture: containment is the boundary (sandboxed commands may use the network by default, fail-closed); every agent reads URLs through one audited `fetch` (allowlist, out-of-sandbox) |
| [85](ADR-85-memory-gate.md) | The judge can skip memory extraction on routine tickets (a confident "nothing worth remembering"; fail-open, off by default) |
| [86](ADR-86-overlay-rebase.md) | Skill overlays rebase onto an updated base (3-way merge; reverse-apply reconstructs the old base, clean merge auto-applies, conflict is left for the user) |
| [87](ADR-87-shared-lesson-pool.md) | Cross-agent knowledge sharing (GM-6): a controlled shared pool of generalized lessons (promoted only after holding in >=2 distinct contexts and non-project-scoped; recalled below own memory, verify-first) |
| [88](ADR-88-skill-shape-and-library.md) | Skill shape (Job/When/Steps/Tools/Proof) + advisory authoring checks reused by agent self-upgrade; a seeded 12-skill library (SKILL.md + references + scripts) from agency-agents methodologies (our own words) |
| [89](ADR-89-vendor-badge.md) | Show the AI vendor behind each agent's model as a small badge (Live inspector + pixel office); our own monogram, detected from provider/model |
| [90](ADR-90-jev-skill-quality.md) | Optional fail-open Jev read of a skill's quality (one job, actionable steps, proof demands evidence, no stale code) on top of the always-on structural check |
| [91](ADR-91-agent-created-skills.md) | Agents may create a new skill only through strict gates (trusted lesson held in two contexts, no duplicate, full shape, safe), contained to the agent, offered only with a qualifying lesson, flagged when its lesson stops holding |
| [92](ADR-92-skill-contradictions.md) | Skills cannot contradict each other: one subject one skill (calibrated words+meaning), no explicit opposite directive (references included; updates only for introduced conflicts), judge read when on, experience flag |
| [93](ADR-93-skill-update-robustness.md) | Skill updates an agent can land: whole body (`content`) or diff, the real patch error, a unique-content fallback; found by the 6-model learning-loop runs |
| [95](ADR-95-linux-sandbox-network.md) | Sandboxed commands get the network only as a private, outbound-only network (no inbound, no host loopback) — or none, with the reason in their output; CLI agents are refused; containment harness on the shipping backend (Ubuntu: 0 escapes) |
| [94](ADR-94-memory-measured.md) | Memory measured on labelled sets (2 152 mentions, 112 questions): keys keep `+`/`#`, "same name" only when the difference is noise, entity vectors from the name, a verdict miscount trusts none, zero vectors never indexed, the graph channel ordered by relevance, cut-off extractions keep their finished facts |
| [96](ADR-96-memory-reuse-cli-agents.md) | Memory reuse on CLI agents: Agenium's own word (ticket, comments, protocol results) is never stored as untrusted; the work order names Windows PowerShell 5.1's byte-order mark; the eval records what the check rejected and what the agent held |
| [97](ADR-97-mcp-oauth.md) | OAuth for remote MCP servers: the user signs in once per server (`agenium mcp login`, Sign in in the app), discovery + dynamic registration + PKCE + loopback redirect + `resource`-bound tokens in the vault, refresh before expiry and after a 401, agents told to ask the user; a test server that is both authorization and MCP server |
| [98](ADR-98-learning-from-chat.md) | The Lead learns from the user's chat once it has been quiet `[memory] chat_idle_minutes` (default 20): the scheduler's round, each part once (watermark), facts and preferences only, the user's own words never untrusted, a failed extraction waits a quiet period |
| [99](ADR-99-dependency-policy-cargo-deny.md) | cargo-deny in local CI for both workspaces: permissive licenses (+ MPL-2.0), any RustSec vulnerability fails unless an exception says why it does not apply, crates.io only; `zip` moved to 9 (RUSTSEC-2026-0367/8/9), candle's read-only zip 8 excepted |
| [100](ADR-100-in-run-context-compaction.md) | In-run context: from 60 % of the window old tool results are cleared (no model call, last four kept, only when it frees a tenth), from 85 % the run's conversation is replaced by the model's notes and the run goes on (twice), then the checkpoint as before |
| [101](ADR-101-history-sent-as-providers-accept-it.md) | Every request's history is made acceptable: thinking only to the model that wrote it, unanswered calls closed, stray results dropped, empty blocks gone, safe tool-call ids; calls in an answer cut by max_tokens are not run (from pi's bug history) |
| [102](ADR-102-edit-matching.md) | `edit_file` keeps CRLF and the BOM, matches typographic quotes, dashes and trailing spaces loosely, and replaces only the matched bytes |
| [103](ADR-103-context-overflow-recovery.md) | A provider's context overflow is recognised per vendor and recovered once (clear old results or compact); compaction notes must be whole and fit; new tool results count before the next request |
| [104](ADR-104-messaging-a-running-lead.md) | The user's message to a running Lead/Solo reaches it before its next step (never a worker: workers get work through tickets and the plan); late messages open the next turn |
| [105](ADR-105-lenient-arguments-parallel-reads.md) | Harmless tool-argument shape mistakes are repaired against the schema; read-only calls of one turn run together, results in order |
| [106](ADR-106-long-lead-chat-summary.md) | A long Lead chat (over ~50k tokens) is sent as a summary of its older part plus ~15k tokens of recent messages; the stored chat stays whole; the summary is updated, never cut at a tool result |
| [107](ADR-107-vendor-presets.md) | 29 known vendors as presets (adapter, endpoint, key variable): `agenium provider presets`, `provider add <id>`, "More vendors…" in the app; every endpoint probed |
| [108](ADR-108-grok-cli-backend.md) | Grok Build CLI as a backend: prompt file per turn + resume, Claude-style stream, `GROK_HOME` isolation (own config, no leader/memory/compat scanning), always-approve + a fail-closed `PreToolUse` hook; 5/5 golden tasks on grok-4.7 |
| [109](ADR-109-mcp-legacy-sse-resources-prompts.md) | MCP: legacy HTTP+SSE when a server refuses Streamable HTTP; resources and prompts as read-only tools; SSE text decoded across chunks; checked against the official reference server |
| [110](ADR-110-messaging-a-running-cli-lead.md) | A running CLI Lead is messaged: the queue reaches it with the next Agenium tool result, a Claude hook's added context, or its next turn; each message carries the run's tag so files cannot pose as the user |
| [111](ADR-111-cloud-vendor-presets.md) | Azure OpenAI (v1 API, `api-key` header), Amazon Bedrock (bedrock-mantle, API key) and Cloudflare Workers AI as presets with a `<placeholder>` the user fills in; Vertex and Copilot wait for refreshed tokens |
| [112](ADR-112-monaco-and-xterm.md) | Monaco side-by-side diff per changed file and an xterm.js view of a run's shell session, bundled and loaded on demand (pinned, MIT) |
| [113](ADR-113-prompt-cache.md) | Prompt cache across vendors: nothing per-run in the system (tools follow it), one cache route per agent, notes on the cached prefix (`tool_choice: none`), OpenRouter breakpoints, cache misses logged; DeepSeek golden tasks 77.7 % → 90.2 % cached |
| [114](ADR-114-effort-levels.md) | Each model's own effort levels, by the vendor's names: reported by the vendor (Anthropic, OpenRouter, Codex, Antigravity) or from a per-model table (OpenAI, Gemini, xAI, DeepSeek); the agent form offers only those; CLI agents get the level too |
| [115](ADR-115-custom-endpoint-requests.md) | What a user may add to a custom endpoint's requests (headers, secret ones in the vault; per-model settings and effort levels; extra body fields; the effort field) and what stays Agenium's: its own fields, plain names, bounded values, no placeholders, no HTTP-owned or multi-line headers, checked on save and on every request |
| [116](ADR-116-team-run-findings.md) | Mixed real-model team run: sandboxed shell for hooked CLIs, generated files out of commits, the planner drafts first |
| [117](ADR-117-cli-yolo-agenium-tools-unsandboxed.md) | Vendor CLIs work through Agenium's tools (their own shell is off, nothing asks the user); a machine without a sandbox may run unsandboxed by the user's choice (Settings), Codex included |
| [118](ADR-118-learning-evidence-from-hermit.md) | From hermit: a lesson counts a context only when it was in play (shown as a rule and found by the work); skills and the pool count only contexts naming closed tickets; learning claims are reported with paired bootstrap intervals |
| [119](ADR-119-cli-runs-see-no-user-configuration.md) | A vendor CLI run by Agenium sees none of the user's configuration: per-run home and HOME for every CLI, Claude's claude.ai connectors and skills off; checked by an isolation probe |
| [120](ADR-120-installs-decisions-overlap.md) | Installing on the user's machine asks the user under every policy; the user's answers reach later tickets; tickets naming the same file run in order; a merge nudges overlapping work to rebase |
| [121](ADR-121-preview-browser.md) | A browser in the app: tabs per project, pick elements (selector, HTML, styles, box, component/file) and send them with notes to the Lead; external sites as references; pages cannot reach Agenium |
| [122](ADR-122-user-answers-the-lead.md) | In a team the user answers the Lead, who answers on the ticket; projects coloured by state; a team shows its current and past projects; Node works in the Windows sandbox (listable parents) |
| [123](ADR-123-skill-tags-and-pentest-skills.md) | Skills carry domain tags (pentest, web, coding…), shown on the agent; a pentest team gains app/firmware and reverser roles and five pentest/reverse skills; cross-surface findings link via handoff/aggregate/suggest_link |
| [124](ADR-124-in-app-skill-authoring.md) | Author skills in the app with Monaco: edit SKILL.md and its references/scripts (tiers 2 and 3), Evaluate before Save (shape lint + Jev quality), validated and contained writes |
| [125](ADR-125-ann-sidecar-wired.md) | ADR-50's ANN sidecar wired: tombstones instead of usearch `remove` (slot reuse rewires the graph), compaction at 10 % churn, exact read-your-writes catch-up, journal/identity/crc checks with rebuild, threshold reached on the write that crosses it |
| [126](ADR-126-jev-client-and-done-check.md) | Jev client: 5 s deadline, retries for 429/503/529, off only on a rejected key or exhausted quota, one reused client, malformed answers ignored; a report that says done while nothing checked the work after its last change goes back once (`done_check`, off by default) |
| [127](ADR-127-judge-modules-shadow-cache-trim.md) | The judge split into one file per use; shadow mode (observe only), billed usage kept, an answer cache with the review's questions prefetched at submit, and a trimmer for long shell/grep results (off by default) |
| [128](ADR-128-find-code-and-judge-keys.md) | `find_code`: find code by what it does (judge-rated folders → files → units, 10/10 on this repo); named judge keys take turns, a key out of quota is set aside until the user re-enables it (told by name), a failing key rests 30–60 min; every judge use on by default, fail-open |
| [129](ADR-129-cli-agenium-tools-only-background-shell.md) | Vendor CLIs work only through Agenium's tools (own shells off, own file tools only in their scratch folder); background commands (`shell_output`, `shell_stop`) for every agent; a Solo agent's subagents labelled |
| [130](ADR-130-closing-and-reopening-projects.md) | Only the user closes a done project: its team goes free, whole; reopening is the user's step and offers the team that worked on it, now or once it is free; the team starting and a project done are told, the Lead's chat leads to the work |
| [131](ADR-131-stop-reaches-waits-and-quota-handover.md) | The user's stop reaches a running command and a wait for approval at once; a new model frees an agent set aside for its quota or login (its Lead handover withdrawn by ADR-132) |
| [132](ADR-132-no-automatic-fallback-resume-the-agent.md) | Agenium never moves an agent's work to another model: a CLI that cannot run or a quota used up sets the agent aside, the ticket stays with it, the user gives it another model and resumes it (a Resume button on its card, its page and Needs you); the CLI fallback setting only draws CLI agents' lessons |
| [133](ADR-133-long-commands-as-awaited-jobs.md) | In a vendor CLI's run a `shell` command still going after 45 s goes on as a job the run waits for; its end comes to the agent as a message, and the report waits for it |
| [134](ADR-134-cli-sessions-kept-per-ticket-and-shared-logins.md) | A ticket's vendor-CLI session is kept between its runs and resumed by every CLI; another vendor gets the conversation; logins kept in step across runs on one account |
| [135](ADR-135-quota-back-login-retry-hard-link-reads.md) | An agent out of quota goes back to work with the same model once the quota should be back (at the vendor's reset, or tried from 30 minutes on); a turn refused for a login another run refreshed runs again; no read through a hard link or a swapped folder; a closed ticket's kept CLI sessions end |
| [136](ADR-136-codex-shares-the-users-login-file.md) | Codex runs share the user's own login file (a hard link in the per-run home, which sits on the login's file system): one refresh token for every session, as on the user's machine; the copy stays where no link can be made and for the other CLIs |
| [137](ADR-137-logins-lent-without-refresh-tokens.md) | Claude Code and Grok runs are lent the login without its refresh token while the access token outlives the run; the lent copy follows the user's file; one run at a time refreshes near the expiry |
| [138](ADR-138-skill-editor-files-saved-together.md) | The skill editor writes SKILL.md with its references, scripts and assets in one validated step: fields for name, description and tags, files added in place, uploaded or removed, unsaved changes guarded |
| [139](ADR-139-mcp-tools-found-by-search.md) | MCP tools found by search past 20 tools or 10k tokens (`tool_search`: words + meaning + the judge, 55/58 first on 123 tools; as right as Claude Code's own search with 22 % fewer tokens; `call_tool`), the run's tool list unchanged; a server whose tools change during a run (`list_changed`, a server's own search out of quota) is followed |
| [140](ADR-140-refusals-full-context-output-limit.md) | A refusal says why (the vendor's category and explanation, on the ticket) and its declined answer is not kept, so a resume starts clean; a full context window stops an answer like the output limit; a run's output limit is the model's own (up to 64k) |
| [141](ADR-141-skill-descriptions-say-when.md) | A skill's description says what it does and when to use it (third person, "Use when …"): advised on import, save and Evaluate; the 13 built-ins that did not are rewritten; the skill hint reads descriptions whole |
| [142](ADR-142-fenced-content-step-budget-lenient-report.md) | External content (pages, MCP answers) in a fence with a random id it cannot close, naming its source; the agent told when a tenth of its steps is left; `submit_report` read as models write it (aliases, paths, confidence words); an opt-in prompt-injection golden task (passed by DeepSeek, Codex, Claude Code) |
