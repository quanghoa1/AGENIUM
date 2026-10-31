# ADR-46: Backups: full + differential incremental; migrations are non-transactional with a backup safety net

> **Amended by ADR-49** (SQLite primary): on SQLite a Full backup is a consistent snapshot image and migrations are transactional; the rest stands.

- **Status:** Proposed (implements ADR-23)
- **Date:** 2026-10-23 (Asia/Saigon)
- **Context / decision / consequences:**

- zstd JSONL exports with sha256; hourly incremental (rows changed since the base full), daily full; retention 24 hourly / 7 daily / 4 weekly plus the newest 3 pre-migrate backups.
- Migrations on SurrealDB are not rolled back by the DB on failure, so a pre-migrate backup is mandatory and `migrate` restores it on failure (tested). Restore into a fresh root matches content hashes (M0-11).
- Engine marker file prevents opening a SurrealKV directory with RocksDB and vice versa.

## Amendment (2026-10-31, code audit H3, M8, L6, L7)

- An incremental set also holds a snapshot of every agent database (`agents_in_full` in its manifest): before, it held
  only `core`, and restoring the newest set brought agent memory back from the base full set, up to a day old.
- An incremental is only applied on the base it was cut from: maintenance takes a full set as soon as `core`'s schema
  version moved, and a restore across schema versions is refused with a plain error.
- A folder that is not a whole set (a dot folder, a `.partial` left by a crash, a manifest id not its folder) is not
  listed; `.partial` folders older than a day are removed. Pre-migrate snapshots are kept by version number, then time.
