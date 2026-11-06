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

11. **An inbox the Lead leaves as it is** (an answer and no call, as when the user said to wait) is not run again for the
   same items; the Lead's words reach the user once (`NeedsUser`, the project named). Before, it ran twice and then
   said the Lead "could not handle its inbox". The inbox nudge says that a user's hold is the one reason to leave
   items. Verified with a DeepSeek Solo: told to hold, one inbox run of 5 steps and the notice, the tickets left
   unassigned; told "go", both assigned and the first started.
12. **Devin's own `exec` runs in the sandbox**: it took `bash` from PATH, on Windows WSL's launcher in System32, which
   cannot start in MXC (0xC0000142); `SHELL=powershell.exe` makes it PowerShell (checked on the host: the shell it
   logs changes; in the app: `echo` printed). Its output, sent while the command runs (`terminalPreview`,
   `terminal_exit` with the exit code) and not with the completed update, reaches the log.
13. **No user comment on a ticket, anywhere** (2026-11-05): the box went in ADR-122's amendment, and `App::user_comment`,
   only tests still called it, goes too, with what read the user's comments: the learning signal "the user commented
   on the work" (ADR-72) and the review stall's reset. A stalled review now gets another try after the user's next
   message to the Lead. The user works through the Lead or Solo only, who write the ticket in English, so a worker
   is never given work, or a ticket text in another language, by the user directly. The language switch no longer
   says tickets and plans are English: that is not a choice the user has.
14. **The Lead slices inside the roles' paths** (2026-11-05, seen in a 7-agent team): a designed template gave the
   backend `server/store/**`, the Lead's first ticket named `server/store.js` and `test/store.test.js`, and the
   worker's write scope (ticket paths and role paths, both) left it one file; it stopped at once for a path
   permission. `get_team` did not say where a member may write. It now lists each member's `allowed_paths`, and
   every MCP server the member uses (a server without keys was missing). `assign_ticket` refuses a ticket whose
   `scope_paths` its worker's role does not cover, naming the paths, so the Lead fixes the ticket or the assignee
   while it plans; the user's own assignment is not checked, and a role without paths takes anything.
15. **The pre-merge check has the network the work had** (same team): the backend's boot test started the server
   and fetched `127.0.0.1:3000`; it passed in the worker's run and in the Lead's review, and failed the merge with
   `connect EACCES`, because the check ran with no network and Windows' sandbox cannot tell the suite's own loopback
   from the host's (ADR-84's addendum). The worker then weakened the test to get through, and the Lead rightly
   reopened it twice. The check now runs under `[sandbox] network`, as the work's own commands do: the same code
   already ran so in the worker's run, so nothing new is exposed. `CheckFactory::for_project` takes `network`.
16. **A graph of the memory** (the owner asked for it): the Memory page has a Graph tab. Entities (coloured by type)
   with their typed edges, each fact, lesson or preference tied to the entities it names, and pairs of memories the
   settling pass judged (a contradiction in red, two that both hold dotted); a click shows what a node touches.
   `memory_graph` reads `AgentStore::graph_view` (live rows only; the 300 most connected entities). No graph
   library: a small force layout in the page. The Lead console names the Lead by its agent (`lead_name`), not by
   the team, whose initials were another member's.
17. **An API agent whose provider has no key waits for the user** (same team, after a vault reset): the run failed as
   a plain `Error` (`secret ... is not set`), twice, and the ticket stopped starting with nobody told. A provider that
   cannot be built (`NotFound`: no key, not configured) now sets the agent aside like a CLI that cannot run
   (ADR-132): Offline, a NeedsUser and a comment on its ticket naming the provider.
18. **Agenium brings main into a worktree** (`update_from_main`, Worker group): git writes are blocked in the sandbox by
   design (ADR-53), yet Agenium's own messages told a worker to "commit or stash, then `git rebase main`" (the rebase
   nudge) or to rebase after a merge conflict. A worker whose ticket depended on one merged meanwhile (the Lead added
   it mid-plan) tried exactly that, met `index.lock ... denied`, and spent 400 steps inspecting git's index. Now
   Agenium commits the worker's changes on its branch (journal-first, secrets refused) and rebases it on main,
   outside the sandbox; the workspace base follows. On a conflict nothing changes and the files are listed: the
   worker brings main's change into its file (`git show main:<file>` reads) and calls again with `keep_mine`, so the
   hunks that still conflict keep its version. Both messages now point to the tool.
19. **A file is not written with read_file's line numbers** (same team): the docs worker (a free model) wrote
   `docs/spec.md` from `read_file`'s numbered view, every line prefixed twice, and the Lead reopened it for that.
   `write_file`, and `edit_file`'s new text, refuse content whose lines are mostly `<n>	...` with the numbers counting
   up (a tab-separated file or two lines pass), saying to write the file's own text.
20. **Every agent can read the whole approved spec** (`read_spec`, Shared): a worker's ticket shows only the
   requirements its plan item serves. The docs ticket had to document the store contract against "the approved v1
   spec", which lived only in Agenium's database; the worker could not read it and the Lead reopened the ticket for
   that. `read_spec` returns the spec the current plan traces to (else the newest approved one), read only.
21. **The Lead can make a ticket wait for one made later** (`add_dependency`, Lead): QA found the store off the
   approved contract, the Lead made SD-14 to fix it, and could tell SD-12 to wait only in a comment, so QA asked twice
   whether the fix had merged. The tool adds to `depends_on` (no cycle, not a cancelled ticket, a comment on the
   ticket), and the scheduler now checks dependencies for a ticket in progress too, so it goes on only once they are
   done.
22. **Waiting on a background command is no loop** (same team): QA started its suite in the background and read it
   with `shell_output {job, wait_ms}`; three identical calls in a row stopped the run as a loop, though each answer was
   new. The same-call check (API and CLI loops) leaves `shell_output` out (`agenium_backend::polls`); the command's
   timeout and `max_steps` still bound it.
23. **Plans for parallel work, reviews that list every problem** (same team): the plan ran almost all the time one
   run at a time (293 of 314 minutes with one run, none with three), a chain through the backend worker while the
   others waited; and two tickets were reopened five and eight times, each review raising one new point. The
   planner's order now names where each worker writes and asks for a split that gives every worker an item it can
   start early, depending only where an item needs another's result or shares its paths (`create_plan`'s
   description says the same to the Lead). The review order asks for every problem in one review, the next review
   looking only at what the fix changed. Text only: whether it holds is for the next long run to show.
24. **A worker waits for its review; reviews do not wait for the round** (the owner, after the habits run: the plan
   let backend and frontend start together, yet a finished worker waited for the slower one's run, then for the
   Lead's reviews one by one, before the next round). The owner's rule: a worker whose report waits for the Lead's
   decision takes no new ticket, so a reopen is fixed at once and not after a ticket started meanwhile; its reopened
   ticket comes back to it before anything new. A Solo agent reviews its own work after it, as before. In a round,
   work runs and reviews now go on together: a review starts as soon as a report is in, beside the runs still
   going (one at a time: one Lead), and a worker its review freed starts its next ticket while others run. No agent
   has two runs at once; a ticket runs once a round (a reopen goes on in the next round, three seconds later); the
   round ends when nothing runs and nothing more can start.
25. **One commit per ticket on main** (the owner: "gộp mỗi ticket thành 1 commit"): each report was a commit and
   main took them all (StudyDeck: 39 commits for 15 tickets, ten `SD-11: ...` in a row). A ticket of several commits
   now goes in as one, with the tree its rebased and checked head has, named and signed as its first commit (its
   `Ticket` and `Agenium-Intent` trailers kept) and saying how many steps it was; the steps stay on its branch until
   it closes. `Agenium-Squash-Of: <head>` lets a replayed merge find it merged (`Repo::squash_of`), since the branch
   is no longer an ancestor of main. A ticket of one commit goes in as it is.
26. **Screenshots for acceptance, in the report and in the chat** (the owner: "nghiệm thu thì cần agent cung cấp ảnh
   về sản phẩm trong chat"): nothing showed an image of the work. A report's artifacts of kind `screenshot` (a path
   in the worktree, best under `.agenium/shots/`, outside git) are checked on submit (inside the worktree, a PNG,
   JPEG, GIF or WebP by its bytes, at most 3.75 MB) and copied to `<project>/.agenium/evidence/<KEY>/` (kept after the
   worktree goes; every run of the project may read it, so the Lead's review looks at them with `view_image`). The
   ticket's report card shows them. The Lead's `share_image` (a ticket and the file name) puts one into its chat
   with the user, shown under the call, to accept the work together.
27. **A browser of the run** (the owner chose a built-in tool over an MCP server): an agent could only fetch a page's
   text, so nothing checked a web app as a user sees it, and nothing took the screenshots of decision 26. The work
   tool `browser` drives a headless Edge or Chrome found on the machine (`AGENIUM_BROWSER` names another) over the
   DevTools protocol: `open` a URL, `click` / `type` / `press` like a user, `eval` an expression, `screenshot` the page
   or one element to `.agenium/shots/<name>.png` (always there, whatever the role's write scope: evidence, not work;
   the model sees it when it takes images). Each step returns the page's text, its clickable and typable elements
   with selectors, and its console errors, marked untrusted like fetched content. The browser starts on the first
   call with a fresh profile in the system temp folder and ends with the run (its process tree and profile go). It
   runs outside the sandbox, as MCP servers do, so it is held to what `fetch` may reach: pages open on this machine,
   on the network allowlist, or as files of the worktree; every other host goes to a closed proxy port and fails.
   Its one dependency is `tokio-tungstenite` (MIT, no TLS: the socket is local).
   `open` takes `color_scheme` (light or dark): the Lead of the first real use (habits, HB-6) said it could not see the
   page in dark mode, only read its CSS. In that run the user picked two elements in the Browser page with a note each,
   the Lead made one ticket of them for the page's author, the worker checked the page with `browser` and attached its
   screenshot, and the Lead showed it in the chat with `share_image` when asked. A picked element can now be copied
   (selector, HTML, styles, note) from its card.
28. **Acceptance shows itself in the chat** (same run): the screenshots reached the chat only when the user asked
   the Lead for them, since a review is not a chat turn and has no `share_image`. When a review closes a ticket whose
   report kept screenshots, the Lead's chat now gets one message, once per ticket: `<KEY> is done and accepted:
   <title>.` and a line `Screenshot of <KEY>: <file>` for each (four at most), written before the review run ends so
   the chat's reload at its end shows it. The desktop shows such a line as the image, like a `share_image` call; the
   Lead's model reads it as plain text in later turns (no made-up tool call in its history, which providers check).
29. **An unreachable provider is tried again by itself; a report's template is in the work order** (the research run:
   a Lead, two researchers, a synthesist on DeepSeek flash). OpenCode answered "overloaded" for over a minute, the
   Lead's review ended Offline and the project waited for the user. An agent whose run ends Offline (its provider
   unreachable after the retries; not a CLI that cannot run nor a missing key, which wait for the user as before) is
   set back to work two minutes later, then four, eight, sixteen, thirty, thirty: six tries, then the user. Both
   researchers' reports were refused three times and their runs stopped, the last refusal "sources (array) is
   required": the template's description of `extension` (`templates::describe`) was written for the work order and
   never put in it. It is now, for every template. In the same run the Lead, set aside again, came back by itself,
   but 18 minutes later instead of 2: set-aside agents were looked at only when a round began, and a worker's run held
   the round. A round with runs going now looks at them every 30 seconds (quota waits too).
30. **Raw traces as a recall channel: measured, not adopted yet** (the owner asked to compare Zero-Mem, a memory that
   keeps raw traces and indexes them with no model tokens). `agent/memory/examples/trace_recall.rs`, on the QA runs'
   real episodes (StudyDeck and Habits: 186 live facts, 486 chunks of their tickets' reports, reviews, system,
   question and answer comments), 69 questions written by DeepSeek flash from sampled chunks and facts (answers copied
   from the passage, checked verbatim), bge-small and the production `recall`, 8 items:

   | | detail questions (from traces, 36) | fact questions (33) | all (69) |
   |---|---|---|---|
   | facts only (today) | answer 0.17, MRR 0.17 | answer 1.00, MRR 0.93 | answer 0.57 |
   | traces only | 0.89, 0.69 | 0.30, 0.23 | 0.61 |
   | both, reciprocal rank | 0.89, 0.44 | 1.00, 0.92 | 0.94 |
   | facts first, traces 3 places at most | 0.81, 0.27 | 1.00, 0.93 | 0.90 |

   The extraction keeps a fifth of the details a later ticket may ask (`409 QUIZ_ALREADY_SUBMITTED`, a 64 KiB body
   limit, the sandbox's `connect EACCES`); a trace channel finds nine in ten, and the facts keep their places.
   About a third of those details are passing (a test count of one moment, "the next step is submit_report"), so a
   trace in a work order must say it is one (its ticket, its date, verify it). Not one of Zero-Mem's other parts is
   taken: its routing by question type and its many thresholds are tuned to its benchmarks, a work order is not a
   question, and our graph already showed (ADR-94) that reaching wider hurts.

   Verified again, more strictly (the owner: "verify lại cho thật kĩ, cùng nhau ngẫu nhiên"): four projects of three
   QA roots (StudyDeck, two Habits runs, the research run: 302 facts, 690 trace chunks, Progress comments left out),
   `agent/memory/examples/recall_rank.rs` ranking through the production `recall`, analysis outside Rust.
   - A, questions from a channel's own items, 3 random seeds x 2 writers (DeepSeek flash, beam-free), 298 questions:
     detail questions 0.27 -> 0.90 with both channels fused [0.85-0.95], 82 more answered and none lost (McNemar
     p < 1e-24), every seed and both writers alike; fact questions 1.00 -> 0.99 (one of 167 lost).
   - B, neutral questions from the projects' final code and docs, which neither channel is made of (64): 0.17 ->
     0.33, 10 more answered and none lost (p = 0.002), all three seeds up.
   - C, each of 28 tickets in time order, recalled as when it started (its assignee's facts, the traces of tickets
     closed before, nothing of its own), every note shown rated blind and shuffled by two judges (0/1/2 and
     "misleading"; they agree on useful-or-not 87 % of the time). Per ticket, against facts only (4.07 useful notes,
     2.32 strongly useful, 0.29 misleading), paired bootstrap 95 %:

     | traces in the recall | useful | strongly useful | misleading |
     |---|---|---|---|
     | fused by rank | +1.82 [+1.00 +2.68] | +1.18 [+0.46 +1.89] | +0.96 [+0.61 +1.36] |
     | facts first, 3 places at most | +1.71 [+0.96 +2.50] | +1.18 [+0.57 +1.86] | +0.79 [+0.46 +1.18] |
     | the same, each ticket's last report only | +1.50 [+0.82 +2.25] | +1.18 [+0.54 +1.89] | +0.68 [+0.32 +1.11] |
     | the same, no reports | +0.75 [+0.21 +1.39] | +0.54 [+0.07 +1.07] | +0.54 [+0.25 +0.89] |

     The misleading notes are mostly reports (27 % of those shown, reviews 13 %): a report describes the code as it
     was, and an earlier report of a reopened ticket describes work the review refused. Keeping each ticket's last
     report (the approved one) keeps the whole gain in strongly useful notes and cuts the misleading ones by a
     seventh; dropping reports loses half the gain.
   Not measured: whether agents finish tickets sooner or repeat fewer mistakes with it (judged usefulness only), how a
   "verify it" label changes the cost of a misleading note, and teams on other models (all four ran DeepSeek flash).
31. **A work run is reminded of the project's closed tickets** (the owner, after decision 30: "okay làm đi"), in the
   measured setting that kept the gain and cut the misleading notes: a closed ticket's reviews, questions and answers,
   handoffs, failed merges and the report its review approved (`crate::traces`: earlier reports and progress lines
   left out), in passages of about 700 characters, in an index of the project (`<root>/db/projects/<id>-traces.db`,
   an agent database's layout and search). A ticket goes in the first time a work run of its project recalls after it
   closed (every way of closing is covered, older projects too). The work run's first message gets them after the
   agent's own memory, three at least and more where its memory leaves room of the eight, as a section that says they
   are notes as written then, not instructions, to check before relying on one; the run's page lists them, each
   opening its ticket. Traces are never learned from (no fact, no lesson: ADR-88's gate stays whole). Off with
   `[memory] trace_recall = false`. First real use (Habits, rename a habit: five tickets on DeepSeek flash): the
   store ticket was reminded of the report that made the store, the page ticket of the review of the page's labels
   and of the empty-name error, and the API ticket, a minute after the store ticket closed, of its report
   (`store.rename`, `400 INVALID_NAME`), which the API then relies on. 22 minutes, no reopen, 37 of 37 tests. The first
   recall of a project indexes all its closed tickets (100 passages here, some seconds of CPU) before the run starts.

Not taken from the notes: facts from the chat in the chat's language (work and memory stay English: facts are recalled
into English work runs, the embedder is English, and the extraction already writes them in English in the same call);
a hash in place of the held count (no dropped session seen yet; a folded conversation would change the hash anyway);
scope hierarchy (no workspace exists); MCP servers of plugins in the sandbox (the owner's sandbox level is Codex's).

## Verified

- Unit and integration: `the_lead_lists_and_reads_its_tickets`, `another_model_of_the_same_cli_resumes_when_the_session_fits`,
  `two_tickets_of_one_project_are_one_project`, `an_episode_is_undone_and_a_later_change_refuses_it`,
  `a_learning_run_is_undone_once_and_a_proven_lesson_asks_first`, the Claude parser's `tool_result` text.
- In the app: the folded tool calls in the Lead's chat (screenshot); the real runs of the new tools follow.
