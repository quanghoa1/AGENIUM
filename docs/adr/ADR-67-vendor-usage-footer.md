# ADR-67: The vendors' subscription usage in the app footer

Status: Accepted (2026-10-25, owner: "show the vendors' usage in the app footer, see how Orca does it"). Learned
from Orca's rules (stablyai/orca: status-bar usage segments, its per-vendor readers); our own code, none of theirs.

## Decision

1. **Footer.** One segment per vendor in use (the agent CLIs installed, the OpenCode Go providers configured):
   a bar of the most used window, then each window's % used (`5h`, `tuần`, `tháng`) and the time to the 5-hour
   reset; amber from 60 %, red from 80 %; `--` without a reading, a warning sign when the last reading failed
   (the earlier numbers stay). A click opens the details (plan, each window with its bar, reset time, notes such as
   Codex reset credits, the error) with a refresh.
2. **Only honest channels.** Nothing poses as another client and no user CLI config is written:
   - **Claude**: the `rate_limit_event` Claude Code prints in its stream-json during an Agenium run
     (`unifiedWindows.five_hour/seven_day.utilization`, `resetsAt`), and "Read now" in the Claude details: one
     tiny Haiku turn of the official CLI without tools (owner: "Haiku is cheap, so acceptable; official is
     better"), in a scratch `CLAUDE_CONFIG_DIR` with a copy of the login like a run. Never on the schedule.
     No polling of Anthropic's API: Orca's call to its OAuth
     usage endpoint sends Claude Code's User-Agent and uses the CLI's login token, and its live feed edits
     `~/.claude/settings.json`; both are out.
   - **Codex**: its own app-server RPC `account/rateLimits/read` (`clientInfo.name = "agenium"`, no model turn), in
     a scratch `CODEX_HOME` with a copy of the login like a run (a refreshed login is handed back the same way).
     Orca's fallback to `chatgpt.com/backend-api/wham/usage` sends Codex's User-Agent and originator: out.
   - **OpenCode Go**: `GET <base>/usage` with the provider's own key (`usage.rolling/weekly/monthly`).
   - **Devin** and **Grok** (added 2026-11-05): listed without numbers. Devin's quota (daily and weekly %, resets)
     is shown only by its terminal UI's `/usage`; over ACP, Agenium's channel, `/usage` is unknown and `/status`
     names the plan only (checked in Agenium's sandbox, no model turn, 3000.11.234), and `devin -p` writes its
     `trusted_workspaces.json` atomically into Devin's own `cli` folder, beside the user's plugins, which runs keep
     out of. Third-party readers call an undocumented user-status service with the CLI's login: out, as above.
     `grok usage` prints one session's tokens and cost, not the plan's quota.
   - **Antigravity**: listed without numbers. agy 1.4.7 has no usage subcommand, and `agy -p /usage` runs as a model
     turn (seen on 2026-10-25), so it is never called.
3. **Refresh.** Every 15 minutes in the desktop app (and at start), plus the footer's refresh; the UI hears
   `agenium://usage` after each reading. Readings live in `<root>/vendor-usage.json`.

## Consequences

- `agenium_llm::vendor_usage` (types, parsers, OpenCode Go reader), `agenium_cli_backend::codex::rate_limits`, `claude::rate_limits` (IPC `read_claude_usage`),
  `Event::Limits` and `CliRun::on_limits`, `agenium_orchestrator::vendor_usage`; desktop IPC `vendor_usage`,
  `refresh_vendor_usage`; UI `UsageFooter`.
- Tests: `vendor_usage::vendor_reports_become_windows`, `claude::parses_the_stream` (rate_limit_event),
  `team.rs::vendor_usage_keeps_the_last_good_reading`.
