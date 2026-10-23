# ADR-49: SQLite is the primary store engine; SurrealDB becomes an optional reference backend

- **Status:** Accepted (owner decision, 2026-10-23 12:15 Asia/Saigon). **Supersedes ADR-1 (Database) of the design pack** — that ADR's "Plan B" (SQLite + `sqlite-vec` + FTS5 + `tokio::broadcast`) is now the main line. Amends the storage parts of ADR-2, ADR-3a (realtime), ADR-7, ADR-22 and ADR-23. Reconciles ADR-45 and ADR-46 (see below).
- **Date:** 2026-10-23 (Asia/Saigon)

## Context

The pack chose embedded SurrealDB 3.x (SurrealKV) subject to spike S1 and kept SQLite as Plan B. After S1 (which passed, with caveats: 213 MB for 10k vectors + 66k rows, ~0.8 s open, a heavy dependency tree and a young storage engine) the owner chose SQLite as the primary engine. SQLite is the most widely deployed, best-understood embedded database; it gives a single-file-per-database model that is trivial to back up, inspect and repair, a much smaller footprint (S1: 213 MB SurrealKV vs 16.7 MB SQLite for the same 10k-vector set), and a far smaller build.

## Decision

1. **Engine:** `rusqlite` (bundled SQLite 3.4x, `backup`, `blob`, `functions` features) + `sqlite-vec` (pinned `=0.1.9`, statically linked, registered with `sqlite3_auto_extension`) + FTS5 (`porter unicode61`). WAL journal, `STRICT` tables, `json_valid` CHECKs, foreign keys on.
2. **Layout (replaces `<root>/db` SurrealKV directory):** `<root>/db/core.db` (projects, tickets, comments, events, runs, …) and one `<root>/db/agents/<agent_id>.db` per agent (facts, `fact_vec` vec0 table, `fact_fts` FTS5 table, journal). Absolute memory privacy per agent (ADR-2) is now also *file-level* isolation; deleting an agent deletes one file (+ `.ann`, `.ann.json` sidecars).
3. **Concurrency:** one serialized writer connection per database (`BEGIN IMMEDIATE`, busy-retry with jitter) + 4 reader connections (WAL snapshot reads). Process exclusivity by `flock` on `<db>.lock` with a 1.5 s wait (a just-restarted process does not fail on the previous owner's lock). `core.db` uses `synchronous=FULL` (the event log is the source of truth); agent DBs use `synchronous=NORMAL` (a power loss may drop the last few memories but can never corrupt; a process crash loses nothing acknowledged — verified by the 20× `kill -9` test).
4. **Migrations:** schema version in `PRAGMA user_version`; each migration runs in **one transaction** (SQLite DDL is transactional, so a failed migration rolls back by itself) and a `pre-migrate` backup is still written first and kept (3 newest). A database newer than the binary is refused (`TooNew`).
5. **Event-before-state / idempotency (AGENT-BRIEF invariants) are unchanged:** the event row and the state change are one `BEGIN IMMEDIATE` transaction; `seq` comes from a `counter` row inside the same transaction (gap-free), idempotent by `intent_id` (UNIQUE).
6. **Trait boundary:** `agenium_store::Store` (= `ProjectRepo + TicketRepo + WorkspaceRepo + EventLog + backup/migrate`). `open_store(Engine, root)` returns `SqliteStore` by default; the SurrealDB implementation is the same trait behind cargo features `surreal` / `surrealkv` / `rocksdb` (off by default, not in release builds). Both pass the same shared backend-generic test-suite and the differential test in `docs/STORE-PARITY.md`.
7. **Realtime:** `tokio::broadcast` bus fed by the store's `EventSink` (ADR-45 already did this; SurrealDB live queries were never used).
8. **Schemas:** the SQL DDL under `crates/agenium-store/schema/sqlite/` is canonical and mirrored by `cargo xtask schemas` into `schemas/sql/` (checked in CI). The SurrealQL files remain only as the reference backend's schema (`schemas/ref/`).

## Reconciliation with earlier ADRs

- **ADR-45 (IntentJournal / EventSink in core):** unchanged. The note about the SurrealKV file lock and the sink reference cycle applies equally to the SQLite `Db` (the `flock` is released in `Drop`); the same `AppService::drop` clears the sink.
- **ADR-46 (backups & migrations):** retention (24 hourly / 7 daily / 4 weekly + 3 pre-migrate) and the full/incremental scheme are unchanged. Differences: (a) on SQLite a **Full** backup is a consistent *snapshot image* taken with the SQLite online-backup API (zstd-compressed, sha256 + per-table content hash), not a JSONL export; **Incremental** backups stay JSONL rows changed since the base (the `cuts` map); (b) migrations *are* transactional on SQLite, so the "restore pre-migrate backup on failure" path is only a safety net (still tested); (c) the engine marker file is kept (`core.db` vs a SurrealKV directory cannot be confused).
- **ADR-23 (backup/cleanup):** agent databases support `backup_zstd` / `restore_zstd` (the ANN sidecar is derived data and is never backed up; it is rebuilt).
- **ADR-22 (crash consistency):** WAL + `synchronous=FULL` on core; ANN sidecar consistency is journal-based (ADR-50).

## Consequences

- Linux/macOS builds pull a C compiler (bundled SQLite, sqlite-vec) and a C++ compiler (usearch, ADR-50). No Windows target (ADR-39).
- Quality differences against SurrealDB are tracked in `docs/STORE-PARITY.md` (retrieval recall parity, no live queries, no native graph traversal operators — the *ticket* graph is join tables and needs none; the planned *memory* graph is relational tables + recursive CTEs, see ADR-51).
- Event-log retention (ADR-50) exists only for SQLite.
- Changing back (or adding Postgres, etc.) means implementing `Store` + running the shared suite and the differential test.
