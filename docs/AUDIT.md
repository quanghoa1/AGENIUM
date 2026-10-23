# AUDIT — final consistency audit of the Agenium pack

Run: 2026-10-23 (Asia/Saigon), after the graph-memory docs (ADR-51). Repo: `agenium-app` local git, HEAD `83804be`, nothing pushed. Legend: **PASS** = checked, nothing to change · **FIXED** = problem found and corrected · **OPEN** = not fixed or not verifiable here (stated honestly).

## Summary
| Item | Result |
|---|---|
| (a) docs reflect the latest decisions | **FIXED** (several stale spots), 2 notes OPEN |
| (b) leftover contradictions (grep) | **FIXED** |
| (c) ADR numbering, index, links | **FIXED** (53 broken `#adr-N` anchors) → PASS |
| (d) numbers vs raw results | **PASS** with 2 OPEN (build time / binary size have no raw file; macOS = owner-reported) |
| (e) schemas in sync, `xtask schemas --check` | **PASS** (+ FIXED: Vietnamese tool-schema text) |
| (f) documented commands really work | **PASS** with 1 FIXED doc note, 3 NOT RUN |
| (g) architecture PDF renders incl. Mermaid | **FIXED** (Mermaid was not rendered before) |
| (h) pack README index = zip contents, zip integrity | **PASS** (checked by script at build time) |

## (a) Latest decisions present everywhere
| Decision | Where checked | Result |
|---|---|---|
| SQLite primary, SurrealDB optional (`--features surrealkv`) | arch doc, DECISIONS, PLAN, REPO-LAYOUT, AGENT-BRIEF, README, repo README/ADR-49/50, STORE-* | **FIXED**: stale `RELATE` / `HNSW + BM25` code comments in the arch doc (MemoryStore, assign/comment), plan tasks M0-5 / M15-1 (pointed at `db-*.surql`), `README.md` schema rows now say "reference for the optional backend". Sections that describe SurrealDB (arch "SurrealDB embedded") carry the `[CẬP NHẬT ADR-49]` banner. |
| Linux + macOS first release, Windows deferred | arch doc (rows 5, 9, 12, 29, 3a, shell desktop, wizard, sandbox section, roadmap rows M0-spike/M2/M5, spike table), DECISIONS (ADR 3a, 5, 9, 12, 20, 29, 32), PLAN (SP-1/6, M0-1, M2, M5), M0-SPIKE-CHECKLIST, AGENT-BRIEF | **FIXED**: ~25 places still said Windows/"3 OS"; each now has a `[CẬP NHẬT ADR-39]` note (text kept, not deleted, per the pack's convention); M2 renamed "Sandbox Linux + macOS"; AGENT-BRIEF "CI 3 OS" → local check Linux/macOS; M0-SPIKE-CHECKLIST got a "historical" banner. |
| English-only below Lead + `user_language` (ADR-40) | AGENT-BRIEF, arch, DECISIONS ADR-2c/40, tool schemas, templates, ADR-51 | **FIXED**: 28 tool/entity JSON-schema `description`s were Vietnamese (they are shown to models) → English, in both `schemas/ref/` and the pack `SCHEMAS/`. `templates/*.yaml`: no Vietnamese. `language.rs` and `vi.json` contain Vietnamese on purpose (language names / UI strings). The arch doc and DECISIONS are Vietnamese design documents for the owner, not below-Lead runtime artifacts. |
| No budget | grep `budget` | **PASS**: only decision 3k and the `thinkingBudget` provider mapping remain. (My own ADR-51 wording "25 ms budget" was changed to "time limit" to avoid confusion.) |
| No `Merging` state; `close_ticket` by Lead/User | AGENT-BRIEF, arch §state machine, tool schemas | **PASS** (only negations: "không có trạng thái Merging") |
| Removed `split_ticket` / `request_reassign` | all docs, tool catalogue (24 tools, index = files) | **PASS**: mentioned only as removed / replaced by `revise_plan`; no schema for them |
| 4 templates / 6 vendors | `templates/` (coding, pentest, research, content), arch row 13, DECISIONS ADR-13 | **PASS** |
| Only the user grants always-allow; Lead per run | `approve_permission` / `request_permission` schemas, AGENT-BRIEF | **PASS** |
| Local-only CI | README, PLAN M0-1, arch update table, `.github/workflows` | **PASS** (workflows are `workflow_dispatch` only; `scripts/ci-local.sh/.ps1`) |
| Graph memory ADR | ADR-51 + index, DECISIONS (row + section + ADR-2 note), arch (update table, new subsection, TOC), PLAN (M1-11, M15-7…11, M4.5), AGENT-BRIEF, REPO-LAYOUT, STORE-SCALE §8, STORE-PARITY §5, ADR-49 | **PASS** (new) |
| ADR-51 vs decision 2 (private memory) | ADR-51 §10 | **OPEN (owner decision)**: sharing options B (Lead `knowledge_note` via ticket) and C (project knowledge pack) touch "memory is private"; ADR-51 is **Proposed** until the owner approves. |
| ADR-51 and the closed tool catalogue | ADR-51 §8/§14 | **FIXED**: first draft invented `recall`/`remember` tools; removed (no new protocol tools; harness `MemoryRetriever`). |

## (b) Contradiction greps
- **SurrealDB as primary**: remaining mentions are inside struck-through text, `[CẬP NHẬT]` notes, the "SurrealDB embedded" section (bannered), or the old column of the update table → **PASS** after the fixes in (a).
- **Windows as supported**: see (a). `docs/WINDOWS-GUIDE.md`/`WINDOWS-PROMPT.md` say "unsupported in v1, expect failures, sandbox fails closed" → PASS.
- **Merging / budget / split_ticket**: PASS (see (a)).
- **50k vs 100k threshold**: **FIXED** — `ADR-47` still said "exact is enough up to ~50k facts, no ANN in M0, `knn_ef` default 200" → amended to the ADR-50 rule (exact < 100 000, usearch ≥ 100 000). Other "50k" hits are unrelated (50 ms, 1.5 M rows).
- **ef values**: SQLite path = `ef_search` 128, auto 256 from 200 000 vectors (ADR-50, STORE-SCALE, PROGRESS, `ann.rs` test). `knn_ef = 200` in `agenium-config` is the *SurrealDB-only* knob; ADR-47 now says so → **PASS** (documented, not a contradiction).
- **Vietnamese below Lead**: FIXED (tool schemas). **OPEN (minor)**: Vietnamese *comments* remain in `schemas/ref/db-*.surql`, `config.example.toml`, `crates/agenium-store/schema/surreal/*.surql` (not model-facing; left as is).

## (c) ADR numbers, index, links
- Script: `/workspace/scratch/linkcheck.py` (kept outside the repo, `/workspace/scratch/linkcheck.py`; it checks every `[..](relative)` link and `#anchor` in repo `*.md` + pack `*.md`, ADR file numbers vs `docs/adr/README.md` vs `DECISIONS.md`).
- ADR files 39–51 sequential; index lists exactly 39–51; DECISIONS lists 1–51 and has a section for each → **PASS**.
- **FIXED**: all 53 `DECISIONS.md` index links `#adr-N` pointed at anchors that did not exist (headings are `## ADR-N: …`) → added `<a id="adr-N"></a>` before every ADR heading. After the fix: 84 relative links checked, 0 broken. Backticked repo paths in repo docs: all exist except `agenium-store/schema/sqlite/agent_003.sql` (named in `schemas/README.md` as the *future* location of the draft; intentional).

## (d) Numbers vs raw results (`docs/bench-raw/`)
- STORE-SCALE §2.1 (10k–1M: ingest, DB size, exact/ANN KNN p50/p95, BM25, hybrid, recall, sidecar, RSS, backup, live ingest) re-compared line by line with `linux-facts-10k-1M.txt` → **PASS**.
- STORE-PARITY: SurrealKV ef=40/200 rows (`linux-crash-sqlite20-and-surreal-facts.txt`), parity 3 seeds × 3000 ops, 0 mismatches (`linux-parity-differential.txt`), crash 20 rounds 0 lost (`…crash-sqlite20…`) → **PASS**. **FIXED**: the "measured 2–4k events/s, 1.7–2k facts/s" sentence → "2.0–4.8k events/s (core), 1.6–2.0k facts/s with ANN at 100k–1M (3.1k at 10k)" (matches `linux-core-1M-2M…` and the live-ingest lines).
- Core store and stress tables were spot-checked against `linux-core-1M-2M-crash-surreal20.txt` and `linux-stress-100k-500k-and-core1M.txt` → PASS.
- **OPEN**: "release build 52 s vs 293 s; binaries 10.1 MB vs 122 MB" (STORE-PARITY §4) were measured earlier in this project but **no raw file exists** in `bench-raw/`; not re-measurable cheaply here.
- **OPEN (by nature)**: the macOS table in STORE-SCALE §5 is as reported by the owner's runs; not reproducible on this Linux box.
- Graph numbers (ADR-51 §12, STORE-SCALE §8) come from `bench-raw/linux-graph-cte-probe.txt`: python `sqlite3` probe on a synthetic graph, **not** the Rust store; labelled as feasibility everywhere.

## (e) Schemas
- `cargo xtask schemas --check` → **PASS** (also inside `ci-local.sh`). `schemas/sql/*` = `crates/agenium-store/schema/sqlite/*` (byte-equal), pack `SCHEMAS/sql/` = repo `schemas/sql/` (equal), pack `SCHEMAS/` = repo `schemas/ref/` (equal after the description fixes).
- Tool schemas: `index.json` = 24 files (Lead 15, Worker 5, Shared 4; Solo set lists 22), no stray/missing file; all JSON valid → PASS.
- The draft `schemas/sql/drafts/agent_003_graph_memory.sql` applies cleanly on top of `agent_001`+`agent_002` (SQLite 3.46), is not a migration and is not touched by the xtask check → PASS.
- The crate's `schema/surreal/db-*.surql` differ from the pack originals by design (patch `S1-schema-patch.diff`); explained in both `schemas` READMEs.
- **OPEN**: `db-*.surql` were not updated for ADR-51 (graph memory is SQLite-first; a SurrealQL version is a stretch item).

## (f) Commands run
Environment: Linux, no Tauri system packages, no PowerShell, no Mac.
| Command | Result |
|---|---|
| `scripts/ci-local.sh --no-tauri` (fmt, clippy default + surrealkv, `cargo test --workspace`, surreal tests, schemas-check, sandbox-attack-tests, ui install/vitest/build) | **PASS**: 139 + 53 Rust tests, 8 sandbox attack tests, 7 vitest, UI build |
| `cargo xtask schemas --check`, `AGENIUM_REQUIRE_SANDBOX=1 cargo xtask sandbox-attack-tests` | PASS |
| `cargo run --release -p agenium-store-bench -- facts 10000` | PASS (runs; synthetic corpus numbers) |
| `cargo xtask ci` | **FAILS on this box** (compiles the Tauri crate → needs GTK/WebKit + pkg-config). **FIXED (doc)**: README now says so and points to `ci-local.sh --no-tauri`. (Code untouched.) |
| `scripts/ci-local.ps1`, everything in `WINDOWS-GUIDE.md` | **NOT RUN** (no Windows / PowerShell here) |
| `tauri build`, opening the `.app`, Seatbelt tests (HANDOFF.md) | **NOT RUN** (no Mac; Tauri crate not compiled on this box — only the UI and Rust crates were) |
| `scripts/ci-local.sh --heavy` / `store-bench` 100k–1M | NOT RE-RUN in this audit (results in `bench-raw/` from earlier) |

## (g) Architecture PDF
- The old pipeline printed Mermaid fences as raw code. **FIXED**: `mkpdf.py` now converts the 6 ` ```mermaid ` blocks and renders them with a local `mermaid@10` bundle inside headless Chromium (`--virtual-time-budget`), SVG height capped so diagrams do not spill. New PDF: 58 pages, 1.3 MB. Checked visually on several sample pages (flow and topology diagrams rendered as graphics); strike-through (`~~`) renders as red `<del>`; the new "Graph memory (ADR-51)" subsection is present. Not every page was inspected; the other three diagrams are produced by the same code path and `pdftotext` shows no Mermaid syntax errors.
- **OPEN (cosmetic)**: diagram text uses Mermaid's default font; very wide diagrams are small.

## (h) Pack index and integrity
- The pack `README.md` index now lists: `AUDIT.md`, `WINDOWS-PROMPT.md`, `agenium-app/`, `design/*` (incl. `SCHEMAS/README.md`, `sql/drafts/`), `docs/*`, `spike-reports/`, `ui-screenshots/`. `build_pack.py` verifies at build time that every top-level path in the zip is covered by the README and that README paths exist in the zip, runs `testzip`, and asserts that no `target*/`, `node_modules/`, `.git/` or bench-raw file > 1 MB is included.
- **OPEN**: obsolete files outside the pack remain in `/workspace/agenium-docs/` (`Agenium-Kien-truc.md/.pdf`, `Agenium-implementation-pack-v1.0.zip`, `agenium-m0-spike.zip`) — superseded by the new zips; not included in the pack, not deleted.

## Other open items (honest list)
1. ADR-51 is a design: no Rust code, no benchmark in the real store, no SurrealDB graph parity run; Mem0/SurrealDB comparison text is from memory (web verification was interrupted), flagged as unverified in the ADR.
2. ADR-51 default thresholds (0.90 / 0.80 similarity, hub cap 200, RRF weights, decay constants) are initial guesses until M1.5 benchmarks.
3. macOS Seatbelt backend, Windows and the Tauri crate build were not run on this box.
4. Flaky-test history: the ANN rebuild test (`tests/agent.rs`) was non-deterministic (~2 % of runs under 0.95 recall per the code comment); bound relaxed to 0.90 in commit 8e2f311.
