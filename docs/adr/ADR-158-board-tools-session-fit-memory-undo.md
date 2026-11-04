# ADR-158: The Lead reads its board; a CLI session follows a model that fits; a learning can be undone

- **Status:** Accepted; implemented (2026-11-04)
- **Date:** 2026-11-04 (Asia/Saigon)
- **Amends:** ADR-155 (a session resumed only on the model that wrote it), ADR-118 (the shared pool's evidence),
  ADR-129 (a CLI's own tools), ADR-139 (one tool search).
- **Owner's words:** "thêm tool đó đi, cho lead/solo xem tiến độ ticket, đọc ticket đã và đang tạo"; review notes
  `agenium-remaining-work.md` weighed against the code ("hãy làm những gì hợp lí cho agenium"); work stays in English.

## Context

A long task on a team (Grok 4.7 Lead; Codex, Devin, Agy, OpenCode members) and a Solo (DeepSeek) showed:

- The Lead created tickets for plan items it had covered already (8 refused by the scope and dependency checks), then
  searched for a tool to list them, found none, and read Agenium's store with 22 shell commands.
- Every tool of a CLI's own showed no result in the log or the chat: the drivers dropped the `tool_result` content.
- Claude Code deferred Agenium's tools behind its own `ToolSearch`, the one CLI with a second search.

The owner's review notes, checked against the code: a session was dropped on any model change, a window as large
included; the shared pool counted two tickets of one project as two contexts; a learning could not be undone; the
run timeout and per-ticket cost were already configurable and shown (`cli.run_timeout_mins`, the ticket's Usage);
the scope-hierarchy note was wrong (`workspace` and `project` are different keys, a 0.6 factor, not a conflict).

## Decision

1. **`list_tickets` and `read_ticket`** (Lead group, so Solo too; read only, no event): the board with its progress
   (status, assignee, plan item, dependencies, scope, approved/merged, counts by status; open tickets by default) and
   one ticket whole with its newest 30 comments. Names and descriptions use the words the Lead searched with ("list
   tickets", "get ticket details", "board", "progress"). The inbox prompt names them.
2. **A CLI tool's result** (`Event::ToolOutput`) reaches the log and the chat: Claude Code's and Grok's `tool_result`
   (a tool search lists the tools it found as `tool: <name>`), Codex's command output (an error when it exits non-zero).
   OpenCode (`session.tool.success`/`error`: `data.content[].text`), Agy (`step_update` DONE/ERROR:
   `tool_info.output` or `tool_info.error.message`) and Devin (ACP `tool_call_update` completed/failed:
   `content[].content.text`) too, from streams captured on real runs (amended 2026-11-05). `AGENIUM_CLI_STREAM_DIR`
   appends each line of a CLI's stream to `stream.jsonl` there, for learning a CLI's shapes; off by default (it holds
   file contents and tool output).
3. **One tool search.** Claude Code runs with `ENABLE_TOOL_SEARCH=false` and without `ToolSearch`: Agenium's tools are
   listed as for every other CLI, other servers' tools found with Agenium's `tool_search`. Grok keeps `search_tool` and
   `use_tool`, its only way to reach MCP tools; its config holds Agenium's server alone.
4. **A session follows a model that fits** (amends ADR-155): the kept session records the context it filled at its
   last step; another model of the same CLI resumes it when that context is at most 70 % of the new model's window
   from the catalogue. A window not known, or a CLI that reports no context (only Claude Code and opencode do), keeps
   the old rule: the model that wrote it.
5. **The shared pool needs two projects** (amends ADR-118): a lesson shared with every agent must have held in two
   projects. An agent's own skill still needs two tickets (the owner's rule of contained learning); leaving the pool
   still takes two misleading contexts.
6. **A learning can be undone.** Each change of a memory episode names its facts (`id`, `old_id`). The run page's
   learning step has "Undo this learning": in one transaction, the facts it added are retired and those it replaced
   or retired are live again (their vectors rebuilt by the next refresh). Refused, with nothing changed, when a later
   episode touched one of its facts (undo that one first) or when it was undone already (`memory-revert` log line). A
   lesson that held in contexts since is named first and undone only when confirmed; skills built on lessons gone are
   flagged for review. Episodes learned before this cannot be undone.
7. **Small views:** the live view shows "running projects × `runtime.max_parallel_agents`" (the setting is per
   project); a plugin's preview names the environment variables and headers its MCP servers get (names only). The
   chat folds a run of tool calls into one line, and shows a Lead turn another window or a script sent.

8. **No inbox run beside a Lead's run** (amended 2026-11-05). A chat turn marks no one Working, so the scheduler gave
   the Lead an inbox run while its chat turn still planned: both made the plan and its tickets at once and rewrote
   each other's (a DeepSeek Solo's plan revised three times, four tickets cancelled, 16 minutes). The inbox now waits
   while the Lead has a run going. The same task planned again: one run, one revision of its own, nothing cancelled,
   3.6 minutes.
9. **The inbox run sees the user's latest words** (the last three messages of the chat, as written): it had assigned
   the tickets of a user who had just said not to start, since it saw no chat.
10. **Load on one worker is the team's shape, not a fault**: a ticket's required role must match, and a planner takes
   no tickets; the team of the long task had one backend for nine backend items. A second backend spreads that work.

Known: Devin's own `exec` exits 0xC0000142 in MXC (it starts Git Bash, which cannot start in the container, as for
plugin hooks); its commands go through Agenium's `shell`.

Not taken from the notes: facts from the chat in the chat's language (work and memory stay English: facts are recalled
into English work runs, the embedder is English, and the extraction already writes them in English in the same call);
a hash in place of the held count (no dropped session seen yet; a folded conversation would change the hash anyway);
scope hierarchy (no workspace exists); MCP servers of plugins in the sandbox (the owner's sandbox level is Codex's).

## Verified

- Unit and integration: `the_lead_lists_and_reads_its_tickets`, `another_model_of_the_same_cli_resumes_when_the_session_fits`,
  `two_tickets_of_one_project_are_one_project`, `an_episode_is_undone_and_a_later_change_refuses_it`,
  `a_learning_run_is_undone_once_and_a_proven_lesson_asks_first`, the Claude parser's `tool_result` text.
- In the app: the folded tool calls in the Lead's chat (screenshot); the real runs of the new tools follow.
