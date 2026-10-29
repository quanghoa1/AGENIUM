# ADR-137: Claude Code and Grok runs are lent the login without its refresh token

Status: Accepted (2026-10-29). Owner: "làm vậy khá bất tiện cho người dùng, liệu còn cách khác tốt hơn không" (a
long-lived token or an API key asked of the user; an xAI subscription cannot be turned into an API key). Amends
ADR-134 (logins kept in step), ADR-135 (login retry) and ADR-136 (Claude Code and Grok keep the copy).

## Context

- Claude Code and Grok write their login to a temporary file renamed over it (ADR-136): a hard link cannot share it.
  Each run held a whole copy, refresh token included; refresh tokens are single-use, so the first refresh left every
  other copy, and for up to 5 seconds the user's own CLI, with a used one.
- A run lasts 45 minutes at most. An access token lasts hours (Claude Code's about 8, with its refresh token about a
  month; Grok's several). Most runs never need to refresh at all.
- Claude Code itself, when it copies a login into a temporary configuration folder, drops the refresh token from the
  copy so that the copy cannot refresh.

## Decision

1. **A run is lent the login without its refresh token** (`login_access.rs`) whenever its access token outlives the run
   (its limit plus 15 minutes): it cannot refresh, so it cannot use up the token the user's CLI and the other runs
   share. Claude Code gets its `claudeAiOauth` without `refreshToken` (the tokens of the user's own MCP connectors,
   `mcpOAuth`, stay out too); Grok each sign-in without `refresh_token`.
2. **The lent login follows the user's file**: when the user's file changes (a refresh by the user's CLI or by a run),
   the run's copy is rewritten within 5 seconds, and both CLIs reload a login that changed on disk.
3. **One run at a time refreshes**: when the access token runs out within a run, that run (one per login, in the app)
   gets the whole login and refreshes it for everyone (its new login reaches the user's file as before); other runs
   started meanwhile are lent the login and follow the refreshed one. A lent run whose token ran out with nobody
   refreshing it takes that role after its login error and runs its turn again (ADR-135).
4. The usage probe of Claude Code is lent the login the same way. Codex keeps the shared file (ADR-136); agy and
   OpenCode have nothing to lend (ADR-136).

## Consequences

- Nothing for the user to do: no long-lived token, no API key, the subscription's own login.
- The refresh race is gone but for one case: the one run that refreshes and the user's own CLI refreshing in the same
  5 seconds; the retry of ADR-135 covers it.
- A run no longer carries the user's MCP connector tokens (Claude Code).

## Verified (2026-10-29)

- Test: `a_login_is_lent_without_its_refresh_token_and_one_run_refreshes` (lent, follows the user's refresh, one holder
  near the expiry and the next run lent, the holder's refresh reaching the user's file and the lent copy, no promotion
  with a fresh token, promotion once it ran out).
- Real runs on Windows: Claude Code Haiku (`hello-file`, `csv-total`) and Grok 4.7 (`hello-file`) passed with lent
  logins: no refresh token and no MCP tokens in the run's login file (watched during the runs, key names only); the
  user's Claude Code and Grok login files unchanged.
