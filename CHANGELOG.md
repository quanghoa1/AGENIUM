# Changelog

All notable changes to Agenium. Dates are Asia/Saigon. Decisions behind each change are in [docs/adr](docs/adr/README.md).

## 0.1.0 — first release (unreleased)

The first public build: the desktop app for Windows and Linux (Ubuntu), and the `agenium` command.

### Teams and orchestration
- **A smaller model keeps the work going** (ADR-155): an agent moved from a 1M-token model to a 256K one (or any
  smaller window) has the conversation it resumes folded into notes, piece by piece by the new model itself, before
  its first request, the work order and the recent steps kept as they are; the stored conversation becomes the folded
  one. Before, a resumed history of long writing failed on the provider's refusal at every nudge (a history of tool
  results recovered by clearing them, as it still does first, at no model call). Measured with OpenCode Go's free
  models (beam-free 1M to iceman 262K): the writing-heavy ticket went from failed to done (100/100 codes, twice); the
  tool-heavy one stayed done with 11 % less uncached input. A model's calls run together into one
  (`{..}{..}`) are taken apart instead of failing as invalid JSON. Windows come from the models.dev
  catalogue when the provider does not say; a CLI session resumes only on its own model; the Lead's chat history
  budget follows its model's window and is summarised by the Lead's own model, CLI included.
- **Memory in every run** (ADR-155): a chat turn, the Lead's inbox, a review and the planner's draft recall what the
  agent learned (the preferences the Lead learned from the chat among them), shown on the run's page; a worker sees
  the plan's outline beside its ticket. A checkpoint that left no real note resumes the conversation instead.
- Code audit of 2026-10-31 ([docs/AUDIT-2026-10-31.md](docs/AUDIT-2026-10-31.md)): a ticket reopened or cancelled
  while its merge ran its checks is no longer merged (main moves only in the transaction that reads the ticket again);
  a rebase interrupted by a crash no longer blocks the merge queue; one git failure at start-up no longer keeps the app
  from opening; a message the Lead took in a chat turn that died is queued again; closing a ticket from its review run
  keeps the worktree until the run ends; a report sent again returns what it returned the first time.
- macOS run 6 (2026-10-30): the desktop app builds and starts on macOS. Fixed from the run: Agenium's file tools now
  ask macOS where an opened file really is (`F_GETPATH`), as they already did on Windows and Linux, so a folder swapped
  for a link after the path's check is not written through, and deletes go through the folder's handle; a worktree
  whose directory is gone is added again also when the root is reached through a link; the `fix-failing-test` golden
  task uses `python3` where there is no `python`.
- Messages to a busy Lead wait in a queue (ADR-149): shown above the chat input, each can be changed or taken back
  until the Lead's next chat turn reads them all; they then show in the conversation with the answer (a message once
  steered into a review or inbox run was acted on but never shown). Answers routed to the Lead go the same way.
- Found end to end in the app (ADR-147 amended): a plan item whose tickets were all cancelled no longer goes back to the
  Lead every round (it re-created work the user had dropped); it waits for the user in **Needs you** (Cancelled work,
  **Waive**), and a new ticket of the item carries the cancelled work on by itself. The project card shows the
  **Handover report**; a project completing by itself is the system's change in the audit trail, not the user's.
- A check is tied to the files it ran on (ADR-148): each check a run makes is logged with the worktree's tree, so a
  file changed afterwards by a shell command, a formatter or a generator makes it stale, not only an edit tool. The
  worker's report lists what it changed beyond the ticket and the tests it changed, skipped or removed (shown on the
  ticket page and in the handover); the review goes in order: the ask, undisclosed scope, weakened tests, what is missing.
  `python -m unittest` counts as a check. The eval completes a done project as the app does and records each task's
  checks, report lists and handover; a new opt-in golden task, `fix-failing-test`.
- Done means covered, verified and handed over (ADR-147): a plan must trace every requirement; a ticket follows its
  plan item's dependencies; a cancelled ticket's work is carried on by a replacement (`replaced_by`, or a ticket that
  `follows` it) or waived by the user (**Waive** on the ticket page, `agenium ticket waive`) before its item is done;
  a reviewer approving a criterion whose cited check the runs' logs do not back says how it verified it; a project
  completes only once its handover report is written (`<root>/reports/<project>/handover-vN.md`): coverage per
  requirement, results per criterion with what checked them, reviews (a Solo agent's own review said so), commits,
  checks not run, risks, follow-ups and usage.
- Nothing is lost between a decision and its effect (ADR-147): a message to the Lead (and the answer to a worker's
  question routed through it) is stored before it is acknowledged and reaches the Lead after a restart; an interrupted
  run's edits are kept as its work instead of being undone; a run that panics or is dropped is ended at once and its
  ticket goes on without a restart.
- An agent is told when a tenth of its run's steps is left, so it wraps up instead of being cut off; a work report is
  read the way models write it (`results` for `acceptance`, a path for an artifact, "high" for confidence) instead of
  being refused field after field (ADR-142).
- Solo agent, or a Lead with workers and an optional planner; tickets are the only channel for work, every state
  change is a typed, validated tool call recorded in an event log (crash-safe, resyncs every view).
- Spec → plan → tickets with acceptance criteria → work in a git worktree per ticket → review per criterion → merge
  through one queue. Reopens carry per-criterion feedback; repeated failures go to the user.
- Ticket dependencies (a ticket waits for what it needs to be merged), write scopes per role, pre-merge checks,
  optional push on close, aging and stall detection, Lead inbox, messaging a running Lead.
- A ticket stays inside its plan item's write scope: a path the item never included is refused (the error names the
  item), and a ticket that names no scope takes the item's — a Lead had created a README ticket for a notes-only item.
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
- The opencode CLI knows a model newer than its built-in list at once: each run used to start with an empty cache and
  fetch the list itself, and a prompt sent before that fetch landed got "Model unavailable" (about half the runs on
  OpenCode Go's `iceman-contributor-free`). The weekly price refresh now keeps the models.dev catalogue whole in the
  root, and each opencode run reads its copy.
- An opencode turn that ended with no text event (about one in ten on `iceman`) no longer completes empty: its answer is
  read back from the session; when the model gave none, it is asked once more, and the turn fails if it still gives
  none.
- Model lists are kept (ADR-150): the agent CLIs and their models are listed on a new install, a provider's models
  when it is saved, then when the user reloads one (a button on each provider and CLI, and in the agent form's model picker) or on
  their own once a week; each says when it was listed and whether the last reload failed. The app no longer starts
  every CLI again every five minutes. The OpenCode Go API lists its own free models (`beam-free`).
- OpenCode Go and OpenCode Zen are two choices when adding a provider, each saying how it bills (the same account key
  works on both; Go uses the subscription, Zen the credit); Zen's "insufficient funds" points to Go. The opencode
  CLI offers the models of the user's OpenCode account (its Go subscription and Zen, each labelled free, Go
  subscription or Zen credit) and runs a free model with the user's login too; its sign-in is read without starting
  opencode.
- On Anthropic's and OpenAI's own APIs the forms that move work (the report, a ticket, the plan, the spec, a review)
  are decoded against their schema (`strict`), so their arguments always parse; other providers as before (ADR-144).
- A model its provider no longer serves, or soon will not (the price catalogue says so), is marked DEPRECATED on the
  agent's model badge and in the model picker, with the newest model of its family to use instead.
- When a model declines to go on (a safety classifier), the ticket says why — the vendor's category and explanation —
  and the declined answer is not kept, so resuming starts from the last good step; an answer stopped by a full context
  window is treated like one cut by the output limit; a run's output limit is the model's own (up to 64k) unless the
  agent sets one (ADR-140).
- API providers: Anthropic, Gemini, OpenAI-compatible and OpenAI Responses wires; 30+ vendor presets (OpenAI,
  DeepSeek, OpenRouter, OpenCode, xAI, Mistral, Groq, Azure OpenAI, Bedrock, Cloudflare …); local LM Studio / Ollama
  with detected capabilities.
- Custom OpenAI-compatible endpoints: extra headers (secret ones in the vault), per-model settings and effort levels,
  extra request fields and the effort field, under rules checked on save and on every request (ADR-115).
- Vendor CLIs driven headless with every tool call decided by Agenium: Claude Code, Codex, Gemini (Antigravity),
  Grok, OpenCode; login and usability detected; their own model draws CLI agents' lessons (an API model you pick can
  take that over).
- CLI agents learn with their own brain: lessons and facts are drawn by the agent's CLI, a read-only side call kept as
  its `Memory` run (the CLI's provider and model), unless an API model is set as the extractor; the memory itself is
  the agent's and is recalled the same for API and CLI agents.
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
- Vendor CLIs leave nothing behind: Agenium's check of the opencode login no longer starts opencode's background
  service, which stayed running with the user's MCP servers (opencode 2.1.2, `auth list --standalone`); the empty
  `\tmp\sessions\<id>` folder Grok makes at the root of the worktree's drive for every session is removed after the
  run (104 had gathered on one drive).

### Desktop app
- **The chat shows its model and changes it** (ADR-155): the Lead's chat names the vendor, model and effort it answers
  with, and "Change model" picks another there (the same warning when the context outgrows it); the conversation goes
  on, on the new model. The chat's gauge measures against the model now chosen, so a smaller model shows how far over
  it is ("folded to fit at the next run") before the next turn; a run's gauge keeps the model that run had.
- **A model is picked from a list** (ADR-155): the model field opens the provider's or CLI's models under it, whole
  names with their price, free tag and window, filtered as you type (a name not listed can still be typed). A model
  that reports no effort levels says so, with a link to set them per model. Claude Code is offered by full model ids
  only (its `haiku`/`sonnet`/`opus` aliases change meaning with each release; an agent set to one still runs).
- **`@` names a file in the chat** (ADR-152): typing `@` lists the conversation's documents (attached now or sent
  earlier), narrowed as you type; Enter or Tab puts `@name` in, so ten attached files need no retyping.
- **An attached file can be opened again** (ADR-152): a document attached in the Lead's chat is also kept whole in
  the project's `.agenium/attachments` (outside git), every run of the project may read it, and the message names
  where; asked to check the original, the Lead opens it with `read_file` instead of saying it is not there.
- **A CLI's tool calls are named** (ADR-153): a Codex or Antigravity MCP call reads `server :: tool: argument` in the
  chat and the Live view (it read `mcp_tool_call`); the CLI's call reaches the live view with its arguments.
- **CLI updates say what they know** (ADR-153): no Update button on a CLI at its newest; "Run its updater" where the
  newest is unknown; a CLI whose own check fails says why (grok: "program not found"); an update that changed nothing
  says it is already the newest, and the CLI is shown at its newest after; a version that answered Agenium's check
  after an update here is no longer flagged as beyond the verified one ("checked on this machine").
- **The chat shows an agent's turn as a CLI does** (ADR-155): on the agent's side, what it recalled, each call it made
  (tagged, red when it failed) and its answer, in the order they came, while the turn runs and after it; the chat page
  and the Live view's chat are one state.
- **Python in the sandbox** (Windows): PATH there leaves out the Store's aliases and takes the Python install
  manager's runtime in place of its launcher, so `python` runs in an agent's command.
- **No file written with read_file's line numbers** (ADR-158 decision 19): write_file and edit_file refuse such
  content and say why; a docs worker had written a whole spec so.
- **The live view's "max at once" follows the teams**: each running project counts its workers (one ticket each),
  up to this machine's ceiling (`runtime.max_parallel_agents`); it showed projects × the ceiling, as if fixed.
- **Devin and Grok on the usage footer** (ADR-67), without numbers and saying where their quota is: neither reports
  it to another app (Devin's `/usage` exists only in its terminal UI; over ACP it is unknown).
- **Workers can take in what merged meanwhile** (`update_from_main`, ADR-158 decision 18): Agenium commits their work
  and rebases the branch on main outside the sandbox, where git writes are blocked; a conflict lists the files and a
  second call keeps the worker's version once it brought main's change in. The rebase messages point to it.
- **An API agent whose provider has no key waits for the user** (ADR-158 decision 17), told once, instead of failing
  its runs as plain errors with nobody told.
- **A Graph tab on the Memory page** (ADR-158 decision 16): an agent's entities and their relations, each memory tied
  to what it names, contradictions in red; click a node for what it touches, scroll to zoom, drag to move.
- **The pre-merge check has the network the work had** (ADR-158 decision 15): a test suite that starts its own server
  passed in the worker's run and failed the merge on Windows (`connect EACCES 127.0.0.1`); the check now runs under
  `[sandbox] network` like the work's own commands.
- **The Lead sees where each member may write** (ADR-158 decision 14): `get_team` lists `allowed_paths` and every
  MCP server of a member; a ticket whose paths the worker's role does not cover is not assigned to it (the Lead is
  told which paths), instead of the worker stopping at its first write for a path permission.
- **No user comment on a ticket left anywhere** (ADR-158 decision 13): the user works through the Lead or Solo only.
  A review that keeps stalling gets another try after the user's next message to the Lead (it was a ticket comment
  no screen could write). An unused dependency (`ulid` in documents) is gone; `cargo machete` is clean.
- **A Lead that leaves its inbox as it is** (the user said to wait) is not run again and tells the user why, once;
  and **Devin's own commands run in the sandbox on Windows** (PowerShell, not WSL's bash), their output in the log.
- **A Lead gets no inbox run while it has a run going**: a chat turn and an inbox run planned the same work at once
  and rewrote each other's plan and tickets; and the inbox run now sees the user's latest words, so "do not start yet"
  holds there too.
- **OpenCode's, Agy's and Devin's own tool results** reach the log and the chat (shapes taken from captured streams;
  `AGENIUM_CLI_STREAM_DIR` captures a CLI's stream for diagnostics).
- **A learning can be undone** (ADR-158): the run page's learning step undoes what it wrote to the memory in one
  transaction (added facts out, replaced or retired ones back); refused when a later learning changed one of its facts;
  a proven lesson asks first; skills built on lessons gone are flagged.
- **A CLI session follows another model of its CLI** when what it holds fits that model's window (70 %, from the
  catalogue); before, any model change started over with the conversation as text.
- **The shared lesson pool needs two projects**; an agent's own skills still need two tickets.
- **The live view shows how many runs may go at once** (running projects × `runtime.max_parallel_agents`), and a
  plugin's preview names the environment and headers of its MCP servers.
- **The Lead reads its board** (Lead and Solo): `list_tickets` (tickets and their progress, counts by status) and
  `read_ticket` (one ticket whole with its comments), read only. A Lead that lost track of what it created looked in
  the shell before (a Grok Lead read Agenium's store).
- **CLI tools' results are kept**: Claude Code's and Grok's own tool results (a tool search lists the tools it found)
  and Codex's command output reach the run's log and the chat; before, every CLI tool of its own showed no output.
- **One tool search for every CLI**: Claude Code no longer defers Agenium's tools behind its own `ToolSearch`
  (`ENABLE_TOOL_SEARCH=false`); they are listed as for the other CLIs, and other servers' tools are found with
  Agenium's `tool_search`. Grok keeps its `search_tool`/`use_tool` dispatchers, its only way to reach MCP tools.
- **The chat folds a run of tool calls** into one line (how many, failures, the newest), opened on a click, so the
  agent's text keeps its room; and it shows a Lead turn this pane did not send while it runs.
- **Devin runs on Agenium's configuration only** (ADR-156): in the sandbox it sees its login and its logs and
  sessions, not the user's own config, MCP servers or plugins; the user's Devin plugins stay as they are, untouched.
- **Plugins** (ADR-157): a vendor's plugin (`obra/superpowers`, `owner/repo@tag`, or a folder) is previewed, then
  installed at the commit shown, kept in Claude Code's formats: `agenium-plugin.json` (a `plugin.json` of what was
  taken) beside `skills/`, and a Claude-shaped `installed_plugins.json`. Its skills and MCP servers are taken; hooks, commands, agents and programs are listed and
  never run. An agent's Plugin tab switches a whole plugin; its skills are read-only to learning; an update shows what
  changes and replaces the whole set; removing it switches it off everywhere.
- **Plugin hooks, Agenium's way** (ADR-157): `SessionStart`, `UserPromptSubmit` and `PreToolUse` command hooks run in
  the sandbox for the agents that have the plugin on; their text joins the run's instructions and is never stored, so a
  plugin switched off or removed leaves nothing in a prompt (its agents' kept CLI sessions end too). The chat shows
  each hook a turn ran, folded, and a plugin skill's call as `Plugin::<plugin>::<skill>`.
- **CLIs checked after their updates** (ADR-153, 2026-11-04): agy 1.5.0 and devin 3000.11.234 verified with real
  turns; Devin's new "fusion" pairs (655 variants of a model with a SWE-2 sidekick) are left out of its model list.
- **Devin's models with their efforts** (ADR-156): `swe-2` with medium, high and max (not `swe-2-max` as a model of
  its own), each run given the variant of its model and effort.
- **Devin CLI is driven** (ADR-156): over ACP inside the sandbox, one process per turn with its session loaded again;
  files it hands over are read and written by Agenium after a check (nothing in a read-only run); the user's own
  Devin MCP servers are off by name and its folder is read-only but its logs and sessions; models from `devin models
  list` (`swe-2-max` free). Verified with a Solo on `swe-2-max`: spec, plan, two tickets merged, a memory run.
- **Tools Agenium runs are tagged Agenium** in the chat and the Live room log, whether an API model or a CLI (through
  Agenium's MCP server) calls them; a CLI's own tools are Tool, another server's MCP.
- **The recalled memory is the agent's, not the user's words** (ADR-155): it travels in its own `<memory>` tag in the
  turn's message (the history stays a cached prefix), learning from the chat no longer reads it as something the user
  said, and the chat shows it as "Recalled from its memory" under the user's message.
- Review fixes from the agents' own review of Agenium: a CLI's memory or helper call cut short (its step limit, its
  time) is no longer taken as an answer; a fold of a CLI's conversation never folds the turn's own instruction,
  reserves its notes and keeps the first message whole; a Lead's answered turn whose history could not be stored is
  not answered again; the reply to a message sent while a turn starts says which turn reads it.
- **The Lead sums the work up when it is done** (ADR-147): a project that becomes done gets a short summary from its
  Lead or Solo agent in the chat (what was delivered, how it was checked, what is open), in the user's language.
- **No comment box on a ticket** (ADR-122): the user's requests and corrections go to the Lead or Solo agent, who
  writes to the ticket; the ticket popup and `agenium ticket comment` no longer write there.
- **The chat follows its turn whatever the brain**: the calls of the turn in progress are listed one per line (an
  API model's ran too fast to be seen), a message just sent no longer disappears mid-turn (the live store's dropping
  of old runs reloaded the chat), a run is done when it ends rather than after its first model call, tables in an
  agent's message are drawn, the recalled memory is no longer shown as the user's words, and the Live page's chat is
  the same conversation with the same behaviour. A tool of Agenium reads the same whether codex or an API model calls
  it, and the Live room log keeps the calls made while another page was open.
- **A held message says why** (ADR-149): messages put back after a failed turn no longer read as "busy": the queue
  says the last turn failed and offers "Send again"; the waiting messages show their documents as chips, and editing
  one changes only its text.
- **The chat's model dialog keeps the keyboard** (ADR-149): "Change model" opens a dialog that takes the keyboard and
  closes with Escape; it stayed open while the chat behind it could still take a message.
- **Warned before a smaller model** (ADR-155): changing an agent to a model whose window its context outgrows (say
  550K tokens of context, a 262K window) says so under the model picker and asks first: the next run folds the
  conversation to fit.
- **Context gauge** (ADR-155): the run page, the Live dock and the chat show how full the model's context is (tokens
  read at the last request / its window, amber from 60 %, red from 85 %); compaction, folding and the other steps
  Agenium takes on a run read as sentences on the run's page.
- **The work folder is chosen at install** (ADR-154): the Windows installer has a page for where Agenium keeps your
  skills, MCP servers, agents, projects and memory (for example `D:\Agenium`; `/DATA=` for a silent install); on Linux
  and macOS the app asks once at its first start. No setup screen: the app opens on its main screen in the system's
  language. The MSI is no longer built.
- **Agenium ships no skills and no MCP server** (ADR-88 amended): a new root's library starts empty and says how to
  add one; it holds what you add and what you accept from your agents' lessons. An upgrade replaces the program only:
  your skills, MCP servers, agents' copies and data stay as they are. Roots seeded with the old starter skills keep
  them.
- UI review of 2026-11-01: the status badges (agents, tickets and the board's columns, run outcomes, criteria),
  the ticket chips (approved, merged, reopened, escalated), the pool and the model tags (free, deprecated, price) are
  in the interface's language; **Needs you** words each item in it too (the item carries its sentence as a template
  and its values beside the English; what an agent wrote stays as written), and so does a CLI update's check. A page,
  dialog or ticket popup that fails to render shows what failed with **Try again** in its place, instead of the whole
  window going blank. The browser demo answers the Lead's queue and the role list (the chat page and the agent page
  were blank in it).
- **Propose a team** (ADR-151, Templates and Teams pages): describe the team you want in a short chat; a model you pick
  (an API provider or a CLI, its model and effort) proposes the roles, how many of each, their library skills, write
  scopes and models, with why. Agenium checks the draft (only skills and models you have; a valid template), and it
  opens in the template editor; nothing is created until you save it. A skill the library lacks is proposed with a
  button to write it.
- **Write a skill with an agent** (ADR-151, Skill & MCP page, beside writing one by hand): say what the skill should do
  and add text documents; the model you pick writes it to Agenium's skill rules (the five sections, a description that
  says when, detail in `references/`), and it opens in the skill editor to evaluate and save. Documents are material,
  never instructions.
- A template role's skills may now be library skills: a team made from the template gets them on the role's agents.
- **One skill library, agents propose** (ADR-152): an agent no longer adds a skill by itself; a lesson that held in two
  tickets becomes a proposal in **Needs you**, with its evidence and the library skills it is close to. Review it by
  hand in the skill editor, discuss it with the skill writer, or decline it; what you save goes into the library and is
  switched on for that agent. Agents still improve only their own copy of the skills they have.
- The skill writer sees the whole library: the local embedding model (no API tokens) finds the skills close to a
  request or a draft; a draft shows them with **Merge into X**, and a merged draft opens in the editor on that skill
  with a before/after diff. A skill's **quality on 12 criteria** is checked on this computer (editor, drafts,
  proposals), and the library cards show how the tickets that loaded a skill fared (reopened or not).
- **Attach documents** in the Lead/Solo chat, the team designer and the skill writer: pick files, drop them on the
  window, or paste a file's path. PDF, Word, PowerPoint, Excel, EPUB, CSV, HTML, notebooks and text are read as
  Markdown; agents' `read_file` reads them too.
- **markitdown in Settings → Documents**: install Microsoft's markitdown for Agenium (no Python needed: a pinned,
  checksummed uv fetches one), self-tested on sample documents; a weekly look for a newer version that only tells you;
  update side by side (the working copy stays when the new one fails); remove. It runs in the sandbox.
- The team designer's and the skill writer's chats are kept (History): reopen one where it stopped. The model picked
  last is remembered for the agent form, the team form, the skill writer and the team designer.
- A team from a template can run each role on the model the template names, where this computer runs it.
- The usage page counts the team designer's and the skill writer's calls.
- A helper chat is called by its draft (the skill's or the team's name) once it has one; name it yourself in History or
  in the chat's header, and the name stays.
- **Update an agent CLI from Providers & CLIs** (ADR-153): each CLI shows the version it has, the newest one and the
  one Agenium verified; Update runs the CLI's own updater (asked first, never while one of its runs is going), then
  asks it one short question; if that fails, go back to the version you had (Claude, opencode, Grok).
- Code audit of 2026-10-31: approving a ticket no longer freezes the window during the pre-merge check; Needs →
  Open shows the item's own project, and one failing project no longer empties the page; deleting an agent's skill,
  resetting or deleting a template and removing a judge key ask first; dialogs take the keyboard (focus, Tab, Escape)
  and are marked as dialogs; the browser preview hides under popups; Enter mid-IME no longer sends; the Live page
  reports a failing backend once; `dompurify` raised past its advisories.
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
- Installers: Windows (NSIS, MSI), Linux (`.deb`, AppImage) and macOS (an ad-hoc signed `.app` in a `.dmg`, not
  notarized; Apple silicon; checked in macOS run 7: installed from the `.dmg`, it starts and copies its model). Found on a clean Windows 11 VM (2026-11-01): the app built since the vector index (ADR-125) did not start
  there, as it needed the C++ library of the build machine (`libstdc++-6.dll`); it is now linked in, and local CI fails
  a Windows build that imports a DLL of the build toolchain (`check-dlls.ps1`).
- A newer version installs over the old one and keeps everything: the data lives in the root (`~/.agenium`), apart
  from the app's folder, and uninstalling leaves it too. Checked on a clean Windows 11 VM (2026-11-01) through the
  installers' own windows: 0.1.0 with a project, a team, a provider and its key, then 0.1.1 over it while the app ran
  (closed and restarted by the installer), then the MSI; on Ubuntu, `apt install` of 0.1.1 over 0.1.0 changed no file of
  the root. The app now shows the installer's version, not the crate's.
- Strict Content Security Policy: only the app's own code runs in the window; no remote content.

### Safety
- Code audit of 2026-10-31: tool outputs and the sandbox's git config are no longer written through links a command
  can plant; the file tools refuse a FIFO instead of waiting on it; command output is kept within 16 MiB (its first and
  last 8 MiB); unsandboxed Unix commands leave no background processes; a CLI agent cannot write the files its CLI
  loads as configuration into its run's home; agy's hook denies tools it does not know; Codex can no longer overwrite
  your own login through the shared `auth.json`; the Claude usage probe runs with a clean environment.
- An MCP tool its server marks destructive (deleting, refunding, sending) waits for the user in Needs you, like a
  command: allowed once, always for the project, or refused; set for every server or per server on the MCP tab, down to
  every tool that is not read-only, or none (ADR-143).
- Web pages and MCP answers reach the model in a fence that names their source and closes with a random id, so a page
  cannot end it and speak as Agenium; an opt-in golden task checks that agents do not follow instructions planted in a
  file (DeepSeek, Codex and Claude Code passed) (ADR-142).
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
  as an API agent. The vendored Codex sandbox follows upstream 3a2788e (2026-11-04: deny-path and
  volume-root handling Agenium does not use, Linux and macOS refactors with no new hardening; nothing to carry over).
- On Linux a sandboxed command can no longer have the host resolve names with the network off: mxc-sdk 1.0.1 brings
  the host resolver's folders into the sandbox whole, with systemd-resolved's socket in them; Agenium masks every
  socket there, `/etc/resolv.conf` stays readable (containment row `net.resolver_socket`; fixed upstream after 1.0.1,
  microsoft/mxc#1568). The containment harness counts a payload that printed nothing as broken, not as 0 escapes.

### Memory, skills, tools
- Shared lessons answer for the work they mislead (audit M3, ADR-87 amended): the pool lessons a run is shown are
  logged in the pool, a reopened run's learning can name one as misleading (never confused with the agent's own),
  and a lesson that misled two tickets leaves the pool for good; the lesson it came from is untouched. The run page
  shows both. The memory extraction finds the memory that misled a reopened run far more often (DeepSeek flash:
  21/40 → 40/40 on the same cases), asks again once after an empty answer, keeps an episode's blame when a later
  step fails, and reads an id written as text. The run page also shows a settling pass and the lessons reflected on
  (they showed as a failed extraction with raw JSON).
- Code audit of 2026-10-31: the shared lesson pool follows the lessons it copied (retired, refined or less sure:
  the copy goes); a revised agent skill passes the same shape and size gate as a new one, and "one lesson, one skill"
  follows a lesson's refinements; a refused reflection is asked again (up to 3 times); recall runs off the async
  threads; an MCP server's error text reaches the model cut and fenced; a stdio server's non-UTF-8 line no longer hangs
  it; a context overflow reported with a 5xx is compacted, not retried.
- Learning is credited to what the agent was shown (a continuation recalls nothing), and owed by every close or reopen,
  the user's on the board too; a skill an agent created is read by the judge before a revision (ADR-147).
- One lesson per rule: a refinement updates the lesson (keeping where it held), a too-narrow lesson is blamed when it
  misleads, and a ticket closed without a signal can no longer delete a lesson. A lesson proven in two tickets is
  reflected on once — improve a skill, make one, or keep it as a memory — through the same gates as a proposal in a
  run; an agent can now improve the skills it made. On real memory chains (DeepSeek, Codex) agents now write and
  revise skills from their own experience (ADR-146).
- An agent's memory is settled as it grows: memories that say the same are folded and ones that contradict each other
  (a changed port, version or tool) are put to the user on the Memory page, found by the local embedding model and,
  with a judge key, a few typed questions — no model rereads the memory (ADR-145).
- Per-agent memory with a local embedding model (bge-small), keyword and knowledge-graph search, scoped facts, lessons
  with decay, consolidation; strictly gated learning of new skills and lessons.
- Memory stays fast past 100 000 facts per agent: an approximate (HNSW) vector index takes over from exact search,
  built in the background, checked on every start and rebuilt when it cannot be trusted (ADR-50, ADR-125).
- `SKILL.md` skills (13 built in), skill overlays and agent-created skills behind evidence gates.
- A skill's description says what it does and when to use it — the only text an agent reads before picking one: the
  built-ins say so, and import, save and Evaluate advise when a description does not (ADR-141).
- MCP servers (stdio, Streamable HTTP, legacy SSE, OAuth), bound per agent with their own credentials: two agents
  reach one server with different keys and so different rights (an admin's, a member's), API and CLI agents alike;
  an own key pasted bare keeps the system key's `Bearer ` (it reached the server without it and was refused).
- An agent with many MCP tools (past 20, or 10k tokens of definitions) gets the few it uses most and finds the others
  with `tool_search` — by words, meaning and, with a judge key, Jev's reading (55 of 58 requests found the right tool
  first among 123) — and calls them with `call_tool`; requests stay small and cached on every vendor. Jev reads only
  when words and meaning disagree or the agent searches again, about half the searches, for the same choices. A server
  whose tools change during a run (it says so, or its own tool search runs out of quota and it lists everything) is
  followed; a server whose tool names say nothing is summed up by its descriptions. The thresholds and the judge are
  set on the desktop's MCP tab (ADR-139).
- `tool_search` ranks real servers' tools better: swept on 330 tools of GitHub's, Sentry's, Playwright's, Notion's,
  Slack's and git's MCP servers with 128 requests, a tool's meaning is now taken from its name and the start of its
  description (long descriptions diluted it), and words and meaning are fused with k = 10 and meaning weighed 1.5. The
  right tool is among the 5 a search returns for 93 of 128 requests (86 before) and 53 of 58 on the earlier set (52);
  no model API, same local embedding model (ADR-139).
- `view_image` for vision models; `fetch` through an allowlist.
- Background commands for every agent: `shell` with `background` starts a server or watcher, `shell_output` reads it
  while it runs, `shell_stop` ends it; all stop with the run (ADR-129).
- A Solo agent's CLI subagents are shown as such: the run page and the Live tab label and indent each subagent's
  steps under the step that started it (ADR-129).

### Optional judge
- Code audit of 2026-10-31: only a quota or a rejected key sets a judge key aside (another failure pauses the judge
  a minute); requests too large for the judge are not sent; a key whose vault value is missing is passed over; an
  output whose full text cannot be saved is not trimmed.
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
- Code audit of 2026-10-31: incremental backups now hold every agent's memory (restoring the newest set no longer
  brought memory back from the day's full set); a full set is taken as soon as the schema moves, and a set that spans
  a migration is refused plainly; a half-written backup folder no longer breaks maintenance; pre-migrate snapshots are
  kept by version number.
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
