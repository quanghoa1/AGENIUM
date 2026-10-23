# Agenium — Implementation Pack v1.0 (FINAL, 2026-10-22)

> **[CẬP NHẬT 2026-10-23 — quyết định mới nhất, ưu tiên hơn nội dung gạch bỏ bên dưới]** SQLite là CSDL chính (rusqlite bundled + sqlite-vec + FTS5; ADR-49/50), SurrealDB chỉ là tính năng tùy chọn `--features surrealkv`. Bản phát hành đầu tiên chỉ **Linux + macOS** (ADR-39); **Windows hoãn** (xem `docs/WINDOWS-GUIDE.md`). Ngôn ngữ dưới Lead là **tiếng Anh**, kèm `user_language` (ADR-40). Chi tiết: `DECISIONS.md` ADR-39…51, `docs/STORE-SCALE.md`, `docs/STORE-PARITY.md`. Spike S0–S7 đã hoàn thành (xem `spike-reports/`).
> Pack này đã được cập nhật theo ADR-39…50; `SCHEMAS/sql/*.sql` là DDL SQLite chính, `SCHEMAS/*.surql` chỉ là tham chiếu cho backend SurrealDB tùy chọn.


App desktop Rust (Tauri 2 + React) điều phối AI agent Solo hoặc team Lead + Workers qua hệ thống ticket tích hợp, chạy trong sandbox kiểu Codex. Giấy phép Apache-2.0.

| File | Nội dung |
|---|---|
| `Agenium-Kien-truc-v1.0.md` / `.pdf` | Thiết kế đầy đủ (export từ Page FINAL v1.0): quyết định, domain, lưu trữ, giao thức Agent ↔ Hệ thống, sandbox, vendor API, learning, Rust sketch, lộ trình, M0-spike, câu hỏi mở chờ spike |
| `DECISIONS.md` | Chỉ mục ADR cho mọi quyết định đánh số (1–51, kèm 2b, 3a–3m; ADR-1 đã bị ADR-49 thay thế; ADR-51 = graph memory, Proposed), cột phụ thuộc spike, mẫu ADR mới |
| `IMPLEMENTATION-PLAN.md` | Mốc M0-spike → M5: thứ tự, phụ thuộc, task + acceptance, Definition of Done |
| `M0-SPIKE-CHECKLIST.md` | S0–S7, ngưỡng Pass/Yellow/Kill, phương án B, mẫu biên bản/ADR, cổng Go/No-Go |
| `REPO-LAYOUT.md` | Cargo workspace (crates + trách nhiệm + phụ thuộc), cấu trúc Tauri/React, quy ước |
| `SCHEMAS/sql/*.sql` | **DDL SQLite (chính, sinh bởi `cargo xtask schemas`)**: core, agent (fact + vec0 + FTS5) |
| `SCHEMAS/db-core.surql` | (tham chiếu, backend tùy chọn) Schema SurrealQL database `core` (project, team, agent, spec, plan, ticket, workspace, link, comment, permission_grant, run, event) |
| `SCHEMAS/db-agent.surql` | Schema database memory riêng `agent_<ulid>` (fact + HNSW 384 + BM25, conversation, skill_overlay) — tham chiếu cho backend tùy chọn; bản chính là `SCHEMAS/sql/agent_*.sql` |
| `SCHEMAS/sql/drafts/agent_003_graph_memory.sql`, `SCHEMAS/README.md` | **NHÁP, chưa áp dụng** — DDL graph memory (ADR-51); README giải thích quan hệ giữa các thư mục schema |
| `AUDIT.md` (ở thư mục gốc của pack, ngoài `design/`) | Kết quả rà soát nhất quán toàn bộ pack (PASS/FIXED/OPEN) |
| `SCHEMAS/*.schema.json` | JSON Schema 2020-12: ticket, plan, requirement-spec, handoff, comment |
| `SCHEMAS/tools/*.schema.json` | Schema tool call cho Lead (15), Worker (5), Shared (4); `index.json` ánh xạ role → tool (Solo = tất cả) |
| `SCHEMAS/config.example.toml` | Ví dụ `<root>/config.toml` |
| `AGENT-BRIEF.md` | Prompt tự đủ cho coding agent bắt đầu M0-spike → M0 |

Thứ tự đọc cho người triển khai: README → DECISIONS → M0-SPIKE-CHECKLIST → IMPLEMENTATION-PLAN → REPO-LAYOUT → SCHEMAS → tài liệu kiến trúc.
Ưu tiên khi mâu thuẫn: `DECISIONS.md` > tài liệu kiến trúc > các file còn lại.
