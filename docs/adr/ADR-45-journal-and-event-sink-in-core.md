# ADR-45: IntentJournal and EventSink live in agenium-core

- **Status:** Proposed (internal design, no pack decision changed)
- **Date:** 2026-10-23 (Asia/Saigon)
- **Context / decision / consequences:**

- `IntentJournal` (append/since/by_intent/last_seq) and `EventSink` are traits in `agenium-core` so that `agenium-vcs` and `agenium-bus` do not depend on `agenium-store` and the store does not depend on the bus.
- The store calls the sink inline after each *newly committed* event (never for idempotent replays). The bus is `tokio::broadcast`; each subscriber tracks its last seq and reconciles from the log on gap/lag/start, giving exactly-once in-order delivery (spike S1/S7: live queries do not replay).
- Gotcha handled in `AppService`: store -> sink(bus) -> journal(store) is a reference cycle; `Drop` clears the sink so the SurrealKV file lock is released.
