# Store parity: SQLite (default) vs SurrealDB/SurrealKV (optional feature)

Measured 2026-10-23 (Asia/Saigon) on the Linux reference box (8 cores, NVMe-class virtual disk, release builds), with
`crates/agenium-store-bench` (`store-bench …`). Raw output: `scale-lab/results_*.txt`, `/tmp/app_*.txt` (copied into
`docs/bench-raw/`). Decision: ADR-49. Scale results beyond 100k facts: `docs/STORE-SCALE.md`.
Anything marked **NOT RUN** was not measured; nothing in this file is estimated.

## 1. Correctness

| Check | SQLite | SurrealKV | Result |
|---|---|---|---|
| Shared `Store`-trait suite (`tests/common`, same tests run against each engine: lifecycle, state machine, idempotent `intent_id`, ready-set, backups full/incremental, migration + rollback, retention planner) | 19 tests | 18 tests | all pass (SQLite has 2 extra: archive/retention and engine-mismatch refusal) |
| SQLite-only: retention/archive seq continuity, persistence + refusal to open the other engine's data | 2 | – | pass |
| Agent-memory DB tests (`tests/agent.rs`: hybrid search, supersede, sidecar crash replay, corrupt/missing sidecar, auto switch) | 9 | – (S1 oracle covers the Surreal reference) | pass |
| **Differential test**: the same seeded random workload (3000 ops × 3 seeds: ~660 creates, ~370 appends incl. idempotent duplicates, ~950 rule-rejected and ~450 legal transitions, ~160 approvals, ~160 merges, create replays) driven through the `Store` traits on both engines; results *and error classes* compared; periodic read-outs (`list_tickets` ×8 statuses, `ready_tickets`, `events_since`, `events_for_ticket`, `last_seq`, `get_ticket_by_key`) | – | – | 4656 / 4660 / 4653 comparisons, **0 mismatches** |
| Event written before state, one transaction, `seq` gap-free, idempotent by `intent_id` | by construction + tests | same | pass on both |

`schemas/sql/*.sql` (generated from the migration DDL) is the primary schema; `schemas/ref/db-*.surql` stays for the optional
engine; `cargo xtask schemas --check` verifies both.

## 2. Crash safety: 20 × `kill -9` (child process writes events, tickets, facts; parent SIGKILLs at a random point)

| | SQLite (WAL, core `synchronous=FULL`, agent `NORMAL`, ANN sidecar active) | SurrealKV (HNSW + BM25 reference) |
|---|---|---|
| Rounds | 20 | 20 |
| Acknowledged events / facts / tickets | 24 138 / 32 220 / 2 424 | 8 901 / 11 884 / 898 |
| Lost acknowledged events / facts / tickets | **0 / 0 / 0** | **0 / 0 / 0** |
| `seq` gaps | 0 | 0 |
| Index problems (exact lookup by vector and by token; ANN/HNSW self-hit miss) | 0 (0/1200 self-hit misses) | 0 (0/1200) |
| Worst reopen time | 1.40 s (incl. sidecar replay) | 0.10 s |

A process kill loses nothing on either engine. Power loss is different: SQLite agent DBs use `synchronous=NORMAL`, so the last
few memories of an agent may be lost (never corruption, never core data: `core.db` is `FULL`). That is a documented
trade-off (ADR-49); it was not tested with real power loss (**NOT RUN**).

## 3. Agent memory: quality and latency (384-d bge-small-en + synthetic, 200 held-out queries, recall@10 vs brute force)

| Facts | Engine / index | Ingest /s | KNN p50 / p95 ms | Recall@10 | Hybrid p50 / p95 ms | Needle (50) | Dir on disk | Peak RSS |
|---|---|---|---|---|---|---|---|---|
| 10k | SQLite exact (vec0) | 17 000 | 5.0 / 5.6 | 1.000 | 6.6 / 14.5 | 50 | 19 MB | 199 MB |
| 10k | SQLite ANN (usearch f16) | (build 0.4 s) | 0.18 / 0.23 | 0.990 | 1.3 / 2.2 | 50 | +9 MB sidecar | |
| 10k | SurrealKV HNSW ef=200 | 1 919 | 4.1 / 4.8 | 0.9985 | 6.6 / 7.8 | 49 | 174 MB | 364 MB |
| 10k | SurrealKV HNSW ef=40 | | 3.9 / 6.6 | 0.961 | | | | |
| 100k | SQLite exact (vec0) | 16 600 | 56 / 62 | 1.000 | 68 / 86 | 50 | 184 MB | 582 MB |
| 100k | SQLite ANN (usearch f16) | build 5.7 s (17k/s) | 0.34 / 0.42 | 0.9885 | 8.3 / 16.1 | 50 | +92 MB sidecar | |
| 100k | SurrealKV HNSW ef=200 | 1 932 | 23.4 / 33.9 | 0.9955 | 14.7 / 41.2 | 49 | 1 262 MB | 2 761 MB |
| 100k | SurrealKV HNSW ef=40 | | 33.8 / 50.5 | 0.9215 | | | | |

SurrealKV beyond 100k facts: **NOT RUN** (ingest 1.9k/s ⇒ 1M facts ≈ 9 min of ingest, ≈ 12 GB on disk by linear extrapolation,
RAM ≈ 18 GB; not attempted on a box with 35 GB of free disk). The macOS phase-3 run measured the same shape at 100k
(SurrealKV 4.6× slower ingest, 7.4× bigger on disk); Linux: 8.6× slower ingest, 6.9× bigger.

Core database (tickets/events/comments): SQLite only was measured at scale (STORE-SCALE.md §5: 1M and 2M events, up to 1M
tickets/comments). The Surreal core store was only checked for correctness (sections 1–2); **latency at 1M rows NOT RUN**.

## 4. Footprint

| | SQLite (default) | + `--features surreal` |
|---|---|---|
| Clean release build, `agenium-store-bench` | **52 s** (8 cores; 373 MB target dir) | **293 s** (5.6×; 1.9 GB target dir) |
| Release binary | **10.1 MB** | **122 MB** (12×) |
| RSS with an open core store (1M events / 100k tickets) | 18 MB at open | not measured for the Surreal core store (**NOT RUN**); agent-DB peaks in §3 |

## 5. Quality gaps vs SurrealDB (what SQLite does *not* give us)

| Gap | Impact | Mitigation |
|---|---|---|
| No live queries / change feeds | not used: the orchestrator reads the event log (`events_since`) | none needed |
| No native graph traversal (`->edge->`) | **ticket/dependency graph** = join tables, computed in `agenium-core` (tiny). **Memory graph** (entities/edges, multi-hop reuse across tickets) is planned in [ADR-51](adr/ADR-51-graph-memory-on-sqlite.md) as relational tables + bounded `WITH RECURSIVE`; feasibility probe in `bench-raw/linux-graph-cte-probe.txt`, real benchmarks planned (STORE-SCALE §8). SurrealDB parity for it is a stretch item | ADR-51 (planned, not implemented) |
| HNSW is not built in; ANN is a usearch sidecar (C++ toolchain at build time) | extra moving part, 1.3 GB RAM at 1M facts | journal-in-transaction consistency + automatic rebuild (ADR-50); exact scan always available |
| Agent DB `synchronous=NORMAL` | power loss may drop the last memories | core stays `FULL`; documented |
| Retention/archive is SQLite-only | Surreal build cannot archive | optional engine, documented |
| `intent_id` idempotency window ends when the event is archived (≥ 100 000 events and ≥ 30 days) | far longer than any retry horizon | documented (ADR-50) |
| Single writer per database file (serialised `BEGIN IMMEDIATE`) | one core writer, one writer per agent DB: matches the design; measured 2.0–4.8k events/s (core), 1.6–2.0k facts/s with the ANN on at 100k–1M facts (3.1k at 10k) | write batching (`insert_facts`) |

Where SQLite is clearly better: ingest 8.6×, disk 6.9×, RAM 4.7× at 100k, startup, KNN latency with the ANN, backups
(online snapshot + zstd), tooling (`sqlite3` CLI, DB Browser), and no live-index corruption class beyond the sidecar.
Where SurrealKV was better: nothing measured except KNN recall at ef=200 (0.9955 vs 0.9885 at 100k) and reopen time after a kill.
