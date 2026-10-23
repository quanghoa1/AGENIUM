# Agenium — complete pack (Linux + macOS first release; Windows probe included)

Built 2026-10-23 (Asia/Saigon). Nothing in this pack was pushed or sent anywhere.

## Start here
| You are… | Read / do |
|---|---|
| The owner on **macOS** | `docs/HANDOFF.md` (Vietnamese): run `agenium-app/scripts/ci-local.sh`, build the `.app`, run the sandbox attack tests |
| An AI on **Windows** | paste `WINDOWS-PROMPT.md`; background in `docs/WINDOWS-GUIDE.md` (Windows is unsupported in v1: expect failures, sandbox fails closed) |
| Reviewing the design | `design/` (below) |

`AUDIT.md` (this folder's top level) is the final consistency audit of the whole pack: PASS / FIXED / OPEN per check, including what could not be run.

## Contents
| Path | What |
|---|---|
| `AUDIT.md` | Final consistency audit (checklist a–h, PASS/FIXED/OPEN, honest open items). |
| `ui-screenshots/` | 11 PNG screenshots of the React UI (Vite + mock `invoke` + Playwright; not the Tauri window). |
| `agenium-app/` | Full source (same as `agenium-app-m0.zip`): Rust workspace (`crates/`, `xtask/`), Tauri shell + React UI (`app/`), `schemas/` (incl. generated `schemas/sql`), `docs/` (ADRs 39–51, incl. ADR-51 graph memory (Proposed, design only), STORE-PARITY, STORE-SCALE, raw benches), `scripts/`, `PROGRESS.md`, `HANDOFF.md`. No `target/`, `node_modules/`, or large raw logs. |
| `agenium-app/scripts/ci-local.sh` / `ci-local.ps1` | **Local CI, one command** (GitHub CI quota is exhausted; `.github/workflows/*` are kept but manual-trigger only / optional). Runs fmt, clippy (default + `--features surrealkv`), `cargo test --workspace`, surreal tests, `cargo xtask schemas --check`, sandbox-attack-tests, pnpm install/vitest/build; prints a PASS/FAIL/SKIP summary; logs in `target/ci-local/`. Flags: `--heavy` (scale benchmarks), `--quick`, `--no-tauri`, `--skip-ui`, `--skip-sandbox`, `--skip-surreal`, `--fail-fast` (PowerShell: `-Heavy -Quick -NoTauri -SkipUi -SkipSurreal -FailFast`). |
| `design/Agenium-Kien-truc-v1.0.md` / `.pdf` | Architecture document (Vietnamese), updated 2026-10-23 with a change table and `[CẬP NHẬT]` notes: SQLite primary, graph memory (ADR-51, planned), Linux+macOS first, English-only below Lead + `user_language`, Windows deferred. Old text is struck through, not deleted. |
| `design/DECISIONS.md` | ADR index 1–51 (ADR-1 superseded by ADR-49; ADR-39…51 added; ADR-51 graph memory is Proposed). **Highest priority on conflict.** |
| `design/IMPLEMENTATION-PLAN.md`, `M0-SPIKE-CHECKLIST.md`, `REPO-LAYOUT.md`, `AGENT-BRIEF.md`, `DESIGN-README.md` | Design pack files with a 2026-10-23 update banner. |
| `design/SCHEMAS/` | JSON schemas, tool schemas, config example, `db-*.surql` (reference, optional backend) `sql/` (SQLite DDL, primary), `sql/drafts/agent_003_graph_memory.sql` (DRAFT, not applied, ADR-51) and `README.md` explaining how the schema folders relate. |
| `spike-reports/` | S0–S7 `REPORT.md` + small raw results, `SUMMARY.md`, thresholds. S1 results > 1 MB omitted. Linux results are in the spike reports; macOS phase-3 results are tabulated in `docs/STORE-SCALE.md`. |
| `docs/STORE-PARITY.md`, `docs/STORE-SCALE.md` | SQLite vs SurrealKV parity and the scale benchmarks (Linux + macOS tables, quantization, ANN threshold 100k, ef scaling). |
| `docs/WINDOWS-GUIDE.md`, `WINDOWS-PROMPT.md` | Windows toolchain, expectations, fixed report format. |
| `docs/HANDOFF.md` | Vietnamese hand-off summary. |

Priority on conflict: `DECISIONS.md` > architecture document > everything else. Changing a decision needs a new ADR.

## Status in one paragraph
Graph/relational agent memory on SQLite is **designed (ADR-51) but not implemented**; the ticket graph is plain join tables. SQLite store (exact + usearch ANN, auto switch at 100k facts), parity with SurrealKV tested, scale measured to 1M facts on Linux; sandbox verified on Linux (bubblewrap/Landlock); macOS Seatbelt written but never executed; Windows untested. See `agenium-app/PROGRESS.md` for the verified test counts and the NOT RUN list.
