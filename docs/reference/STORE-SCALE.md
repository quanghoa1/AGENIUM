# STORE-SCALE — how the SQLite store behaves from 10 k to 1 M+ facts per agent

Status: measured 2026-10-23 (Asia/Saigon). Decisions: ADR-49 (SQLite primary), ADR-50 (vector index, thresholds, retention,
concurrency). Raw outputs: `docs/bench-raw/` (Linux). Parity/correctness: `docs/STORE-PARITY.md`.
Everything marked **NOT RUN** was not measured; nothing here is extrapolated unless it says so.

## 1. Method

* **Machines.** Linux: 8 cores, 8 threads for builds, NVMe-class virtual disk (`/dev/vdb`), Debian 13. macOS: the owner's
  M4 Pro VM (phase-3 steering numbers, section 5 — not re-run by me, reproduced as reported).
* **Data.** 384-d vectors (bge-small-en). *Real*: 24 462 real sentence embeddings (+1 000 held-out queries; 200 used);
  larger sizes are convex mixes of two real vectors + noise (clustered, like real memory). *Random* (`BENCH_RANDOM=1`):
  structureless unit-Gaussian vectors — the worst case for HNSW. Ground truth = exact brute force, recall@10.
  Facts also carry text (25 463 real sentences) for FTS5/BM25; hybrid = RRF of KNN + BM25. A "needle" test inserts a
  unique token whose vector points elsewhere and checks hybrid finds it (50 trials).
* **Harness.** `crates/agenium-store-bench` (release build): `facts N`, `core E T C`, `stress N secs`, `crash sqlite 20`,
  `parity`. Everything goes through the production `AgentDb`/`Store` API (not synthetic SQL). `AGENIUM_BENCH_HOME`
  selects the work dir; without the real corpus files the harness generates a *synthetic stand-in* (numbers then differ).
* **Engines compared.** sqlite-vec `vec0` (float / int8 / bit), usearch HNSW (f32/f16/i8), hnsw_rs, vectorlite
  (evaluated separately), SurrealKV/HNSW (reference backend, to 100 k only).

## 2. Linux results

### 2.1 App level (`facts`, f16 ANN, M=16, ef_add=128, ef_search=128, no rerank)

| Facts | Ingest/s | DB size | Exact KNN p50/p95 ms | Exact BM25 p50 | Exact hybrid p50/p95 ms | ANN build | Sidecar | ANN KNN p50/p95 ms | ANN recall@10 | ANN hybrid p50/p95 ms | Reopen w/ sidecar | Live ingest w/ ANN | 4 readers+1 writer KNN p95 ms | Backup | Peak RSS exact → ANN |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 10 k | 17.0 k | 19 MB | 5.04 / 5.57 | 0.77 | 6.6 / 14.5 | 0.4 s | 9 MB | 0.18 / 0.23 | 0.9895 | 1.28 / 2.18 | 8 ms | 3.1 k/s | 0.30 | — | 199 MB |
| 100 k | 16.6 k | 184 MB | 56 / 62 | 7.5 | 68 / 86 | 5.7 s | 92 MB | 0.34 / 0.42 | 0.9885 | 8.3 / 16.1 | 80 ms | 2.0 k/s | 0.43 | 1.2 s, 184 MB | 183 → 330 MB |
| 500 k | 16.7 k | 918 MB | 323 / 365 | 38.6 | 380 / 436 | 39 s | 458 MB | 0.65 / 1.18 | 0.969 | 38.7 / 80 | 0.5 s | 1.7 k/s | 0.59 | 6.1 s, 792 MB | 769 MB → 1.3 GB (run peak 2.3 GB) |
| 1 M | 16.8 k | 1834 MB | 605 / 693 | 84 | 714 / 817 | 85 s | 917 MB | 0.70 / 1.21 | 0.941 | 81 / 161 | 0.9 s | 1.6 k/s | 0.65 | 10.3 s, 1552 MB | 1.5 GB → 2.6 GB (run peak 4.5 GB) |

Restore takes 0.2–4.2 s plus an ANN rebuild (the sidecar is derived and never backed up): 1.5 s / 8.5 s / 38 s / 86 s at
10 k / 100 k / 500 k / 1 M. Needle test: 50/50 at every size and mode.

**Reading it.** Exact search is linear (≈ 0.56 ms per 1 k vectors on this box). ANN is flat (< 1.5 ms p95). *Hybrid*
becomes **BM25-bound** above ~500 k (FTS5 BM25 alone: 7.5 ms @100 k, 38 ms @500 k, 84 ms @1 M, single thread), so even
with ANN a hybrid query costs ~80 ms p50 at 1 M. That is a known limit (section 7).

### 2.2 Vector engines in isolation (384-d, recall@10, 200 queries)

| Engine / setting | 10 k | 100 k | 500 k | 1 M |
|---|---|---|---|---|
| vec0 float32 exact, p50 ms | 4.95 | 51 | 323 | 664 |
| vec0 **int8**, recall | 0.9265 | 0.9155 | 0.8915 | 0.8725 (no speed-up vs float: 582 ms @1 M) |
| vec0 **bit** (binary), recall | 0.48 | 0.44 | — | — (unusable) |
| usearch **f16**, ef96: size / build / p50-p95 / recall | | | 458 MB / 30.7 s / 0.43-0.64 ms / 0.963 | 917 MB / 73.7 s / 0.44-0.74 ms / 0.932 |
| usearch **f32**, ef96 | | | 842 MB / 61 s / recall 0.9605 | 1685 MB / 108 s / recall 0.9215 |
| usearch **i8**, ef96, recall | | | 0.8625 | 0.8275 |
| hnsw_rs (100 k): build / RSS / recall | | 205 s / 504 MB / 0.9625 | not run (5-6× slower than usearch) | not run |

### 2.3 Quantization evaluation (decision: f32 truth in `fact_vec`, f16 in the ANN index)

| Option | Size vs f32 | Recall@10 (1 M) | Speed | Verdict |
|---|---|---|---|---|
| f32 | 1× | 0.9215 (ef96) / 0.969 (ef256) | baseline | accurate, 2× the RAM of f16 |
| **f16** (default) | 0.5× | 0.932 (ef96) / **0.974 (ef256)** | same or faster build (73 s vs 108 s) | **chosen**: recall equal to f32 within noise |
| i8 (usearch) | 0.25× | 0.8275 | fastest | selectable, not default (recall −10 pts) |
| vec0 int8 | 0.25× on disk | 0.8725 and falling with N | **no faster** than float | rejected |
| vec0 bit | 0.03× | 0.44–0.48 | fast | rejected (unusable) |
| rerank ×4 from `fact_vec` | — | +1–2 pts | **+0.6 ms/query** (vec0 point lookups cost ~0.15 ms each; they load a whole 1024-row chunk) | rejected, default off (`AnnOptions.rerank` kept) |

f32 vs f16 at equal ef (500 k, ef256): recall 0.9885 vs 0.9875, p95 2.06 vs 1.24 ms, 842 vs 458 MB. At 1 M, ef256:
0.969 (f32) vs 0.974 (f16) — the difference is noise; f16 halves memory, file size and build time.

### 2.4 Recall vs `expansion_search` (this is the macOS reconciliation)

Real-embedding data, f16 unless noted (`docs/bench-raw/linux-ef-search-and-random.txt`):

| Facts | ef 128 | ef 256 | ef 256 f32 | ef 256, M32, ef_add 256 | p50/p95 at ef 256 |
|---|---|---|---|---|---|
| 100 k | 0.9885 | — | — | — | — |
| 500 k | 0.969 | **0.9875** | 0.9885 | 0.9960 (build 90 s, 522 MB) | 0.85 / 1.24 ms |
| 1 M | 0.941 | **0.974** | 0.969 | — | 1.41 / 3.24 ms |

**Structureless (random) 384-d data, 100 k vectors** (`BENCH_RANDOM=1`):

| Setting | recall@10 | KNN p50/p95 ms |
|---|---|---|
| f16, ef 128 | 0.145 | 0.59 / 0.77 |
| f16, ef 256 | 0.238 | 1.11 / 1.27 |
| f32, ef 256 | 0.2525 | 1.55 / 1.86 |
| f32, ef 512 | 0.3885 | 2.67 / 3.12 |
| f32, ef 256, M32, ef_add 256 | 0.472 | 2.65 / 3.03 |

Random **500 k**, f16, ef 256: recall@10 **0.0825**, p50/p95 2.0/2.5 ms (`docs/bench-raw/linux-random-500k-rerun.txt`) — recall falls further as N grows.

**Conclusion.** The macOS recall of 0.56 → 0.80 (f32, ef up to 512) is reproduced *qualitatively* by data with no
cluster structure: in 384 dimensions a random point's 10 nearest neighbours are barely closer than the other points
(distance concentration), so no graph index can navigate and recall rises only slowly with ef (here 0.15 → 0.39 on 100 k
random vectors; the macOS run reached 0.56 → 0.80, probably because its data/size/seed differ — I did not have that data set, so the
*numbers* are not identical, only the behaviour is reproduced; this is an inference, not a verified cause). On **real embeddings** (and clustered
mixes of them) HNSW works as designed: 0.97–0.99. Real agent memories are text embeddings, so the real-data
numbers are the relevant ones; structureless data is a pessimistic stress test, and exact search is the fallback anyway.
f32 vs f16 makes no difference to this (0.2525 vs 0.238 at ef 256 — within the spread caused by f16 rounding of nearly
equidistant vectors).

**Decision (implemented, `AnnOptions.scale_ef`):** `expansion_search` stays 128 below 200 k vectors and becomes
`max(configured, 256)` from 200 k upward. Cost: +0.2 ms p50; gain: recall 0.969 → 0.988 at 500 k and 0.941 → 0.974 at
1 M. M=32/ef_add=256 reaches 0.996 at 500 k but doubles build time (90 s vs 32 s) — available as options, not default.

## 3. Threshold decision (owner-approved 2026-10-23 15:46 Asia/Saigon)

`DEFAULT_ANN_THRESHOLD = 100 000` facts per agent (`AgentOptions.ann_threshold`; `IndexMode::{Exact, Ann, Auto}`).

| Platform | Exact KNN p50 at 100 k | Crosses 50 ms at |
|---|---|---|
| Linux (this box) | 56 ms | ≈ 90 k |
| macOS M4 Pro VM | 34 ms (p95 38) | ≈ 150 k |

100 k is the portable switch point: below it exact is correct, has recall 1.0 and costs no RAM/disk; above it a background
HNSW build starts (exact keeps serving) and the index flips when ready. At 100 k the sidecar is 92 MB and builds in 6 s.
The macOS-only 150 k was not used so that Linux does not spend 50–90 ms/query in the 90–150 k window.

## 4. Concurrency (4 readers + 1 writer, batch 20, `stress`)

Findings (all Linux, root causes measured, not guessed):

1. **Reader/writer starvation.** One mutex per pooled read connection was unfair: a maintenance checkpoint starved the
   writer for 99 s. Fixed with a condvar-queued reader pool and by running the passive checkpoint on the writer connection.
2. **FTS5/BM25 readers serialised** with the default page cache (total throughput flat in reader count; 500 k hybrid p95
   1.6–1.8 s — the same shape as the macOS "reader p95 up to 4.3 s"). Page-cache size (16 vs 256 MB) and
   `SQLITE_CONFIG_MEMSTATUS` off made no difference; **`PRAGMA mmap_size` = 1 GiB on read connections fixed it (≈10×)**.
3. **WAL growth.** With always-busy readers a PASSIVE checkpoint never restarts the WAL (+12 MB/s, 535 MB). Fix: when the WAL
   exceeds 64 MB the store pauses new reads, drains in-flight reads, runs `wal_checkpoint(TRUNCATE)` and resumes
   (`Db::checkpoint_exclusive`), plus a passive checkpoint every maintenance tick. Test: `busy_readers_do_not_keep_the_wal_growing`.

| Scenario (4 readers) | Idle p50/p95 ms | With writer p50/p95 ms | Writer facts/s | WAL peak/end |
|---|---|---|---|---|
| 100 k hybrid, *before* fixes | 75 / 146 | 89 / 175 | **25** (99 s stall) | — |
| 100 k hybrid, no mmap (control) | 76 / 150 | 88 / 172 | — | — |
| 100 k hybrid, after | 8.1 / 15 | 9.5 / 22.6 | 1 487 | — / 12 MB |
| 100 k BM25, after | 7.1 / 14 | 8.8 / 23 | 1 655 | 96 / 66 MB |
| 100 k KNN (ANN), after | 0.33 / 0.43 | — | — | — |
| 100 k exact KNN, after | — | 93 / 121 | — | — |
| 500 k hybrid, before fixes | 527 / 1580 | — / 1850 | — | — |
| 500 k hybrid, after | 35.5 / 75.8 | 49.5 / 133.5 | 1 467 | 95 / 38 MB |
| 500 k KNN (ANN), after | 0.37 / 0.52 | — | — | — |

`SQLITE_BUSY` count: 0 on both platforms. The remaining hybrid cost at ≥ 500 k is BM25 itself (section 7).

## 5. macOS phase-3 results (M4 Pro VM, as reported by the owner's runs; not re-run on Linux)

| Facts | Brute-force sqlite-vec KNN p50/p95 ms | usearch f32 HNSW p95 | Build | Index size |
|---|---|---|---|---|
| 10 k | 3 / 5 | 1–2 ms | — | — |
| 100 k | 34 / 38 | 1–2 ms | — | — |
| 500 k | 156 / 169 | 1–2 ms | 353 s | 0.8 GB |
| 1 M | 334 / 360 | 1–2 ms | 897 s | 1.6 GB |

* Exact crosses 50 ms at ≈ 150 k; hybrid adds ≈ 5 %. HNSW f32 recall only 0.56 → 0.80 as ef rises to 512 — explained by
  structureless data (section 2.4). i8: ≈ 3× smaller and faster, recall ≈ 0.5 on that data; binary unusable.
* Concurrency: 0 SQLITE_BUSY, reader p95 up to 4.3 s under continuous writes (reproduced on Linux and fixed by mmap +
  WAL gate; **the fixed build has not yet been re-run on macOS — NOT RUN**).
* Event log: 915 k events/s, `seq > N` 2.9 ms at 1 M, archive of 2.5 M rows → zstd 34.6 MB.
* SurrealKV at 100 k: 4.6× slower ingest and 7.4× bigger on disk (Linux: 8.6× slower ingest (1.9 k vs 16.6 k facts/s), 6.9× bigger (1262 vs 184 MB) — same direction).

**Linux vs macOS.** Brute-force: Linux is ~1.5–2× slower per vector (macOS 34 ms vs Linux 56 ms @100 k, 334 vs 605 ms @1 M);
ANN build is *faster* on this Linux box (85 s vs 897 s @1 M, f16 + 8 threads vs the macOS f32 build, so partly different
settings — same direction, not like-for-like). Both agree on the shape: exact is linear, ANN flat, hybrid BM25-bound.

## 6. Core store (SQLite only; events, tickets, comments)

| | 1 M events + 100 k tickets + 300 k comments | 2 M events + 1 M tickets + 1 M comments |
|---|---|---|
| DB size | 382 MB | 1 384 MB |
| `append_event` p50 | 0.08 ms | 0.10 ms |
| `events_since(500)` / `events_for_ticket` | 0.74 ms / 0.11 ms | 0.92 ms / — |
| `create_and_assign_ticket` p50 | 0.56 ms | — |
| `list_tickets` (InProgress) | 42 ms (3.8 k rows) | 385 ms (38 k rows) |
| `ready_tickets` | 82 ms (8 k rows) | 717 ms (78 k rows) |
| 4 readers + 1 writer | reader p95 0.52 ms, 6.1 k events/s (after fixes; 0.85 ms / 3.1 k before) | reader p95 1.29 ms, 2.0 k events/s (before fixes) |
| Backup full / incremental | 2.0 s, 72 MB / 2.1 s, 2.5 MB | 7.1 s, 232 MB / 19.8 s, 25 MB |
| Archive (zstd JSONL) | 750 k rows → 21 MB, 5.6 s, 4 files, contiguous | 1.5 M rows → 43 MB, 13.6 s |
| Archived read p50 / p95 | 58 / 235 ms | — |
| VACUUM | 0.9 s (387 → 196 MB) | 5.4 s (1 386 → 986 MB) |
| Migrate (with pre-backup) | 1.0 s | 4.5 s |
| Peak RSS | 827 MB | 4.4 GB |

The 2 M row was measured before the concurrency fixes; its 4r+1w line is therefore pessimistic (**not re-run**).
Retention: `archive_events` keeps the newest 100 000 events and anything younger than 30 days live; `seq` is never reused.
**Idempotency window:** `intent_id` uniqueness covers *live* events only; retrying an intent older than the window
(≥ 100 000 events and ≥ 30 days) would create a duplicate — far beyond any orchestrator retry horizon (seconds–minutes).

**Crash safety (20 × `kill -9` at random moments):** SQLite 24 138 events / 32 220 facts / 2 424 tickets acknowledged,
0 lost, 0 gaps, 0/1 200 ANN self-hit misses, worst reopen 1.40 s. SurrealKV (reference): 8 901 / 11 884 / 898, 0 lost.
Real power loss was **not tested** (agent DBs use `synchronous=NORMAL`: a power cut may drop the last few memories, never corrupt).

## 7. Known limits and NOT RUN

* **Hybrid at ≥ 500 k facts is BM25-bound** (80 ms p50 / 160 ms p95 at 1 M even with ANN). Mitigation not implemented:
  drop very-high-document-frequency tokens from the FTS query; shard by time.
* ANN needs a C++ toolchain at build time and ≈ 2.6 GB RAM at 1 M facts (1.3 GB for the index f16 + the page cache).
* Cold open of a 1 M-fact agent without a valid sidecar = 86 s rebuild (search falls back to exact meanwhile).
* SurrealKV beyond 100 k facts: **NOT RUN** (disk/time: the 100 k run already uses 1.26 GB and 2.8 GB RSS).
* macOS: the post-fix concurrency run, the f16 default, `scale_ef` and the 100 k threshold were **not re-run on macOS**.
* Linux arm64 / macOS x64 were not built or measured; real power-loss tests not run; 1 M-fact ANN with continuous
  inserts for hours not run (live-ingest-with-ANN was measured over minutes).
* Heavy benchmarks on the user's machine: `scripts/ci-local.sh --heavy` (uses a *synthetic stand-in corpus* unless the real
  corpus files are placed in `$AGENIUM_BENCH_HOME/data`).

## 8. Graph-memory benchmark (ADR-51)

### 8.1 Rust store, run 2026-10-24 (fresh implementation, M15-10)

`cargo run --release -p agenium-store --example graph_bench -- 100000 1000000` on Windows 11 Pro, 10 cores, warm cache,
1 reader, the store's real `AgentStore::expand` (path-guarded two-arm CTE, `INDEXED BY edge_out/edge_in`, scope guard with
context vendor=netgear, result cap 50, no time budget so raw latency is visible). Synthetic graph: entities = edges / 3,
Zipf-like destinations, 10 % of edges scoped, plus one hub with 2 000 incoming edges. Raw:
`bench-raw/windows-graph-bench-rust-store.txt`.

| 1 M edges, hub cap 200 | p50 | p95 | p99 | ADR-51 target (p95) |
|---|---|---|---|---|
| random seeds, depth 1 | 0.25 ms | 0.39 ms | 0.65 ms | |
| random seeds, depth 2 | 0.42 ms | **0.81 ms** | 0.87 ms | <= 25 ms: **met** |
| random seeds, depth 3 | 1.59 ms | **6.09 ms** | 7.78 ms | <= 50 ms: **met** |
| hub seeds (top 1 % degree) | 0.2 ms | 0.3 ms | - | hubs are not expanded (0 nodes) |

Without the hub cap the path-guarded walk enumerates paths: 1 M edges, hub-biased seeds, depth 2 p95 48.8 ms, depth 3 p95
4.1 s; the 2 000-edge hub as seed costs 4.5 / 82 / 5 222 ms at depth 1 / 2 / 3. At 100 k edges with the cap on, hub-biased
seeds (whose degree is still below 200) reach p95 29 ms at depth 3 (10 % over the 25 ms budget, which then interrupts the
expansion). Consequences kept in the code: hub cap 200, depth <= 3 (retrieval uses 2 hops from query entities and 1 hop
from entities of the best text hits), 25 ms `progress_handler` budget. Not run: cold cache, 4 readers + 1 writer, the
set-based shape, ingest throughput under readers, entity-resolution precision on a labelled set, the >= 100-question
multi-hop QA set and Linux/macOS runs. Golden scenarios GM-1..5/7 pass deterministically
(`crates/agenium-memory/tests/graph_memory.rs`, scope leak 0 on GM-2).

### 8.2 Earlier feasibility probe and plan

What existed before the Rust store: a **feasibility probe** (python `sqlite3` 3.46, Linux, warm cache, synthetic graph, *not* the Rust store) in
`bench-raw/graph-cte-probe.py` → `bench-raw/linux-graph-cte-probe.txt`: at 1 M edges (333 k entities) a bounded two-arm
`WITH RECURSIVE` with `INDEXED BY edge_out/edge_in` takes p50 ≈ 0.03 ms (1 hop), 0.2 ms (2 hops), 1.1–1.7 ms (3 hops), p95 ≤ 16 ms; a hub
(1 761 incoming edges) costs 5.8 / 43 / 319 ms at 1 / 2 / 3 hops (3-hop reach = 82 k nodes) → hub cap + depth ≤ 3 + 25 ms time limit. A union-view
instead of two arms cost ≈ 300 ms/query (full scan per step) → forbidden.

Planned harness `agenium-store-bench graph`: (1) traversal latency at 100 k and 1 M edges, hops 1–3, random vs hub seeds, with/without scope filter,
set-based vs path-guarded, cold vs warm, 1 reader and 4 readers + 1 writer, p50/p95/p99 + `EXPLAIN QUERY PLAN`; (2) extraction/merge ingest throughput and
its effect on reader p95; (3) entity-resolution precision/recall per similarity band (auto-merge precision ≥ 0.98); (4) multi-hop QA golden set (≥ 100 questions:
single-hop / 2-hop / scope-trap): recall@8 and MRR for vec+bm25 vs +graph, scope-leak = 0; (5) stretch: parity vs SurrealDB `->edge->` on the same graph;
(6) consolidation and adjacency-cache build time/RAM. Acceptance criteria and golden tasks: ADR-51 §13.

## 9. Rewritten store, re-measured 2026-10-25 (Windows 11, 10 cores)

Sections 2–7 were measured on the trial implementation (`crates/agenium-store-bench`), which the rewrite did not carry
over. `cargo run --release -p agenium-memory --example memory_bench -- 10000 100000 1000000` measures the rewritten
store through the production API: `AgentStore::{add, knn, keyword, hybrid}` and the full `retrieve::recall` path (vector +
BM25 + graph + scope filter + re-rank), 200 queries of 3–6 words, a 50-trial needle test, reopen, and 4 readers running
hybrid while 1 writer adds facts for 10 s. Data is a synthetic stand-in (10–20-word sentences from a 6 000-word Zipf
vocabulary, 384-d hashing-embedder vectors); exact-KNN latency does not depend on the vector distribution. Ingest is one
transaction per `add`, as the memory pipeline writes. No warm-up pass: the first queries after ingest are measured.

**First run, as found** (milliseconds unless noted):

| Facts | Ingest/s | KNN p50/p95 | BM25 p50/p95 | hybrid p50/p95 | recall p50/p95 | needle | 4R+1W reader p95 |
|---|---|---|---|---|---|---|---|
| 10 k | 1 990 | 4.2 / 7.4 | 4.6 / 8.0 | 9.1 / 13.1 | 10.4 / 15.2 | 50/50 | 142 |
| 100 k | 2 264 | 45 / 68 | 46 / 90 | 90 / 156 | 93 / 164 | 50/50 | 1 028 |
| 1 M | 1 915 | 727 / 859 | 534 / 756 | 1 247 / 1 538 | 1 287 / 1 579 | 50/50 | **49 143** |

Three causes, measured:

1. **No WAL gate on agent databases.** `Db::maintain` (passive checkpoint + exclusive TRUNCATE past 64 MB, §4 fix 3) ran
   only on `core.db`, and nothing called the maintenance tick outside tests — so not even `core.db`, scheduled backups or
   run-log compaction ran in the app. Under writes with busy readers the WAL grew without bound. Fix: `Db::write` checks
   the WAL every 32 writes and runs a bounded WAL gate past the limit (gives up if reads do not drain within 2 s, so a
   caller holding a read cannot deadlock; the pool lock is released before the writer is taken); the maintenance tick
   also covers every open agent database and the desktop runs it every minute. Test: without the gate the WAL peaks at
   16× the limit under busy readers.
2. **Stopwords in fact search.** The FTS query ORed every token, so BM25 touched most of the index. `fts_search_query`
   drops English stopwords (near-zero IDF, so ranking barely changes); entity-name search keeps the full query.
3. **Reopen ≈ 1.1 s was a measurement artifact**: it followed the concurrency phase and included WAL recovery. It is now
   measured before that phase.

**After the fixes:**

| Facts | Ingest/s | KNN p50/p95 | BM25 p50/p95 | hybrid p50/p95 | recall p50/p95 | needle | reopen | 4R+1W reader p95, writer/s |
|---|---|---|---|---|---|---|---|---|
| 10 k | 4 656 | 3.7 / 4.0 | **0.5 / 1.3** | **4.4 / 5.2** | **5.5 / 6.3** | 50/50 | 18 | **104**, 3 242 |
| 100 k | 4 188 | 35 / 37 | **3.6 / 12.2** | **38 / 46** | **41 / 50** | 50/50 | 9 | **454**, 2 844 |
| 1 M | 3 189 | 1 244 / 1 892 | **127 / 394** | 1 302 / 1 834 | 1 346 / 2 282 | 50/50 | 9 | **4 619**, 854 |

Reading it:
- At 10 k and 100 k every query got 2–13× faster; BM25 at 100 k (3.6 ms) is now below the trial's 7.5 ms.
- Above 100 k the cost is **exact KNN**: the ANN index that ADR-50 switches on at 100 k facts (`[memory] ann_threshold`)
  is **not wired in the rewrite**, so search is linear (~35 ms at 100 k, 0.7–1.2 s at 1 M). The 1 M KNN figure moved
  from 727 to 1 244 ms p50 between runs although `knn()` did not change: without a warm-up, the first run read freshly
  written, cache-hot WAL pages, while with the gate those pages are in the main file — treat it as cache-dependent, not
  a regression. Wiring the ANN sidecar is the remaining work.
- 4 readers + 1 writer is far beyond real use (an agent reads at run start and writes a few facts when a ticket closes;
  the shared pool sees reads from many agents and rare writes). The store opens 2 reader connections per agent database,
  so 4 readers queue; at 1 M each query is dominated by the 1.2 s exact KNN.
- Not run on the rewrite yet: macOS, Linux, a warm-cache pass, the graph concurrency/cold cases (§8.1 lists them).
