# AGENT-BRIEF — Bắt đầu triển khai Agenium (M0-spike → M0)

> **[CẬP NHẬT 2026-10-23 — quyết định mới nhất, ưu tiên hơn nội dung gạch bỏ bên dưới]** SQLite là CSDL chính (rusqlite bundled + sqlite-vec + FTS5; ADR-49/50), SurrealDB chỉ là tính năng tùy chọn `--features surrealkv`. Bản phát hành đầu tiên chỉ **Linux + macOS** (ADR-39); **Windows hoãn** (xem `docs/WINDOWS-GUIDE.md`). Ngôn ngữ dưới Lead là **tiếng Anh**, kèm `user_language` (ADR-40). Chi tiết: `DECISIONS.md` ADR-39…50, `docs/STORE-SCALE.md`, `docs/STORE-PARITY.md`. Spike S0–S7 đã hoàn thành (xem `spike-reports/`).
> Cập nhật cho agent: dùng SQLite (không Surreal) làm store mặc định; chỉ Linux+macOS; tiếng Anh dưới Lead + `user_language`; chạy kiểm tra bằng `scripts/ci-local.sh`; không push, không CI GitHub.


Bạn là coding agent triển khai **Agenium v1.0** (thiết kế FINAL ngày 2026-10-22): app desktop Rust (Tauri 2 + React/TS) điều phối AI agent Solo hoặc team Lead + Workers, phối hợp qua hệ thống ticket tích hợp, chạy trong sandbox kiểu Codex. Prose tài liệu bằng tiếng Việt; code, identifier, commit message, comment trong code bằng tiếng Anh.

## Đọc theo thứ tự
1. `README.md` — chỉ mục pack.
2. `DECISIONS.md` — ~~53 mục quyết định (1–38 kèm a–m)~~ 1–50 (kèm a–m; ADR-39…50 là quyết định mới nhất). Không được làm trái; muốn đổi thì viết ADR mới (mẫu cuối file) và dừng lại hỏi.
3. `M0-SPIKE-CHECKLIST.md` — việc đầu tiên.
4. `IMPLEMENTATION-PLAN.md` — task + acceptance theo mốc.
5. `REPO-LAYOUT.md`, `SCHEMAS/` — cấu trúc repo, DB, JSON schema, config.
6. `Agenium-Kien-truc-v1.0.md` — thiết kế đầy đủ (Rust sketch ở mục "Phác thảo code Rust").

## Bất biến không được phá
- Ticket là nguồn sự thật; agent không nói chuyện trực tiếp với nhau; mọi giao tiếp là comment có kiểu.
- AI chỉ hành động qua tool call đã validate schema; orchestrator tất định áp luật và ghi trạng thái. Lỗi validate trả về làm tool result; 3 lỗi liên tiếp dừng run; thiếu tool kết thúc → nudge.
- Ghi event trước, idempotent theo `intent_id`; reconcile DB ↔ git khi khởi động.
- Ticket state: New → Assigned → InProgress ⇄ Pending → Resolved → Closed | Reopened → InProgress; Cancelled. Closed chỉ qua `close_ticket` (Lead/User, có reason; ticket code phải `merged`). Merge không tự đóng. Không có trạng thái Merging.
- Phụ thuộc thỏa khi ticket trước Closed, hoặc Resolved + `approved` (+ `merged` nếu có code).
- Không có `split_ticket`, `request_reassign`. Worker không tự kéo việc.
- Lead cấp quyền chỉ theo run; "luôn cho phép" chỉ người dùng.
- Không budget (quyết định 3k): chỉ hiển thị token + chi phí ước tính, không chặn.
- Secret chỉ trong keyring (hoặc vault với khóa chủ trong keyring nếu S5 chọn); không bao giờ log secret. Telemetry tắt.
- Memory riêng tư theo agent, tiếng Anh (xem lại sau S6). **Graph memory (ADR-51, Proposed, chưa cài):** entity/edge có scope trong DB riêng của agent, truy xuất lai vec+BM25+graph; bài học có scope không áp sang vendor/project khác; chia sẻ giữa agent chỉ qua ticket (`knowledge_note`) hoặc gói tri thức project do Lead/người dùng duyệt (cần chủ sở hữu xác nhận). Đồ thị ticket/phụ thuộc = bảng join. DDL nháp: `SCHEMAS/sql/drafts/`; không áp dụng trước M1.5.

## Nhiệm vụ đầu tiên
1. Tạo repo `agenium-spikes/` và chạy M0-spike theo checklist. Chốt ngưỡng trước, timebox cứng, đo trên máy tối thiểu, lưu evidence thô. Mỗi spike viết biên bản + ADR theo mẫu.
2. Ở cổng Go/No-Go (ngày 10): tổng hợp bảng Spike × Kết quả × Quyết định; đề xuất sửa quyết định 1/2b/5/9/22/24 nếu cần. **Dừng và báo người dùng** trước khi sang M0.
3. Khi được Go: tạo repo `agenium/` theo `REPO-LAYOUT.md`, làm M0-1 → M0-11 theo `IMPLEMENTATION-PLAN.md`. Bắt đầu từ `agenium-core` (state machine + `dep_satisfied` + property test), rồi store theo engine S1 chọn, dùng `SCHEMAS/db-core.surql` làm điểm xuất phát.

## Cách làm việc
- Mỗi task: nhánh riêng, test trước/đi kèm, kiểm tra local (`scripts/ci-local.sh`) xanh trên Linux/macOS, PR nhỏ có mô tả acceptance đã đạt.
- Sinh JSON Schema từ Rust bằng `schemars`; so với `SCHEMAS/tools/*.schema.json`; khác thì cập nhật cả hai và ghi lý do.
- Không thêm dependency nặng/không rõ license (cargo-deny). Code port từ Codex giữ header + NOTICE.
- Gặp mâu thuẫn giữa tài liệu: ưu tiên `DECISIONS.md` > Page > các file khác; ghi lại và hỏi.
- Báo cáo cuối mỗi mốc: task đã xong, số đo, rủi ro, ADR mới.
