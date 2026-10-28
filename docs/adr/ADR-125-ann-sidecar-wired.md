# ADR-125: The ANN sidecar is wired (ADR-50 implemented): tombstones, compaction, build and catch-up rules

Status: Accepted (2026-10-28; owner: "tiếp tục với phần memory", after the sidecar had been proposed for deferral).
Implements ADR-50 §1–5 in the rewritten store; amends it where measurement on this implementation disagreed.

## Context

ADR-50 decided an exact sqlite-vec index below 100 000 facts per agent and a usearch HNSW sidecar from there, with a
journal written in the same transaction as every vector change so the sidecar "never serves wrong results". The
rewrite kept the journal (`ann_journal`) but not the sidecar: past 100 k facts search was linear (35 ms at 100 k,
0.7–1.2 s at 1 M, `STORE-SCALE.md` §9). Wiring it on this code base surfaced three behaviours that ADR-50 did not
cover.

## Decision

1. **Module `agenium_store::ann`**, behind `AgentStore` (no API change for callers): `VectorIndex` with `Exact`
   (sqlite-vec) and `Sidecar` (usearch 2.26.5, pinned; f16, M 16, ef_add 128, ef_search 128 → 256 from 200 000
   vectors, rerank off — all ADR-50 defaults). `AgentStore::open_with(…, AgentOptions)` takes `IndexMode::{Exact, Ann,
   Auto}` and `ann_threshold`; the orchestrator passes `[memory] ann_threshold`. Files: `<db>.ann` + `<db>.ann.json`,
   removed with the agent.
2. **Deletes are tombstones, not `remove`.** usearch reuses a removed key's slot for the next add, which rewires the
   old node's neighbours to an unrelated vector. Measured: after 100 removes + 100 adds on 700 vectors, an untouched
   fact could no longer find itself. A retired or superseded fact stays in the graph (it still helps navigation) and
   is filtered out of every result (`filtered_search`); the tombstone set is saved in the meta file.
3. **Compaction.** When tombstones plus in-place replacements (a re-embedded fact) exceed 10 % of the live vectors
   (and 64), a fresh build runs in the background while the old sidecar keeps serving, then replaces it. A saved
   sidecar that is due for compaction is rebuilt on load instead of used.
4. **Consistency, as ADR-50 §5, made exact for this code:**
   - Writes apply to the active sidecar before `add`/`supersede`/`retire`/`reindex` return (read-your-writes; tested
     with readers running throughout: no fact is served after its retire returned).
   - Replay does not trust the journal's order: every fact touched past `applied_seq` is set to what `fact_vec` holds
     now, so replay is idempotent. A key already holding the same direction is left alone (a build scans facts added
     after its start, which the catch-up sees again; a remove + add would rewire the node and count as churn).
   - On open: meta version, database identity (`agent_meta.ann_db_id`), dimension, dtype, connectivity, file length
     and crc32; then, in one read snapshot, the journal must still hold every entry past `applied_seq` (`applied_seq`
     ≤ the journal's high-water mark and no gap), the replay runs, and `(count, sum(id))` must equal `fact_vec`'s.
     Any failure records why (`AnnStatus.last_discard`) and rebuilds; a restored older database or another agent's
     copied sidecar is refused this way (tested).
   - The journal is trimmed only up to what a saved sidecar covers and never past a running build's starting point;
     with no sidecar at all (below the threshold) the maintenance tick empties it, so it no longer grows for the life
     of an agent.
5. **Builds** read `fact_vec` through vec0's shadow tables in short read transactions (8 192 rows each; vec0 itself
   can only scan everything in one statement or look rows up one by one, each loading a whole 1 024-row chunk). This
   ties the build to sqlite-vec 0.1.9's layout, which is pinned (`=0.1.9`); a layout change fails loudly (`Corrupt`)
   and the store falls back to exact. Inserts are parallel (all cores), the first 4 096 serial.
6. **Recall, not exactness, from a parallel build.** On tight synthetic clusters a 4-thread build misses ~0.04 % of
   self-hits (a serial one none). ADR-50's build times depend on parallel inserts, so builds stay parallel; tests
   hold the correctness invariant strictly (never a fact that is not live) and recall to ≤ 0.5 % self-hit misses.
   The crash test (20 hard kills during adds/retires with saves every 97 changes) had 0 self-hit misses.
7. **Auto switch without a periodic count.** The vector count can have grown by at most the journal entries since it
   was last counted, so the threshold is checked on every write with one O(1) read and counted only when that bound
   reaches it: the write that reaches the threshold starts the build.

## Consequences

- A C++17 toolchain is needed to build the store (MSYS2 gcc on Windows-GNU, verified; MSVC not tried).
- Each vector write costs a journal read and a vector lookup while a sidecar is active (≈ 0.8 ms per add at 10 k
  facts: ~1 250 adds/s against ~4 000 exact-only) — memory writes a few facts per closed ticket, so this is accepted.
- Measured at 100 000 facts with bge-small embeddings (`STORE-SCALE.md` §13): KNN 32.5 → 0.70 ms p50, hybrid
  38.1 → 4.1 ms p50, build 6.8 s, sidecar 92 MB, reopen with the sidecar 99 ms. **recall@10 0.967**, below ADR-50's
  0.9885 (different, synthetic corpus; parallel build); ef_search 256 at 100 k and a serial build are not measured yet.
