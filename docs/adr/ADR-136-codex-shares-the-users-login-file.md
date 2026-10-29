# ADR-136: Codex runs share the user's own login file

Status: Accepted (2026-10-29). Owner: "ở refresh token, đó là lỗi gì nhỉ … agenium chỉ điều khiển cli thôi mà"; "hãy thử
với codex đi" (Claude Code is left on the copy: the session the owner works in uses its login). Amends ADR-56 (per-run
home, login copy), ADR-134 (logins kept in step) and ADR-135 (login retry).

## Context

- A run's CLI gets its own configuration folder (`CODEX_HOME`) so that none of the user's MCP servers, skills, hooks
  or rules reach it, only Agenium's. The login lived in the user's folder, so it was **copied** in.
- OAuth refresh tokens are single-use. With one copy per run, a refresh in one run left every other copy (and, until
  synced, the user's file) holding a used token; Codex then refused the next refresh ("refresh_token_reused") and asks
  to sign in again. The sync of ADR-134 and the retry of ADR-135 narrowed this, but the copy caused it.
- A tool that runs a CLI in the user's own folder has one login file, and the CLI handles its sessions itself. Codex's
  source (`codex-rs/login/src/auth`): it writes `auth.json` in place (open, truncate, write; no temporary file
  renamed over it), and before it refreshes, and on a 401, it reads `auth.json` from disk again, so a token another
  session refreshed is picked up rather than refreshed twice.

## Decision

1. **A Codex run's `auth.json` is the user's own file**: a hard link in the per-run home (`link_login`), not a copy.
   Every run and the user's own Codex read and write one file, as Codex's sessions do on the user's machine. A hard
   link needs no privilege (unlike a symbolic link on Windows) and resolves inside the sandbox: the contained process
   reaches the file through the link in its writable home, while the file's own folder stays out of its reach
   (`a_hard_link_in_a_writable_home_reaches_its_file`, MXC and bubblewrap).
2. **The per-run home sits on the login's file system** where the temp folder may not: `~/.cache/agenium-cli-homes` on
   Linux and macOS (`/tmp` is often a tmpfs, and a hard link cannot cross file systems); the temp folder on Windows,
   under the user's profile like `.codex`.
3. Where no link can be made (another volume), the copy, its sync and the login retry of ADR-134/135 stay. The run's
   home going removes the link, never the user's file; a kept session never holds it. The usage probe links the same
   way.
4. **Claude Code and Grok keep the copy** (with its 5-second sync and the login retry of ADR-135); the owner asked for
   the link everywhere ("khi dùng nhiều session của các cli khác mà không bị gì thì việc chúng có cơ chế như codex là
   điều hiển nhiên"), so both were read first. They do share one login safely across their own sessions, but through
   the folder, not the file: each writes its login to a temporary file renamed over it (Claude Code 2.1.312: an atomic
   write helper with a staging file and a rename; Grok 1.0.41: `auth.json.tmp` renamed, an in-place write only when the
   disk is full), takes a lock file beside it (`.storage-write`, `auth.json.lock`) and reloads the file when it
   changes. A rename gives the path a new file: a hard link would keep the old one and come apart at the first refresh
   anywhere, the user's own CLI included, which is the copy again. Claude Code also refuses a login that is a
   symbolic link. Claude Code itself, when it copies a login into a temporary configuration folder, drops the refresh
   token from the copy so that the copy can never refresh.
5. agy keeps its login in the OS credential store, which every session shares already; OpenCode gets the provider's
   API key, which does not refresh. Neither has a file to link.

## Consequences

- Several Codex agents on one account refresh as one: no "refresh_token_reused", no sync window. For Claude Code and
  Grok the window stays (5 s, then one retry); a long-lived token that never refreshes (`claude setup-token` with
  `CLAUDE_CODE_OAUTH_TOKEN`, Grok's `XAI_API_KEY`) would close it, at the cost of a step by the user.
- A Codex run writes the user's login file directly (a refreshed token), as the user's own Codex does; a run stopped in
  the middle of that write could leave it cut short, the same risk as the user's own Codex.

## Verified (2026-10-29)

- Real Codex (gpt-6-luna) in Agenium's sandbox on Windows: the work and review runs' `auth.json` was the user's file (2
  names, same file); three runs at once (`hello-file`, `csv-total`, `rename-function`) all passed with the file shared
  by up to four names; the user's login unchanged and whole afterwards, back to one name.
- Tests: `a_runs_login_is_the_users_own_file` (an in-place write reaches the user's file; the home going leaves it),
  `a_hard_link_in_a_writable_home_reaches_its_file` (Windows MXC; Ubuntu bubblewrap with the folders in `/tmp` and in
  `~/.cache`).
- Ubuntu 25.10 VM: `/tmp` is a tmpfs, so a link from it to `~/.codex` fails ("Invalid cross-device link"); from
  `~/.cache` it is made. Codex itself is not installed there: the real runs were on Windows.
