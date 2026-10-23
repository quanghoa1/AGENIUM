# REPO-LAYOUT — Agenium v1.0 (FINAL, 2026-10-22)

> **[CẬP NHẬT 2026-10-23 — quyết định mới nhất, ưu tiên hơn nội dung gạch bỏ bên dưới]** SQLite là CSDL chính (rusqlite bundled + sqlite-vec + FTS5; ADR-49/50), SurrealDB chỉ là tính năng tùy chọn `--features surrealkv`. Bản phát hành đầu tiên chỉ **Linux + macOS** (ADR-39); **Windows hoãn** (xem `docs/WINDOWS-GUIDE.md`). Ngôn ngữ dưới Lead là **tiếng Anh**, kèm `user_language` (ADR-40). Chi tiết: `DECISIONS.md` ADR-39…50, `docs/STORE-SCALE.md`, `docs/STORE-PARITY.md`. Spike S0–S7 đã hoàn thành (xem `spike-reports/`).
> `agenium-store`: SQLite mặc định (`sqlite/`, feature `sqlite`, `ann` = usearch), SurrealDB sau feature `surrealkv`; `agenium-store-bench` là bench/parity harness; `scripts/ci-local.{sh,ps1}`; `docs/adr/`.


Monorepo, giấy phép Apache-2.0 (LICENSE + NOTICE giữ nguồn Codex). Rust stable mới nhất, edition 2021, Tauri 2, React + TypeScript (Vite), pnpm.

```text
agenium/
├── Cargo.toml                 # [workspace], resolver = "2", members = ["crates/*", "app/src-tauri"]
├── rust-toolchain.toml
├── LICENSE  NOTICE  README.md
├── deny.toml                  # cargo-deny: license + advisories
├── .github/workflows/         # ci.yml, sandbox-pentest.yml, release.yml (**chỉ chạy thủ công**; dùng `scripts/ci-local.sh`)
├── crates/
│   ├── agenium-core/          # domain types, ID (ULID + AGN-n), TicketStatus/AgentStatus, check_transition, RuleError
│   ├── agenium-config/        # resolve_root, layout root, config.toml (serde), bootstrap file, setup wizard backend
│   ├── agenium-store/         # Repo traits (TicketRepo, AgentRegistry, EventLog, ConversationStore, MemoryStore) + impl **sqlite (mặc định, ADR-49)** + impl surrealdb tùy chọn (`--features surrealkv`), schema/migrate, JSONL export/import
│   ├── agenium-secrets/       # trait SecretStore: KeyringStore | VaultStore (khóa chủ trong keyring) — chọn theo spike S5
│   ├── agenium-llm/           # trait Provider, Message chuẩn hóa, adapter anthropic / gemini / openai_compat, retry, caching, effort map, model list, giá
│   ├── agenium-backend/       # trait AgentBackend: ApiBackend (M1), CliBackend (M2.5)
│   ├── agenium-tools/         # tool set lõi + protocol tools; schema schemars; validate; cắt output
│   ├── agenium-mcp/           # McpHub (stdio, Streamable HTTP), credential binding
│   ├── agenium-memory/        # Embedder (fastembed), pipeline extract → ADD/UPDATE/DELETE, retrieval hybrid, lesson/learning (M4.5); graph memory (ADR-51, M1.5/M4.5): entity resolution, trích xuất, kênh graph + RRF
│   ├── agenium-skills/        # SkillRegistry, SKILL.md parse, overlay copy-on-write
│   ├── agenium-sandbox/       # trait Sandbox + ~~windows/~~ macos/ linux/ (Windows hoãn, fail-closed) (fork Codex), policy, pentest tests
│   ├── agenium-vcs/           # trait Vcs (git2/gix): init, remote test, worktree/ticket, commit, diff, rebase, merge queue, secret scan
│   ├── agenium-bus/           # event bus (tokio::broadcast) + dispatcher, emit sang Tauri
│   ├── agenium-orchestrator/  # agent loop, work order, scheduler (DAG, deps_satisfied), Lead/Worker, watchdog/stall-nudge, inbox/SLA, merge flow, usage
│   └── agenium-eval/          # golden task runner (bin), fixtures, chấm điểm, báo cáo so sánh
├── app/
│   ├── src-tauri/             # Tauri 2 shell: commands (IPC), event forwarding, updater, tray, notifications
│   └── ui/                    # React + TS
│       ├── src/routes/        # Dashboard, Board, Ticket, Agents, Settings, Diagnostics
│       ├── src/components/    # Kanban (dnd-kit), DiffView (Monaco), Terminal (xterm.js), AgentView (stream), NeedsYou, ApprovalCard
│       ├── src/api/           # wrapper invoke() có kiểu, sinh từ Rust qua specta/tauri-specta
│       ├── src/state/         # store (zustand) đồng bộ từ event + resync từ DB
│       └── src/i18n/          # en, vi (i18next)
├── templates/                 # built-in team template YAML: coding, pentest, research, content
├── schemas/                   # JSON Schema sinh từ Rust (schemars) + sql/ (= migration) — commit để review diff; sql/drafts/agent_003_graph_memory.sql = NHÁP ADR-51 (chưa áp dụng)
├── eval/golden/               # golden tasks: <id>/task.yaml, fixture repo, checks
├── spikes/                    # chỉ trong repo agenium-spikes/ riêng (code bỏ) — không merge vào đây
└── docs/                      # pack này + ADR mới (docs/adr/ADR-<n>.md)
```

## Trách nhiệm module & đồ thị phụ thuộc

| Crate | Phụ thuộc | Ghi chú |
|---|---|---|
| `agenium-core` | serde, ulid, thiserror | Không I/O. State machine + guard thuần, test property (proptest) |
| `agenium-config` | core, dirs, toml | `AGENIUM_HOME` > bootstrap > `~/.agenium` |
| `agenium-secrets` | core, keyring, (chacha20poly1305/age) | Không bao giờ log giá trị |
| `agenium-store` | core, rusqlite/sqlite-vec/usearch (surrealdb tùy chọn) | Mọi truy cập DB qua trait; ghi event trước trạng thái |
| `agenium-bus` | core, tokio | Mất thông báo được; UI luôn resync từ DB |
| `agenium-llm` | core, secrets, reqwest, eventsource | Không biết ticket |
| `agenium-tools` | core, sandbox, vcs, schemars | Protocol tools chỉ tạo `Intent`, không ghi DB trực tiếp |
| `agenium-sandbox` | core | fail-closed khi thiếu cơ chế |
| `agenium-vcs` | core, git2/gix | Thao tác idempotent theo ticket + sha |
| `agenium-mcp` | core, secrets, rmcp | |
| `agenium-memory` | core, store, llm, fastembed | DB `agent_<ulid>` riêng |
| `agenium-skills` | core, config | |
| `agenium-backend` | llm, tools, mcp | |
| `agenium-orchestrator` | tất cả trên | Nơi duy nhất áp luật và ghi trạng thái |
| `app/src-tauri` | orchestrator, config, store, bus | Mỏng: chỉ IPC |

## Quy ước
- Lỗi: `thiserror` trong lib, `anyhow` chỉ ở bin/app. Không `unwrap()` ngoài test.
- Async: tokio multi-thread; trait async dùng `async_trait` (hoặc AFIT khi đủ).
- Log: `tracing` JSON vào `<root>/logs/`, lớp che secret bắt buộc.
- ID: `TicketId`/`AgentId` = ULID newtype; `TicketKey` = `AGN-<n>` theo project.
- Schema JSON: sinh bằng `schemars` từ type Rust, xuất ra `schemas/` bằng `cargo xtask schemas`; file trong `SCHEMAS/` của pack là bản tham chiếu khởi đầu.
- IPC: lệnh Tauri trả `Result<T, ApiError>` có kiểu, sinh TS bằng tauri-specta.
