# ADR-134: A vendor CLI resumes its own session of a ticket; one CLI account in several runs at once

Status: Accepted (2026-10-29). Owner: "tất cả cli có resume session đang chạy chứ nhỉ? để khỏi mất context đang làm việc,
khi qua ticket mới thì mới chạy 1 session mới"; "1 tài khoản cli có thể mở nhiều session cùng lúc để làm việc mà";
"nếu đổi vendor AI thì đưa lại cuộc hội thoại cho vendor tiếp nhận và tiếp tục làm việc được không". Amends ADR-56
(per-run home, login copy) and ADR-60 (continuation runs).

## Context

- **A ticket often takes several runs of its agent**: a question answered, a review reopened, a quota used up, the
  user's stop. Each run of a vendor CLI had its own per-run home, where the CLI keeps its session, and the home went
  with the run (Claude Code was even told not to keep one, `--no-session-persistence`). The next run started a new
  session shown the conversation so far as text: the CLI's own turns, tool results and reasoning were gone.
- **One CLI account runs several sessions at once** (several agents on one Claude or Codex login). Each run had a copy
  of the user's login; a login the CLI refreshed went back to the user's file only when the run ended. Refresh tokens
  are single-use: a run started meanwhile held the old one and failed at its next refresh.

## Decision

1. **A ticket's CLI session is kept between its runs** (`run/session.rs`), per agent and ticket, under the agent's
   folder (`cli-sessions/<ticket>`): the run's home without the login copy (and without what holds the old home's
   path or can be fetched again: Codex's SQLite databases, with which it refused the resume, "no rollout found";
   OpenCode's cache), the CLI's session id, and how many messages of the ticket's conversation it holds. OpenCode's
   provider key reaches its server through the environment and is in no kept file.
2. **The next run of the ticket resumes it** when it is the same CLI and the conversation is the one the session holds
   (no run of an API model or another CLI went on in between): the home comes back before the run's own files are
   written, the CLI resumes its session (Claude Code and Grok `--resume <id>`, Codex `exec resume <id>`, agy
   `--conversation <id>`, OpenCode's server prompting the kept session instead of a new one) and is sent only what is
   new (the user's answer, the reviewer's findings). The run log says so ("the CLI's session of the
   ticket's earlier run goes on"). A resume the CLI refuses starts again, once, from the conversation as text, and the
   log says why. A new ticket is a new session; a ticket's kept session is removed after 14 days without a run.
3. **Every driven CLI resumes** (`Driver::resumes_across_runs`); a driver that could not would get the conversation as
   text, as all did before.
4. **Another vendor gets the conversation.** When the user gives the agent another model mid-ticket (ADR-132), the next
   run is a continuation: the earlier runs' messages go to the new model, as messages to an API model or as a vendor
   CLI's first prompt. The new vendor's own session starts there and is kept from then on.
5. **Logins stay the same across runs and the user's own CLI** (`sync_login`, every 5 s while a run goes on and once
   after it): a login a run's CLI refreshed reaches the user's file at once; a login refreshed elsewhere (another run,
   the user's own CLI) reaches every run's copy that has not refreshed its own. Changed on both sides, the user's
   wins. A login written halfway moves nowhere; each write is a whole file renamed into place.

## Consequences

- A question answered or a review reopened costs no context: the CLI goes on in its own session, with its own turns,
  tool results and reasoning, and is not sent the conversation again.
- Kept sessions hold the CLI's transcripts (what the model read and wrote in the worktree), not the login; they are
  in the agent's folder of the root, like its run logs.
- Not done: a refresh at the same instant in two runs (within 5 s) can still use one refresh token twice, and the
  later run fails once (its next run starts from the refreshed login); a closed ticket's kept session stays until the
  14-day sweep.

## Verified (2026-10-29)

Tests: `a_session_is_kept_without_the_login_and_resumed_only_whole` (session.rs), `logins_stay_the_same_across_runs_and_the_users_cli`
(driver.rs).

Real CLIs: `agenium-eval --task ask-first` (the agent reads a code word, asks the user where to write it, and is
answered without the code word, which the question must not say either; the ticket's second run must go on from the
first): Claude Code (Haiku), Codex (gpt-6-luna), Grok (grok-4.7), agy (Gemini 3.8 Flash) and OpenCode (DeepSeek v4.1
Flash through OpenCode Go) each resumed its session and wrote the code word without reading the file again. Codex first
failed ("no rollout found for thread id"): its databases were then left out of the kept session. A scan of OpenCode's
kept session during the run found the provider key in none of its 30 files. See [VERIFICATION](../VERIFICATION.md).
