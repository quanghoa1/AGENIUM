# ADR-46: Backups: full + differential incremental; migrations are non-transactional with a backup safety net

> **Amended by ADR-49** (SQLite primary): on SQLite a Full backup is a consistent snapshot image and migrations are transactional; the rest stands.

- **Status:** Proposed (implements ADR-23)
- **Date:** 2026-10-23 (Asia/Saigon)
- **Context / decision / consequences:**

- zstd JSONL exports with sha256; hourly incremental (rows changed since the base full), daily full; retention 24 hourly / 7 daily / 4 weekly plus the newest 3 pre-migrate backups.
- Migrations on SurrealDB are not rolled back by the DB on failure, so a pre-migrate backup is mandatory and `migrate` restores it on failure (tested). Restore into a fresh root matches content hashes (M0-11).
- Engine marker file prevents opening a SurrealKV directory with RocksDB and vice versa.
