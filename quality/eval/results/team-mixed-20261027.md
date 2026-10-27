# Mixed team, real models: Claude Lead with Codex, Grok, Gemini and DeepSeek workers (2026-10-27)

Release check of the orchestration on real models after the 2026-10-26/27 changes (cancel, memory, CLI detection,
custom endpoints, `view_image`). One run, through the headless CLI on a test root (the same runtime the desktop app
drives), Windows 11, MXC sandbox on, approval policy `sandboxed`, Jev judge on.

## Team

| Agent | Role | Backend | Model, effort |
|---|---|---|---|
| rc-lead | coding-lead (Lead) | Claude Code CLI | `claude-opus-5-5`, low |
| rc-planner | coding-planner | API (OpenCode Go) | `deepseek-v4.1-flash`, low |
| rc-backend | coding-backend | Codex CLI | `gpt-6-luna`, low |
| rc-frontend | coding-frontend | Grok CLI | `grok-4.7`, low |
| rc-tester | api-tester | Antigravity CLI (agy) | `gemini-3.8-flash-low` |
| rc-writer | technical-writer | API (OpenCode Go) | `deepseek-v4.1-flash`, low |

## Request (Vietnamese, as a user would type it)

> Build a small "Quote of the day" web app: a Python stdlib server (`GET /api/quote` returns a random quote from
> `quotes.json`, at least 10 Vietnamese quotes; `GET /` serves the page), an `index.html` + JS page with a "Câu khác"
> button, unittest tests for the API, and a README on running the app and the tests. Split the work across the team.

## What happened

1. Lead (chat, 25 s): spec v1 with 4 requirements and acceptance criteria, out-of-scope list, proposed split. The user
   approved it.
2. Lead (chat, 55 s): plan v1 with 4 items and 4 tickets, dependencies RC-2/RC-3 → RC-1 and RC-4 → RC-1..3 (the planner
   was skipped here: finding 3).
3. `agenium run`: RC-1 (Codex) → review → merged; RC-2 (Grok) and RC-3 (agy) in parallel once RC-1 merged; RC-3 parked
   on a permission request (finding 1), approved by the user; RC-3 → review → merged; RC-4 (DeepSeek) after all three →
   review → merged. **4/4 tickets Closed and merged, plan 4/4 done.** The delivered app's tests pass
   (`python -m unittest`: 2 tests OK).

| Run | Agent | Steps | Tokens in / out | Outcome |
|---|---|---|---|---|
| chat: spec | rc-lead (Claude) | 2 | 54,048 / 1,710 | spec v1 proposed |
| chat: plan | rc-lead | 10 | 161,371 / 4,352 | plan v1, 4 tickets assigned |
| RC-1 work | rc-backend (Codex) | 7 | 167,640 / 3,313 | Completed |
| RC-1 review | rc-lead | 4 | 96,876 / 1,515 | approved, merged |
| RC-2 work | rc-frontend (Grok) | 14 | 208,455 / 3,190 | Completed |
| RC-3 work | rc-tester (agy) | 20 | 140,028 / 806 | Parked: permission for `python -m unittest` |
| RC-2 review | rc-lead | 3 | 74,390 / 812 | approved, merged |
| RC-3 work (after the fix) | rc-tester | 17 | 224,587 / 2,504 | Completed |
| RC-3 review | rc-lead | 4 | 95,764 / 1,219 | approved, merged |
| RC-4 work | rc-writer (DeepSeek) | 32 | 576,105 / 13,374 | Completed (~$0.014) |
| RC-4 review | rc-lead | 5 | 101,906 / 1,163 | approved, merged |

Total ~1.9 M tokens in (mostly cache reads) / 34 k out; 13 min wall clock, of which ~3.5 min RC-3 waited for the user.
The CLIs run on the user's subscriptions; the API cost was $0.014.

## Findings and what was changed

1. **Vendor CLIs that run commands on the host could not test their work.** Claude, Grok and agy are checked through
   their hook: each of their shell commands needs the allowlist or the user (only Codex runs its commands in a
   sandbox). Without someone at the terminal every command was refused, even `Get-ChildItem`: the Lead reviewed by
   reading only ("the shell sandbox denied my live smoke test"), Grok checked its page by reading, agy parked for
   permission. **Fixed:** where a sandbox works, these CLIs also get Agenium's `shell` tool over MCP, which runs in the
   sandbox (worktree writable) without asking; their instructions say so, and a refused host command points to it.
   After the fix agy ran 9 commands through it (its tests among them), and the Lead ran the code before approving
   RC-3 and RC-4.
2. **Generated files reached `main`.** Running the tests created `tests/__pycache__/*.pyc`, which the auto-commit
   took; later a ticket's own test run modified those tracked files and failed its write-scope check, and the writer
   spent steps restoring them. **Fixed:** Agenium keeps tool caches and installed packages (`__pycache__/`, `*.pyc`,
   `.pytest_cache/`, `node_modules/`, `.venv/` …) out of commits through `.git/info/exclude` (local; the project's
   `.gitignore` is untouched), added on project creation and before each worktree.
3. **The planner was bypassed.** Asked in chat to plan right after the spec was approved, the Lead created the plan
   itself; the team's planner never ran (ADR-59: it drafts plan v1, the Lead creates the plan from the draft).
   **Fixed:** `create_plan` waits for the planner's draft while the planner has not given up.
4. **`agenium run` listed stale "needs you" lines** (other projects' items, a permission already answered).
   **Fixed:** it lists what still waits for the user in this project.
5. Noted, not changed: on Windows the merged ticket's worktree could not be removed at once (a file still in use); the
   journaled removal is retried and succeeded.

## Jev (judge) in this run

Review assist ran before each review (one network failure, review went on); on RC-1 it marked the response-shape
criterion *unclear (68 %)*, which the Lead called out as the one to confirm by the test ticket. Memory gate ran after
each ticket and never skipped (every ticket of a new project was worth remembering). Details: `docs/JEV.md`.
