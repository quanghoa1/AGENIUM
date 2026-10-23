# PROGRESS — Agenium M0 (scaffolding), Linux

Working dir: /workspace/agenium-app (local git only, NO push). Spike code (reference only): /workspace/agenium.
Always: `unset CARGO_TARGET_DIR; export PATH=$HOME/.cargo/bin:$PATH`.
Last full verification: 2026-10-23 ~12:12 (Asia/Saigon, UTC+7) on Linux x86_64 (Debian 13, kernel 6.12, rustc 1.99.1).

## M0 acceptance status (IMPLEMENTATION-PLAN.md)
| Item | Status | Evidence / caveats |
|---|---|---|
| M0-1 workspace + CI + cargo-deny + LICENSE/NOTICE | DONE locally; CI NOT RUN | `.github/workflows/{ci,sandbox-pentest,release}.yml` written for Linux + macOS only (macOS `continue-on-error`). Workflows have never executed on GitHub. `cargo-deny` not installed locally (CI job only). |
| M0-2 config | DONE | 18 tests: 3 root-resolution branches, layout creation, config.toml round-trip, `user_language`, `knn_ef`. |
| M0-3 setup wizard + sysreq | DONE (backend + UI, tested in vitest + IPC contract test) | The Tauri window itself was NOT launched visually (no display); see "NOT RUN". |
| M0-4 core | DONE | 16 tests incl. proptests (no illegal transition, Closed terminal, dep_satisfied). |
| M0-5 store | DONE; engine switched to SQLite (ADR-49) | Shared backend-generic suite (19 tests) passes on SQLite (default) and SurrealKV (optional feature `surreal`). RocksDB feature compiles through the same trait; its build/test NOT RUN. See "Store switch" below. |
| M0-6 secrets | DONE | 15 unit + 3 canary tests. Real OS keyring only proven in spike S5, not in this test run (mock/in-memory + hybrid vault tested). |
| M0-7 vcs | DONE | 5 unit + 5 lifecycle + 2 sandboxed-git. Local bare remotes only; real GitHub/GitLab writes NOT RUN. Pre-merge check runs unsandboxed (CheckRunner hook). |
| M0-8 project | DONE | `agenium.toml`, create/open/close, default `<root>/projects` (orchestrator tests). |
| M0-9 bus + UI resync | DONE | 4 bus tests (backfill, lag, no dup); UI single-flight resync (vitest). Real kill/restart mid-burst of the packaged app NOT RUN; covered by store-level test. |
| M0-10 reconcile | DONE | 24 injection cases (8 S4 points x variants), in-process fault injection, not real `kill -9`. Re-ran 3x: stable. |
| M0-11 backup | DONE | zstd JSONL full + differential incremental, retention 24/7/4 + 3 pre-migrate, restore-hash test. |
| Sandbox (spike decisions) | DONE on Linux | bwrap 36/36 + 8 legit; Landlock helper suite passes; fail-closed. Landlock gap: chmod/utime outside roots. ABI<3 only simulated. |
| macOS | EXPERIMENTAL, NOT RUN | Seatbelt v2 code + test written, never executed; owner's 30-case suite not imported. |
| Windows | DEFERRED (ADR-39) | |

## Test counts (final verification 2026-10-23 17:05 Asia/Saigon, `scripts/ci-local.sh --no-tauri`, Linux x86_64)
`cargo test --workspace --exclude agenium-app`: **139 passed, 0 failed, 0 ignored** · `cargo test -p agenium-store --features surrealkv`: **53 passed, 0 failed** · sandbox-attack-tests (bwrap + Landlock, `AGENIUM_REQUIRE_SANDBOX=1`): 8 passed · vitest: 7 passed (3 files) · `cargo fmt --check`, `clippy -D warnings` (default and `--features surrealkv`), `xtask schemas --check`, `pnpm build`: PASS.
NOT RUN in this box: the Tauri crate `agenium-app` (no GTK/WebKit dev packages; `--no-tauri` skips it; it had 4 tests in the earlier full run), macOS and Windows runs.

## Steering applied
Schema FLEXIBLE (patch applied) · knn_ef config (default 200) · keyring per-target features · Linux+macOS only · English-only below Lead + `user_language` (ADR-40) · Embedder trait (ADR-47) · Seatbelt broad-read + deny ~/.ssh + env -i + ulimit (ADR-43) · README macOS notes · Codex vendoring layout (ADR-41, VENDOR.md, sync script, SANDBOX-MAINTENANCE.md, `sandbox-attack-tests`).

## Open items / needs owner confirmation
- ADR-44 git CLI instead of git2/gix · ADR-48 tauri-specta deferred (hand-written TS types + contract test) and xtask as workspace member · ADR-41 Rust files of Codex sandbox are reference-only (internal crate deps) · ADR-47 default embedder unchanged.
- seccomp table is x86_64 only (aarch64 Linux unsupported until added).
- Dev-path ticket transitions use timestamp intent ids (not idempotent across retries) — M1 uses real intents.
- Ticket detail/Agents UI, Monaco/xterm/dnd-kit: M1+.

## Store switch: SQLite primary, SurrealDB optional (owner decision 2026-10-23 12:15, ADR-49/50) — DONE on Linux
Status as of 2026-10-23 17:05 (Asia/Saigon). All committed locally (no push).
- [x] SQLite backend (per-agent DBs, migrations + pre-migrate backup, full/incremental zstd backups, event log with event-before-state + intent idempotency, hybrid RRF over sqlite-vec KNN + FTS5, writer-serialised txns, process lock). `open_store(Engine::DEFAULT = Sqlite)`; SurrealKV behind `--features surrealkv`.
- [x] `VectorIndex`: exact (sqlite-vec) + usearch HNSW sidecar with journal; **auto switch at 100 000 facts (owner-approved, configurable)**; `ef_search` 128, auto-scaled to 256 from 200k vectors (`AnnOptions.scale_ef`, pinned by `BENCH_EF`). Measured (Linux, real embeddings, f16): recall 0.9885 @100k, 0.969 @500k, 0.941 @1M at ef128; 0.974 @1M at ef256; f32 = f16. Structureless random data gives 0.08-0.47 recall for any dtype/ef (matches macOS 0.56-0.80 qualitatively; inference, macOS not re-run).
- [x] Event-log retention (zstd JSONL archives, seq continuity; SQLite only).
- [x] Parity SQLite vs SurrealKV (3 seeds x 3000 ops, 0 mismatches) - docs/STORE-PARITY.md.
- [x] docs/STORE-SCALE.md (Linux 10k-1M, macOS phase-3 table + comparison, quantization, ef study, threshold, concurrency fixes: mmap 1 GiB readers, condvar pool, WAL gate 64 MB). Raw: docs/bench-raw/.
- [x] ADR-50 rewritten, ADR index `docs/adr/README.md`, README, REPO-LAYOUT, `schemas/sql` generated (`xtask schemas --check` OK).
- [x] Local CI: `scripts/ci-local.sh` / `.ps1` (flags `--heavy --quick --no-tauri --skip-ui --skip-sandbox --skip-surreal --fail-fast`); GitHub workflows kept but `workflow_dispatch` only.
- [x] docs/WINDOWS-GUIDE.md, WINDOWS-PROMPT.md, HANDOFF.md (vi). Bench harness runs anywhere via `AGENIUM_BENCH_HOME` (synthetic corpus fallback when the real embeddings are absent; numbers then not comparable).
- NOT RUN / limits: SurrealKV at 500k/1M; power-loss test; macOS re-run after fixes; Windows; Tauri crate rebuild in this box; 2M-event core concurrency numbers are from before the reader-pool fix.
- Deviations needing owner confirmation (also in HANDOFF.md): git CLI vs git2/gix (ADR-44) · tauri-specta deferred (ADR-48) · xtask as workspace member · CI Linux+macOS and manual-only · Codex Rust files reference-only (ADR-41) · default embedder (ADR-47) · SQLite switch (ADR-49) · usearch C++ dependency · retention SQLite-only · ANN threshold 100k · ef auto-scaling to 256 at 200k · mmap/WAL-gate defaults.
Bench harness: `crates/agenium-store-bench` (`store-bench facts|core|crash|parity|stress|surreal-facts`).

## Graph memory design (ADR-51) — documentation only, 2026-10-23 (Asia/Saigon)
- [x] `docs/adr/ADR-51-graph-memory-on-sqlite.md` (Proposed): entity/edge/fact_entity model, scoped lessons, LLM extraction (English), entity resolution, bounded `WITH RECURSIVE`, hybrid vec+BM25+graph RRF + re-rank, reuse flow, forgetting, sharing options (B ticket note / C project pack need owner OK), limits, benchmark plan, acceptance criteria, golden tasks GM-1..7, tests, milestones (M1 hook, M1.5 core, M4.5 signals/packs).
- [x] Draft DDL `schemas/sql/drafts/agent_003_graph_memory.sql` (applies on agent_001+002 in SQLite 3.46; NOT a migration; not touched by `xtask schemas --check`), `schemas/README.md`.
- [x] Example queries self-tested: `docs/bench-raw/graph-queries-selftest.py`; feasibility probe (python, synthetic, 100k/1M edges, hub case): `docs/bench-raw/linux-graph-cte-probe.txt`.
- [x] Corrected "graph not needed" claims (STORE-PARITY §5, ADR-49): ticket graph = join tables; memory graph = planned (ADR-51). STORE-SCALE §8 = benchmark plan.
- NOT RUN: the Rust implementation, the planned benchmark harness, SurrealDB graph parity; Mem0/SurrealDB comparisons in the ADR are from memory (web check was not done).
