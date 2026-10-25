# ADR-76: Run-log compaction after 90 days

Status: Accepted (2026-10-25, M5).

## Decision

A run's structured log (LLM calls, tool calls, progress) is diagnostic and grows `core.db`. On the maintenance tick,
runs that ended more than `[backup] run_log_keep_days` ago (default 90, 0 = never) have their logs compacted: the
bulky rows are deleted, the rows the audit trail needs — refused protocol calls (`kind = "tool"`, `ok = false`) — are
kept, and one `compacted` marker row records how many were removed. A run is compacted once (the marker stops a
second pass). Everything else (events, tickets, comments, backups) is unchanged, so the audit, run list and ticket
history keep working; only per-step run diagnostics past the window are dropped.

## Consequences

- `agenium_store::core::runs::compact_logs(cutoff)`; `[backup] run_log_keep_days`; `maintenance_tick` reports
  `logs_compacted`.
- Test: `core.rs::old_run_logs_are_compacted_keeping_refused_calls`.
