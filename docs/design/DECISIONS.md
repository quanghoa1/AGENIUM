# DECISIONS — Agenium v1.0 (FINAL, 2026-10-22)

Chỉ mục quyết định kiểu ADR. Nguồn: mục "Quyết định đã chốt" trong `Agenium-Kien-truc-v1.0.md`. Trạng thái: **Accepted** cho mọi mục; mục có cột *Spike* có thể được sửa sau cổng Go/No-Go của M0-spike (ghi thành ADR mới, không sửa ADR cũ).

| ADR | Chủ đề | Trạng thái | Phụ thuộc spike |
|---|---|---|---|
| [ADR-1](#adr-1) | Database | **Superseded by ADR-49** | S0, S1 |
| [ADR-2](#adr-2) | Memory | Accepted | — |
| [ADR-2b](#adr-2b) | Embedding | Accepted | S6 |
| [ADR-2c](#adr-2c) | Ngôn ngữ | Accepted | S6 |
| [ADR-3](#adr-3) | Team | Accepted | — |
| [ADR-3a](#adr-3a) | UI & nền tảng | Accepted | S7 |
| [ADR-3b](#adr-3b) | AI vendor (bản đầu) | Accepted | — |
| [ADR-3c](#adr-3c) | AgentBackend | Accepted | — |
| [ADR-3d](#adr-3d) | Giao tiếp | Accepted | — |
| [ADR-3e](#adr-3e) | Đa mục đích | Accepted | — |
| [ADR-3f](#adr-3f) | Topology hình sao | Accepted | — |
| [ADR-3g](#adr-3g) | Handoff & cách ly context | Accepted | — |
| [ADR-3h](#adr-3h) | Từ yêu cầu đến plan | Accepted | — |
| [ADR-3i](#adr-3i) | Trạng thái ticket | Accepted | — |
| [ADR-3j](#adr-3j) | QuotaExhausted | Accepted | — |
| [ADR-3k](#adr-3k) | Không giới hạn budget | Accepted | — |
| [ADR-3l](#adr-3l) | Template team & Role | Accepted | — |
| [ADR-3m](#adr-3m) | SOUL.md, ROLE.md & manifest | Accepted | — |
| [ADR-4](#adr-4) | Duyệt quyền | Accepted | — |
| [ADR-5](#adr-5) | MCP | Accepted | S5 |
| [ADR-6](#adr-6) | AI Solo | Accepted | — |
| [ADR-7](#adr-7) | Project & workspace | Accepted | — |
| [ADR-8](#adr-8) | Skills | Accepted | — |
| [ADR-9](#adr-9) | Sandbox | Accepted | S2, S3 |
| [ADR-10](#adr-10) | Bảo mật & báo cáo | Accepted | — |
| [ADR-11](#adr-11) | Workspace theo ticket & Git | Accepted | S4 |
| [ADR-12](#adr-12) | Agenium root (nơi lưu) | Accepted | — |
| [ADR-13](#adr-13) | Vendor API layer | Accepted | — |
| [ADR-14](#adr-14) | Bản đồ lưu trữ | Accepted | — |
| [ADR-15](#adr-15) | Retry & MCP transport | Accepted | — |
| [ADR-16](#adr-16) | Tool set | Accepted | — |
| [ADR-17](#adr-17) | Phát hiện lặp | Accepted | — |
| [ADR-18](#adr-18) | Merge thất bại | Accepted | — |
| [ADR-19](#adr-19) | Khóa chỉnh sửa của người dùng | Accepted | — |
| [ADR-20](#adr-20) | Lead lỗi & câu hỏi worker | Accepted | — |
| [ADR-21](#adr-21) | UI & thông báo | Accepted | — |
| [ADR-22](#adr-22) | Nhất quán khi crash | Accepted | S4 |
| [ADR-23](#adr-23) | Backup & dọn dẹp | Accepted | — |
| [ADR-24](#adr-24) | Yêu cầu hệ thống | Accepted | S0, S3 |
| [ADR-25](#adr-25) | Gán project | Accepted | — |
| [ADR-26](#adr-26) | Telemetry | Accepted | — |
| [ADR-27](#adr-27) | Marketplace & đồng bộ ngoài | Accepted | — |
| [ADR-28](#adr-28) | Giá trị vận hành & quy tắc | Accepted | — |
| [ADR-29](#adr-29) | Kỹ thuật & phát hành | Accepted | S1 |
| [ADR-30](#adr-30) | CLI: phát hiện & cách ly | Accepted | — |
| [ADR-31](#adr-31) | Local / custom endpoint | Accepted | — |
| [ADR-32](#adr-32) | Xóa agent | Accepted | — |
| [ADR-33](#adr-33) | Giấy phép | Accepted | — |
| [ADR-34](#adr-34) | An toàn & vận hành bổ sung | Accepted | — |
| [ADR-35](#adr-35) | Learning | Accepted | — |
| [ADR-36](#adr-36) | Giao thức Agent ↔ Hệ thống | Accepted | — |
| [ADR-37](#adr-37) | Spike M0 trước khi dựng khung | Accepted | — |
| [ADR-38](#adr-38) | Kiểm thử & eval | Accepted | — |
| [ADR-39](#adr-39) | Bản đầu chỉ Linux + macOS (Windows hoãn) | Accepted | — |
| [ADR-40](#adr-40) | Ngôn ngữ: người dùng tùy chọn, từ Lead trở xuống chỉ tiếng Anh | Accepted (sửa ADR-2c) | — |
| [ADR-41](#adr-41) | Codex sandbox vendoring không sửa + wrapper | Accepted (làm rõ ADR-9) | — |
| [ADR-42](#adr-42) | Linux sandbox: bubblewrap / Landlock ≥ 3, fail-closed | Accepted | — |
| [ADR-43](#adr-43) | macOS Seatbelt profile v2 | Accepted (thử nghiệm) | — |
| [ADR-44](#adr-44) | VCS qua git CLI | Accepted | — |
| [ADR-45](#adr-45) | IntentJournal + EventSink trong core | Accepted | — |
| [ADR-46](#adr-46) | Backup full + incremental; migration | Accepted (sửa bởi ADR-49) | — |
| [ADR-47](#adr-47) | Trait Embedder; model mặc định | Accepted | — |
| [ADR-48](#adr-48) | xtask là workspace member; kiểu IPC viết tay ở M0 | Accepted | — |
| [ADR-49](#adr-49) | **SQLite là store chính; SurrealDB tùy chọn** | Accepted (chủ sở hữu 2026-10-23) | — |
| [ADR-50](#adr-50) | VectorIndex exact + ANN, ngưỡng 100k, concurrency, retention | Accepted (2026-10-23) | — |
| [ADR-51](#adr-51) | **Graph memory trên SQLite** (entity/edge/scope, truy xuất lai vec+BM25+graph) | **Proposed** (thiết kế; chờ chủ sở hữu duyệt mục 10 — chia sẻ giữa agent) | — |

<a id="adr-1"></a>
## ADR-1: Database

- **Trạng thái:** ~~Accepted (2026-10-22)~~ **SUPERSEDED bởi ADR-49 (2026-10-23): SQLite là engine chính; đây là "Phương án B" cũ được nâng thành đường chính. SurrealDB còn là backend tùy chọn.**
- **Quyết định:** SurrealDB 3.x embedded (crate `surrealdb`); SurrealKV đã ra 1.0 (13/10/2026, SurrealDB 3.3.4): engine SurrealKV hay RocksDB chốt bằng so sánh trực tiếp trong spike S1. Phương án B nếu S1 Kill: SQLite (`rusqlite`/`sqlx`) + `sqlite-vec` cho vector + FTS5 cho BM25, realtime bằng `tokio::broadcast`; Repo trait giữ nguyên

<a id="adr-2"></a>
## ADR-2: Memory

> **[CẬP NHẬT ADR-51 (Proposed)]** Thêm đồ thị entity/edge có scope trong cùng DB riêng của agent; chia sẻ giữa agent chỉ qua ticket (`knowledge_note`) hoặc gói tri thức project do Lead/người dùng duyệt (chờ chủ sở hữu xác nhận) — vẫn không có DB chung.

- **Trạng thái:** Accepted (2026-10-22)
- **Quyết định:** Riêng tư tuyệt đối theo agent, như trí nhớ con người; chia sẻ thông tin chỉ qua ticket/comment. Pipeline tự viết bằng Rust (ý tưởng Mem0, không dùng thư viện/dịch vụ Mem0), lưu hoàn toàn trong SurrealDB

<a id="adr-2b"></a>
## ADR-2b: Embedding

- **Trạng thái:** Accepted (2026-10-22) · xem lại sau spike S6
- **Quyết định:** Local, chỉ tiếng Anh: fastembed-rs + `bge-small-en-v1.5` (384 chiều); trait `Embedder` cho phép đổi sang API. Giữ chỉ tiếng Anh, xem lại theo số đo spike S6 (recall@5 Anh vs Việt, so với model đa ngữ như multilingual-e5-small / bge-m3)

<a id="adr-2c"></a>
## ADR-2c: Ngôn ngữ

- **Trạng thái:** Accepted (2026-10-22) · xem lại sau spike S6
- **Quyết định:** Ngôn ngữ làm việc là tiếng Anh. Solo: agent tự dịch yêu cầu tiếng Việt sang tiếng Anh, memory bằng tiếng Anh. Team: Lead dịch yêu cầu sang tiếng Anh; ticket, comment, giao việc đều bằng tiếng Anh; worker làm hoàn toàn bằng tiếng Anh; Lead dịch kết quả/câu hỏi về ngôn ngữ của người dùng khi trao đổi với họ

<a id="adr-3"></a>
## ADR-3: Team

- **Trạng thái:** Accepted (2026-10-22)
- **Quyết định:** Mỗi agent thuộc đúng một team

<a id="adr-3a"></a>
## ADR-3a: UI & nền tảng

- **Trạng thái:** Accepted (2026-10-22) · xem lại sau spike S7
- **Quyết định:** Tauri 2 + React (TypeScript), đa nền tảng Windows/macOS/Linux; dnd-kit (kanban), Monaco (code/diff), xterm.js (terminal); cập nhật realtime qua SurrealDB live query rồi Tauri event. Mockup UI: `/workspace/agenium-mockup/` (`index.html`, `mockup.png`) **[CẬP NHẬT ADR-39: Windows hoãn khỏi bản đầu (chỉ Linux + macOS); phần Windows/3 OS ở đây là thiết kế hoãn hoặc lịch sử]**

<a id="adr-3b"></a>
## ADR-3b: AI vendor (bản đầu)

- **Trạng thái:** Accepted (2026-10-22)
- **Quyết định:** 6 vendor cloud + Local: Anthropic (Claude), OpenAI, Google Gemini, OpenRouter, Opencode (API key gói subscription qua HTTP API, hoặc CLI `opencode`), DeepSeek (qua adapter OpenAI-compatible; thinking bằng `reasoning_effort`; KV cache đĩa tự động, khớp prefix từ token 0); Local / custom endpoint (quyết định 31). Tất cả implement trait `Provider`; API key lưu trong OS keyring qua `keyring` (không bao giờ trong Agenium root)

<a id="adr-3c"></a>
## ADR-3c: AgentBackend

- **Trạng thái:** Accepted (2026-10-22)
- **Quyết định:** Chỉ hai backend: API và CLI. `ApiBackend` là chính, cho cả 6 vendor cloud + Local (M1). `CliBackend` (M2.5): CLI chính chủ ở chế độ headless JSON-stream (`claude -p --output-format stream-json`, `codex exec --json`, `agy -p --output-format stream-json`, `opencode run --format json`) với 4 lớp kiểm soát: MCP của Agenium cho intent/tool, sandbox, giới hạn bước/lặp, bắt buộc report; memory/skill do Agenium inject. Opencode và Google dùng API hoặc CLI (`opencode`, `agy`)

<a id="adr-3d"></a>
## ADR-3d: Giao tiếp

- **Trạng thái:** Accepted (2026-10-22)
- **Quyết định:** Mọi giao tiếp giữa agent diễn ra trên ticket dưới dạng comment có kiểu; không có tin nhắn trực tiếp agent-agent. Comment là nguồn sự thật và audit duy nhất; người dùng xem thread live và có thể trả lời/override

<a id="adr-3e"></a>
## ADR-3e: Đa mục đích

- **Trạng thái:** Accepted (2026-10-22)
- **Quyết định:** Agenium không chỉ cho coding. WorkReport = phần lõi chung + phần mở rộng theo template (JSON Schema, có version, orchestrator validate). Không bắt buộc "tests pass" toàn cục; template có thể bắt buộc

<a id="adr-3f"></a>
## ADR-3f: Topology hình sao

- **Trạng thái:** Accepted (2026-10-22)
- **Quyết định:** Worker không bao giờ giao tiếp trực tiếp với nhau; mọi thứ qua Lead trên ticket. Worker chỉ được đề xuất liên kết `related_to`; Lead tương quan, tạo ticket tổng hợp, chuyển tiếp context giữa ticket, tất cả được ghi log

<a id="adr-3g"></a>
## ADR-3g: Handoff & cách ly context

- **Trạng thái:** Accepted (2026-10-22)
- **Quyết định:** TicketLink chỉ để truy vết; nội dung ticket liên kết không tự vào context. Chuyển thông tin chỉ qua comment `Handoff` do Lead/người dùng đăng trên ticket đích, bất biến. Worker chỉ đọc được ticket của mình

<a id="adr-3h"></a>
## ADR-3h: Từ yêu cầu đến plan

- **Trạng thái:** Accepted (2026-10-22)
- **Quyết định:** Discovery → RequirementSpec được người dùng duyệt (khóa v1) → Plan (milestone + item, trace về spec) được người dùng duyệt → mới tách ticket. Mọi ticket phải gắn plan item; thay đổi plan có version, đổi scope cần người dùng duyệt

<a id="adr-3i"></a>
## ADR-3i: Trạng thái ticket

- **Trạng thái:** Accepted (2026-10-22)
- **Quyết định:** New → Assigned → InProgress ⇄ Pending → Resolved → Closed (terminal, chỉ qua `close_ticket` của Lead/User, merge không tự đóng) hoặc Reopened → InProgress; Cancelled từ mọi trạng thái chưa kết thúc. Reopened quá 3 lần thì gắn cờ escalated. Agent: Idle, Working, Waiting, QuotaExhausted, Offline

<a id="adr-3j"></a>
## ADR-3j: QuotaExhausted

- **Trạng thái:** Accepted (2026-10-22)
- **Quyết định:** Chỉ khi provider báo hết quota/credit (402 hoặc 429 mã quota như `insufficient_quota`; 429 rate limit tạm thời chỉ xếp hàng + backoff): agent dừng, ticket giữ trạng thái + comment System, không được giao việc, escalate ưu tiên cao nhất + thông báo desktop. Đổi vendor/model hoặc nạp credit thì tiếp tục từ checkpoint

<a id="adr-3k"></a>
## ADR-3k: Không giới hạn budget

- **Trạng thái:** Accepted (2026-10-22)
- **Quyết định:** Không có budget token/chi phí/thời gian theo agent hay ticket; ưu tiên hoàn thành đúng chất lượng. Chi phí/token chỉ là chỉ số hiển thị: UI hiện token đã dùng + chi phí ước tính (theo bảng giá) cho từng run và ticket, không chặn, không cảnh báo dừng. Người dùng tự quản lý chi phí; an toàn do sandbox đảm nhận

<a id="adr-3l"></a>
## ADR-3l: Template team & Role

- **Trạng thái:** Accepted (2026-10-22)
- **Quyết định:** UX nhiều lớp: template có sẵn (Coding, Pentest, Research, Content) → Lead đề xuất role qua `propose_role`, chỉ tạo agent khi người dùng duyệt → tùy biến và lưu template riêng (YAML/JSON có version). Role là mô tả công việc, Agent là thực thể gắn một role và một team

<a id="adr-3m"></a>
## ADR-3m: SOUL.md, ROLE.md & manifest

- **Trạng thái:** Accepted (2026-10-22)
- **Quyết định:** SOUL.md (danh tính agent) + ROLE.md (mô tả role) có version, sửa trong UI, agent chỉ đề xuất. Capability manifest sinh tự động mỗi run từ cấu hình thật; skill nạp đầy đủ qua `load_skill`. Thứ tự context: work order của quyết định 36

<a id="adr-4"></a>
## ADR-4: Duyệt quyền

- **Trạng thái:** Accepted (2026-10-22)
- **Quyết định:** Trong allowlist: Lead duyệt, chỉ có hiệu lực cho run hiện tại; ngoài allowlist: chuyển lên người dùng. "Luôn cho phép" (thêm vĩnh viễn vào allowlist project) chỉ người dùng cấp được; Lead không bao giờ cấp quyền vĩnh viễn

<a id="adr-5"></a>
## ADR-5: MCP

- **Trạng thái:** Accepted (2026-10-22) · xem lại sau spike S5
- **Quyết định:** Mỗi agent có kết nối MCP riêng với credential binding riêng (key dùng chung hoặc riêng); secret trong OS keyring (Windows Credential Manager / macOS Keychain / Linux Secret Service) qua crate `keyring`. Kiểm chứng ở spike S5: Windows Credential Manager giới hạn blob ~2,5 KB (2.560 byte, ít hơn sau UTF-16), không chứa nổi token OAuth/khóa RSA lớn. Phương án B: keyring chỉ giữ một khóa chủ, mọi secret lưu trong vault mã hóa (XChaCha20-Poly1305 hoặc age) tại Agenium root **[CẬP NHẬT ADR-39: Windows hoãn khỏi bản đầu (chỉ Linux + macOS); phần Windows/3 OS ở đây là thiết kế hoãn hoặc lịch sử]**

<a id="adr-6"></a>
## ADR-6: AI Solo

- **Trạng thái:** Accepted (2026-10-22)
- **Quyết định:** Cùng vòng plan + ticket như team, agent solo chính là Lead: tự duyệt plan, ticket, review. Stall-nudge áp dụng cho mọi agent. Được tự sửa plan, tạo ticket theo cùng luật version; người dùng vẫn được báo khi đổi scope

<a id="adr-7"></a>
## ADR-7: Project & workspace

- **Trạng thái:** Accepted (2026-10-22)
- **Quyết định:** Project = folder (git repo) có `agenium.toml`, đặt ở đâu cũng được (mặc định `<root>/projects`); dữ liệu team/agent/plan/ticket nằm trong SurrealDB tại `<root>/db`, project chỉ được tham chiếu bằng đường dẫn trong DB `core`; mỗi team một project tại một thời điểm. Workspace riêng `<project>/agents/<agent_id>/`, cấm đọc chéo. Export `.agenium` (không chứa secret)

<a id="adr-8"></a>
## ADR-8: Skills

- **Trạng thái:** Accepted (2026-10-22)
- **Quyết định:** Skill là folder: SKILL.md kiểu Codex/Claude (frontmatter name/description + body) + tùy chọn `scripts/`, `references/`, `assets/`; lưu tại `<root>/agents/<id>/skills/<name>/` hoặc `<root>/roles/<role>/skills/<name>/`. Progressive disclosure: name + description → `load_skill` nạp body → file tham chiếu đọc khi cần; scripts chỉ chạy trong sandbox. Nguồn: built-in, người dùng viết trong UI, import từ folder/zip/URL. Base có version, gán theo role/agent; agent chỉ tiến hóa overlay riêng (quyết định 35)

<a id="adr-9"></a>
## ADR-9: Sandbox

- **Trạng thái:** Accepted (2026-10-22) · xem lại sau spike S2, S3
- **Quyết định:** Fork/adapt sandbox của OpenAI Codex (https://github.com/openai/codex, Apache-2.0) cho Windows/macOS/Linux; giữ LICENSE/NOTICE. Xin quyền là comment trên ticket, template team đặt network allowlist mặc định, Lead duyệt trong allowlist (theo run), ngoài thì người dùng. Sandbox Windows của Codex đã đổi kiến trúc (2026): hai local user riêng (offline/online), binary `codex-command-runner`, setup một lần cần quyền admin để tạo user + firewall rule, DPAPI; bản restricted token + proxy env cũ chỉ chặn mạng kiểu advisory. Kiểm chứng ở spike S2/S3. Phương án B cho Windows: (1) WSL2, (2) Docker/VM nhẹ theo project, (3) nhãn experimental: chỉ ghi-trong-workspace, không tuyên bố cách ly mạng, tắt YOLO mặc định trên Windows **[CẬP NHẬT ADR-39: Windows hoãn khỏi bản đầu (chỉ Linux + macOS); phần Windows/3 OS ở đây là thiết kế hoãn hoặc lịch sử]**

<a id="adr-10"></a>
## ADR-10: Bảo mật & báo cáo

- **Trạng thái:** Accepted (2026-10-22)
- **Quyết định:** Secret chỉ trong keyring của OS; log là event log có cấu trúc (mức info/debug, che secret). Export báo cáo Markdown từ WorkReport/Plan có cấu trúc (PDF sau). Export audit trail theo project

<a id="adr-11"></a>
## ADR-11: Workspace theo ticket & Git

- **Trạng thái:** Accepted (2026-10-22) · xem lại sau spike S4
- **Quyết định:** Một cấu trúc thống nhất cho mọi domain. Mọi project là git repo (không có remote thì `git init` local; remote tùy chọn với SSH key ed25519 + Test repo). Mọi ticket có git worktree + branch `agenium/<ticket-id>`; cùng luồng review diff → merge queue tuần tự. Layout chuẩn: `deliverables/`, `artifacts/`, `notes/`, `.agenium/`. Khác biệt domain chỉ nằm trong template (role, skill, report template, network allowlist, pre-merge check). File nhị phân lớn qua Git LFS hoặc store theo content hash ngoài git, commit tham chiếu

<a id="adr-12"></a>
## ADR-12: Agenium root (nơi lưu)

- **Trạng thái:** Accepted (2026-10-22)
- **Quyết định:** Người dùng chọn thư mục gốc trong setup wizard lần chạy đầu. Không chọn thì mặc định `~/.agenium` trên mọi OS (Windows `%USERPROFILE%\.agenium`), giống Hermes agent (`~/.hermes`), Codex (`~/.codex`), Claude (`~/.claude`). Thứ tự: env `AGENIUM_HOME` > root chọn lúc setup > mặc định. Root chứa `config.toml`, `db/`, `models/`, `logs/`, `projects/`, `agents/<id>/` (SOUL.md, `skills/`), `roles/<role>/` (ROLE.md, `skills/`), `templates/`, `backups/`. Secret luôn trong OS keyring, không bao giờ trong root. Đổi root sau trong Settings qua bước di chuyển/migrate **[CẬP NHẬT ADR-39: Windows hoãn khỏi bản đầu (chỉ Linux + macOS); phần Windows/3 OS ở đây là thiết kế hoãn hoặc lịch sử]**

<a id="adr-13"></a>
## ADR-13: Vendor API layer

- **Trạng thái:** Accepted (2026-10-22)
- **Quyết định:** 3 adapter M1 (Anthropic, Gemini, OpenAI-compatible cho OpenAI/OpenRouter/Opencode/DeepSeek/Local), tổng 6 vendor cloud + Local. Người dùng chọn model + effort (low/medium/high/max), không có công tắc thinking riêng. Prefix ổn định + history append-only để tối đa cache; Anthropic đặt `cache_control` cuối tools, cuối system tĩnh và message cuối. `max_steps` 200/run, chạm giới hạn thì tóm tắt + checkpoint + run mới. Danh sách model lấy từ API + cache, giá chỉ hiển thị từ file cấu hình; đếm token theo usage + ước lượng delta; ảnh/PDF khi model hỗ trợ; limiter theo API key; tool call sai 3 lần liên tiếp thì dừng run; thinking giữ trong history, không vào memory, ẩn mặc định. Layer hoàn tất cho M1

<a id="adr-14"></a>
## ADR-14: Bản đồ lưu trữ

- **Trạng thái:** Accepted (2026-10-22)
- **Quyết định:** Dữ liệu điều phối/truy vấn ở DB (`core`, `agent_<ulid>`), sản phẩm công việc trong git của project, cấu hình/tài nguyên ở root, secret chỉ ở OS keyring

<a id="adr-15"></a>
## ADR-15: Retry & MCP transport

- **Trạng thái:** Accepted (2026-10-22)
- **Quyết định:** Retry: lỗi mạng, timeout, 5xx, 529, 429 tạm thời (theo `retry-after`), đứt stream; backoff 1s nhân đôi, trần 30s, jitter, tối đa 8 lần hoặc 5 phút, rồi agent → Offline + probe nền + tự resume. Đứt stream: bỏ output dở, gửi lại request y hệt, tool đã xong không chạy lại. 400/401 báo lỗi; 402/429 quota → QuotaExhausted. MCP: stdio (local, sandbox, tự restart) và Streamable HTTP (fallback SSE), OAuth/header token trong keyring, domain trong allowlist; tool chỉ đọc được retry, tool ghi trả lỗi về model

<a id="adr-16"></a>
## ADR-16: Tool set

- **Trạng thái:** Accepted (2026-10-22)
- **Quyết định:** Tool lõi: read_file, list_dir, glob, grep, write_file, edit_file, multi_edit, apply_patch, shell, fetch, todo, ticket intents, MCP. Read-before-edit, khớp duy nhất, apply_patch cho nhiều file, cắt output 30.000 ký tự / 256 dòng (bản đầy đủ lưu artifact), shell timeout 120 giây / tối đa 10 phút, path giới hạn trong worktree. Port từ Codex (Apache-2.0) có attribution; Claude Code chỉ học hành vi (clean-room)

<a id="adr-17"></a>
## ADR-17: Phát hiện lặp

- **Trạng thái:** Accepted (2026-10-22)
- **Quyết định:** Chỉ dừng run khi CÙNG một tool với args GIỐNG HỆT được gọi 3 lần liên tiếp; cùng tool nhưng args khác (vd. đọc nhiều file) là hợp lệ

<a id="adr-18"></a>
## ADR-18: Merge thất bại

- **Trạng thái:** Accepted (2026-10-22)
- **Quyết định:** Ticket giữ Resolved (chưa Closed). Merge queue rebase trên main mới nhất; pre-merge check fail → Reopened về đúng worker kèm log; conflict → hệ thống tự Reopened về đúng worker kèm danh sách file conflict + Handoff do hệ thống sinh chứa diff của ticket đã merge; worker sửa trong worktree rồi Resolved lại; conflict lớn/thiết kế → Lead quyết, có thể tạo ticket resolve. Phòng ngừa: Lead khai báo scope file/khu vực cho mỗi ticket trong plan, ticket chồng scope phải có depends_on; sau mỗi merge, ticket đang làm đụng cùng file nhận nudge rebase sớm

<a id="adr-19"></a>
## ADR-19: Khóa chỉnh sửa của người dùng

- **Trạng thái:** Accepted (2026-10-22)
- **Quyết định:** Khi project đang chạy, worktree và branch `agenium/*` là chỉ đọc với người dùng trên UI; thay đổi của người dùng đi qua comment `User` trên ticket hoặc gửi Lead; sửa từ bên ngoài bị phát hiện → revert và báo người dùng; sửa `main` trực tiếp chỉ khi project tạm dừng

<a id="adr-20"></a>
## ADR-20: Lead lỗi & câu hỏi worker

- **Trạng thái:** Accepted (2026-10-22)
- **Quyết định:** Lead lỗi/QuotaExhausted/Offline: worker làm tiếp, câu hỏi xếp hàng, quá SLA báo người dùng; người dùng gán agent khác làm Lead, không mất dữ liệu. Câu hỏi worker: Lead trả lời; Lead không chắc thì escalate người dùng

<a id="adr-21"></a>
## ADR-21: UI & thông báo

- **Trạng thái:** Accepted (2026-10-22)
- **Quyết định:** 5 màn hình: Dashboard, Board, Ticket, Agents, Settings. Thông báo chỉ cho QuotaExhausted, cần duyệt, escalation, epic xong. Agent view xem live kiểu Codex/Claude desktop. Worker trong team: chat chỉ đọc, người dùng pause/stop được, muốn lái thì comment ticket hoặc nhắn Lead. Lead và Solo: chat trực tiếp

<a id="adr-22"></a>
## ADR-22: Nhất quán khi crash

- **Trạng thái:** Accepted (2026-10-22) · xem lại sau spike S4
- **Quyết định:** DB = trạng thái, git = nội dung; ghi event trước, thao tác git idempotent theo ticket + sha; reconcile khi khởi động, tự sửa hoặc báo người dùng

<a id="adr-23"></a>
## ADR-23: Backup & dọn dẹp

- **Trạng thái:** Accepted (2026-10-22)
- **Quyết định:** Incremental mỗi giờ + full mỗi ngày, JSONL nén zstd; giữ 24 bản giờ, 7 ngày, 4 tuần; full backup trước migrate schema, lỗi thì tự rollback; mặc định `<root>/backups` (đổi được); không chứa secret; không nhân đôi nội dung git, có remote thì push branch khi Closed. Tool output đầy đủ là artifact git, không vào DB; run log quá 90 ngày được nén gọn; Settings hiển thị dung lượng + dọn dẹp

<a id="adr-24"></a>
## ADR-24: Yêu cầu hệ thống

- **Trạng thái:** Accepted (2026-10-22) · xem lại sau spike S0, S3
- **Quyết định:** Tối thiểu CPU 64-bit 4 nhân, 8 GB RAM, 10 GB SSD trống; khuyến nghị 8 nhân, 16 GB RAM, 50 GB SSD; không cần GPU; Windows 10 22H2/11, macOS 12+, Linux kernel 5.13+ (Landlock); git 2.30+; internet ổn định. Máy yếu tự giảm mức song song **[CẬP NHẬT ADR-39: Windows hoãn khỏi bản đầu (chỉ Linux + macOS); phần Windows/3 OS ở đây là thiết kế hoãn hoặc lịch sử]**

<a id="adr-25"></a>
## ADR-25: Gán project

- **Trạng thái:** Accepted (2026-10-22)
- **Quyết định:** Một team làm đúng một project tại một thời điểm; Solo cũng vậy. Team/Solo chỉ được gán sang project khác sau khi project hiện tại Cancelled hoặc Completed. Nhiều project chạy đồng thời trên máy chỉ khi mỗi project có team/Solo riêng; dùng chung giới hạn song song toàn cục + limiter theo API key. Memory: một memory mỗi agent (agent làm nhiều project tuần tự trong đời), fact gắn `project_id`, ưu tiên project hiện tại

<a id="adr-26"></a>
## ADR-26: Telemetry

- **Trạng thái:** Accepted (2026-10-22)
- **Quyết định:** Tắt mặc định, không gửi gì. Crash report tùy chọn (opt-in), che secret, không chứa prompt/code

<a id="adr-27"></a>
## ADR-27: Marketplace & đồng bộ ngoài

- **Trạng thái:** Accepted (2026-10-22)
- **Quyết định:** Không có marketplace (không nằm trong kế hoạch); thêm thủ công MCP/skill/template (dán MCP config, import skill từ folder/zip/URL, import template YAML) + danh sách nhỏ MCP phổ biến built-in. Ticket lưu duy nhất trong DB của app, tự chứa, không đồng bộ Jira/GitHub; export Markdown

<a id="adr-28"></a>
## ADR-28: Giá trị vận hành & quy tắc

- **Trạng thái:** Accepted (2026-10-22)
- **Quyết định:** Solo tự merge sau self-review, tự `close_ticket` + báo người dùng, có tùy chọn "duyệt trước khi merge" theo project. 1 ticket active/worker; tối đa 4 agent song song (tự giảm trên máy yếu); SLA câu hỏi blocking 10 phút; stall 5 phút, nudge 3 lần rồi escalate. Quyền đã duyệt có hiệu lực theo run của ticket; "luôn cho phép" (thêm vào allowlist project) chỉ người dùng cấp được, Lead không cấp (quyết định 4). Conflict lớn (> 5 file, hoặc ngoài `scope_paths`, hoặc worker sửa thất bại 2 lần) → Lead quyết

<a id="adr-29"></a>
## ADR-29: Kỹ thuật & phát hành

- **Trạng thái:** Accepted (2026-10-22) · xem lại sau spike S1
- **Quyết định:** `schema_version` trong `core`, migrate lúc khởi động (backup/rollback theo quyết định 23). Test: unit/property cho state machine/guard/scheduler, mock provider record/replay, test xâm nhập sandbox trong CI cả 3 OS, e2e Tauri. UI tiếng Anh + tiếng Việt (i18next). Tauri updater có chữ ký, ký code Windows/macOS từ beta. Model trích xuất memory = model của agent, có thể đặt model rẻ hơn. Engine chốt theo spike S1; đổi engine về sau qua JSONL **[CẬP NHẬT ADR-39: Windows hoãn khỏi bản đầu (chỉ Linux + macOS); phần Windows/3 OS ở đây là thiết kế hoãn hoặc lịch sử]**

<a id="adr-30"></a>
## ADR-30: CLI: phát hiện & cách ly

- **Trạng thái:** Accepted (2026-10-22)
- **Quyết định:** Người dùng chọn API hoặc CLI cho từng agent. Agenium chỉ PHÁT HIỆN CLI đã cài trong PATH (`claude`, `codex`, `agy`, `opencode`): version + trạng thái đăng nhập; Agenium không xử lý đăng nhập, người dùng tự login bằng CLI của mình. Cách ly không cần clone binary: không bao giờ ghi vào config global của người dùng (`~/.claude`, `~/.codex`, ...); chỉ dùng lại auth của người dùng (read-only). Mỗi run Agenium tự cấp: MCP = MCP nội bộ Agenium + MCP riêng của agent; không hook; không memory/session resume của CLI; không file instruction global của người dùng. Ưu tiên cờ CLI bỏ qua config global (vd. cờ strict MCP config); nếu không có thì thư mục config tạm theo run, auth trỏ read-only về bản gốc. Binary pin trong `<root>/bin/` chỉ khi CLI không cách ly được hoặc cần ghim version. Cờ/env chính xác cho từng CLI: cần kiểm chứng ở M2.5. Process CLI chạy trong sandbox Agenium, tắt network/web tool riêng của CLI; usage đọc từ JSON stream. Khai báo phiên bản tối thiểu cho từng CLI + contract test trong CI; CLI không tương thích thì agent rơi về ApiBackend

<a id="adr-31"></a>
## ADR-31: Local / custom endpoint

- **Trạng thái:** Accepted (2026-10-22)
- **Quyết định:** Hỗ trợ chính thức trong MVP: provider "Local / custom endpoint" qua adapter OpenAI-compatible (Ollama, LM Studio, llama.cpp server, vLLM); base URL tùy chỉnh, key tùy chọn lưu trong OS keyring; danh sách model lấy từ endpoint; không có giá (cost 0); capability flags (tools, vision, context length) tự phát hiện hoặc đặt tay; cảnh báo nếu model không hỗ trợ tool calling; network allowlist tự thêm endpoint local đã cấu hình (localhost)

<a id="adr-32"></a>
## ADR-32: Xóa agent

- **Trạng thái:** Accepted (2026-10-22)
- **Quyết định:** Chỉ xóa hẳn, không có archive. Hộp thoại xác nhận mạnh: cảnh báo toàn bộ memory, skill tự học, lịch sử SOUL/ROLE và workspace của agent sẽ bị xóa vĩnh viễn; bắt gõ tên agent; bị chặn khi agent còn ticket mở (phải giao lại trước); có tùy chọn export trước khi xóa

<a id="adr-33"></a>
## ADR-33: Giấy phép

- **Trạng thái:** Accepted (2026-10-22)
- **Quyết định:** Mã nguồn mở Apache-2.0 (tương thích code port từ Codex, giữ NOTICE)

<a id="adr-34"></a>
## ADR-34: An toàn & vận hành bổ sung

- **Trạng thái:** Accepted (2026-10-22)
- **Quyết định:** Prompt injection: nội dung ngoài (fetch, MCP, file trong repo) đánh dấu untrusted trong context, không thể tự cấp quyền hay đổi allowlist. Quét secret (kiểu gitleaks) trước auto-commit và push, phát hiện thì chặn + báo. Model compact context = model của agent, có thể đặt model rẻ hơn. Bảng giá đi kèm bản cập nhật app, sửa được trong root. SOUL/ROLE/skill mặc định của template do Agenium viết, có version, kèm eval test nhỏ. Một người dùng, một máy; chuyển máy qua export/import `.agenium`

<a id="adr-35"></a>
## ADR-35: Learning

- **Trạng thái:** Accepted (2026-10-22)
- **Quyết định:** Học từ lỗi (Reopened, pre-merge fail, tool error, Lead sửa), ý tưởng tốt (Lead khen, cách làm hiệu quả), phản hồi người dùng. Lesson là memory fact có scope (vendor/product/version/language/project/domain) + cue + evidence + confidence; khớp chính xác mới áp dụng, gần khớp chỉ là gợi ý cần kiểm chứng; superseded không xóa; decay khi không dùng. Skill base bất biến; agent tiến hóa overlay copy-on-write riêng tại `<root>/agents/<id>/skills/<name>/` (base_version, diff, changelog, evidence, nhãn `updated`); `propose_skill_update` tự áp vào overlay của chính agent khi lesson validated ≥ 2 ngữ cảnh khác nhau, không cần người dùng duyệt, có audit log, không thông báo từng thay đổi (chỉ hiển thị trong UI, xem diff/revert bất cứ lúc nào); decay half-life 90 ngày không dùng, ẩn khi confidence < 0,2; review sau ticket chỉ khi có tín hiệu (Reopened, merge/pre-merge fail, Lead khen/sửa, phản hồi người dùng); Lead không ghi lesson vào memory của worker, chỉ góp ý qua Review, worker tự rút lesson; base cập nhật → overlay cần rebase (3-way merge); người dùng xem diff, revert, hoặc duyệt promote thành base mới

<a id="adr-36"></a>
## ADR-36: Giao thức Agent ↔ Hệ thống

- **Trạng thái:** Accepted (2026-10-22)
- **Quyết định:** Chỉ tool call đã validate theo schema mới được ghi; text tự do không phải hành động; lỗi trả về làm tool result, 3 lỗi liên tiếp dừng run, thiếu tool kết thúc thì nudge. Work order cố định theo thứ tự cache. Lead thấy team roster + `get_team`; worker chỉ thấy Lead + role mình. Tool tách theo role (Solo = Lead + Worker). Worker không kéo việc: Lead `assign_ticket`, scheduler khởi động run. `submit_report` → Resolved; `review_ticket` approve → merge queue (nếu có code), reopen kèm feedback theo AC. Merge không tự đóng: Lead quyết Closed bằng `close_ticket` (có reason, System thực thi); có code thì merge thành công là điều kiện bắt buộc, ticket không có code (vd. research) Lead đóng trực tiếp; Lead không chắc thì hỏi người dùng. Không có `split_ticket`: ticket quá lớn thì Lead `revise_plan` (có version), tạo ticket mới cho plan item mới và thu hẹp scope ticket gốc. Không có `request_reassign`: worker báo vướng (thiếu MCP/credential/skill, việc ngoài role) bằng `ask_question`, Lead quyết. Phụ thuộc: ticket phụ thuộc được mở khóa ngay khi ticket trước được Lead approve (`review_ticket` approve, cờ `approved`) và, nếu có code, merge thành công (cờ `merged` trên ticket Resolved); không cần chờ `close_ticket`, không có trạng thái Merging

<a id="adr-37"></a>
## ADR-37: Spike M0 trước khi dựng khung

- **Trạng thái:** Accepted (2026-10-22)
- **Quyết định:** Mốc M0-spike (tối đa 10 ngày) trả lời bằng code bỏ đi: S0 build môi trường sạch, S1 SurrealDB (cổng), S2 sandbox Windows (cổng), S3 sandbox macOS/Linux, S4 worktree/merge queue/reconcile (cổng), S5 keyring, S6 embedding, S7 realtime → UI. Ngưỡng Pass/Yellow/Kill chốt trước khi chạy, timebox cứng (hết giờ = Yellow), đo trên máy tối thiểu (quyết định 24), evidence tái lập được, mỗi spike một biên bản + ADR. Phương án B ghi sẵn ở quyết định 1, 5, 9. Cổng Go/No-Go ngày 10; kết quả cập nhật ngược vào Page trong tuần **[CẬP NHẬT ADR-39: Windows hoãn khỏi bản đầu (chỉ Linux + macOS); phần Windows/3 OS ở đây là thiết kế hoãn hoặc lịch sử]**

<a id="adr-38"></a>
## ADR-38: Kiểm thử & eval

- **Trạng thái:** Accepted (2026-10-22)
- **Quyết định:** Ngoài test tất định (quyết định 29): bộ golden task cố định chấm tự động + người chấm, bắt đầu 5 task ở M1 và tăng dần, chạy lại sau mỗi thay đổi lớn về prompt/orchestrator/model. Trang chẩn đoán (run log có cấu trúc theo ticket) và nút xuất gói hỗ trợ (log đã che secret + cấu hình + phiên bản), không gửi tự động

## ADR-39 … ADR-51: quyết định bổ sung khi triển khai M0 (2026-10-23)

Bản đầy đủ (tiếng Anh) nằm trong repo code: `agenium-app/docs/adr/ADR-39 … ADR-51` và chỉ mục `docs/adr/README.md`. Tóm tắt:

<a id="adr-39"></a>**ADR-39 — Bản đầu chỉ Linux + macOS.** Windows hoãn: không CI, không bundle, không backend sandbox (fail-closed). Sửa ADR-3a, ADR-9, ADR-24. Hướng dẫn thử nghiệm: `docs/WINDOWS-GUIDE.md`.

<a id="adr-40"></a>**ADR-40 — Ngôn ngữ.** Người dùng nói với Lead/Solo bằng ngôn ngữ tùy chọn (`app.user_language`). Từ Lead trở xuống mọi nội dung (ticket, plan, handoff, memory, skill, tin nhắn giữa agent, tool-call) chỉ tiếng Anh. Sửa ADR-2c.

<a id="adr-41"></a>**ADR-41 — Sandbox.** Vendoring mã Codex không sửa (chỉ tham chiếu, các file Rust phụ thuộc crate nội bộ), mã của Agenium nằm ở crate wrapper `agenium-sandbox` sau trait `Sandbox`; script `scripts/sync-codex-sandbox.sh`; `docs/SANDBOX-MAINTENANCE.md`.

<a id="adr-42"></a>**ADR-42 — Linux sandbox.** bubblewrap ưu tiên, hoặc Landlock ABI ≥ 3; thiếu cả hai ⇒ từ chối chạy agent (fail-closed).

<a id="adr-43"></a>**ADR-43 — macOS Seatbelt profile v2.** Thử nghiệm; chạy bằng `.app` bundle (binary thô hiển thị webview trắng); chưa ký/notarize.

<a id="adr-44"></a>**ADR-44 — VCS qua git CLI** (không git2/gix); intent ghi journal trước; fault injection trong process.

<a id="adr-45"></a>**ADR-45 — IntentJournal + EventSink trong `agenium-core`.** Realtime bằng `tokio::broadcast`; áp dụng cho cả SQLite.

<a id="adr-46"></a>**ADR-46 — Backup/migration.** Full + differential incremental; retention 24 giờ/7 ngày/4 tuần + 3 pre-migrate. Trên SQLite (ADR-49): full là snapshot image (online-backup API, zstd), migration chạy trong một transaction.

<a id="adr-47"></a>**ADR-47 — Trait `Embedder`**, model mặc định giữ nguyên; số chiều cố định theo DB agent.

<a id="adr-48"></a>**ADR-48 — `xtask` là workspace member; kiểu IPC TS viết tay ở M0** (tauri-specta hoãn, có contract test).

<a id="adr-49"></a>**ADR-49 — SQLite là store chính (chủ sở hữu, 2026-10-23 12:15 Asia/Saigon).** `rusqlite` bundled + `sqlite-vec` + FTS5; `db/core.db` + `db/agents/<id>.db`; một writer tuần tự + 4 reader; WAL; `core.db` synchronous=FULL, DB agent NORMAL. SurrealDB là backend tùy chọn sau cargo feature `surrealkv` (đối chứng; cùng bộ test và differential test). **Thay thế ADR-1.** Bằng chứng: `docs/STORE-PARITY.md` (parity 0 sai khác trên 3 seed × 3000 thao tác; crash `kill -9` ×20: 0 mất, 0 lỗ seq).

<a id="adr-50"></a>**ADR-50 — Vector index và quy mô.** `VectorIndex`: Exact (sqlite-vec, đúng tuyệt đối) + ANN (usearch HNSW f16 sidecar). `IndexMode::Auto` chuyển sang ANN ở **100 000 fact/agent** (cấu hình được; chủ sở hữu duyệt 2026-10-23 15:46). `expansion_search` 128, tự nâng 256 từ 200 000 vector. Reader pool công bằng + `mmap_size` 1 GiB + WAL gate 64 MB. Retention event log: archive `.jsonl.zst`. Số liệu: `docs/STORE-SCALE.md`.

<a id="adr-51"></a>**ADR-51 — Graph memory trên SQLite (Proposed, 2026-10-23; chỉ thiết kế, chưa cài).** Bổ sung ADR-2 (không thay): memory vẫn riêng tư theo agent, nhưng thêm đồ thị quan hệ trong **cùng DB `agent_<id>.db`**: bảng `entity` (type, name, aliases, scope vendor/project/language, confidence, evidence, created/updated, `valid_from/valid_to/superseded_by`), `edge` (kiểu cạnh: is_a, part_of, depends_on, uses, configured_with, causes, fixes, workaround_for, conflicts_with, applies_to, replaced_by, observed_in, related_to; có scope + thời hạn hiệu lực + evidence), `fact_entity`, `fact_usage` (tín hiệu tái sử dụng), `entity_merge_log`. Trích xuất bằng LLM (chỉ tiếng Anh; nội dung từ web/tool là `untrusted`, không tạo lesson). Gộp entity: alias → FTS → embedding (≥ 0.90 tự gộp, 0.80–0.90 hỏi LLM; ngưỡng tạm, chờ benchmark; không gộp khác type/scope/version). **Lesson có scope:** key scope lệch với ngữ cảnh → loại cứng (bài học TP-Link không áp cho Netgear); thiếu key → phạt mềm. Truy vấn: `WITH RECURSIVE` hai nhánh (cạnh ra/vào) dùng `INDEXED BY edge_out/edge_in`, giới hạn độ sâu ≤ 3 (mặc định 2), chống chu trình (path-string hoặc `UNION`), trần hub (degree > 200 không mở rộng), giới hạn thời gian 25 ms; **không dùng VIEW UNION ALL** (đo ≈ 300 ms/truy vấn = quét toàn bảng mỗi bước). Truy xuất lai: vec KNN + FTS5 BM25 + mở rộng đồ thị 1–2 hop → RRF (k=60) → xếp lại theo scope × recency × confidence × usage. Luồng tái sử dụng: truy xuất trước ticket, ghi sau ticket; ticket Reopened và sửa của Lead là tín hiệu âm; quên/hợp nhất định kỳ (cạnh/entity không xóa, chỉ `valid_to`). **Chia sẻ giữa agent (cần chủ sở hữu duyệt, đụng quyết định 2):** (A) riêng tư mặc định; (B) Lead gửi `knowledge_note` qua comment ticket — khuyến nghị bật; (C) gói tri thức project `<project>/.agenium/knowledge/*.jsonl` do Lead/người dùng duyệt, nhập read-only, quét secret — khuyến nghị, opt-in; (D) DB chung bị từ chối. So sánh: SurrealDB `->edge->` gọn hơn nhưng ta giữ SQLite (ADR-49); Mem0 graph: ngoài DB graph riêng, không scope ứng dụng (mô tả theo trí nhớ, chưa kiểm chứng trong môi trường này). Số liệu khả thi (probe python, đồ thị tổng hợp, KHÔNG phải store Rust): 1M cạnh, độ sâu 3 p50 ≈ 1–2 ms, p95 ≤ 16 ms; hub 1 761 cạnh vào: 3 hop = 319 ms (nên cần trần hub). Kế hoạch benchmark (100k/1M cạnh, hop 1–3, p50/p95, golden set multi-hop QA, parity SurrealDB nếu khả thi), tiêu chí chấp nhận, golden task GM-1…7, kế hoạch test: ADR gốc `docs/adr/ADR-51-graph-memory-on-sqlite.md`, `docs/STORE-SCALE.md` §8. Vị trí milestone: M1 hook (`MemoryRetriever` trong harness + log `fact_usage`; không thêm tool giao thức), **M1.5 lõi graph + benchmark**, **M4.5 tín hiệu/consolidation/gói tri thức**; benchmark song song M2. DDL nháp: `SCHEMAS/sql/drafts/agent_003_graph_memory.sql` (chưa áp dụng).

## Mẫu ADR mới

```text
ADR-<n>: <tiêu đề>
Ngày / Trạng thái: Proposed | Accepted | Superseded by ADR-<m>
Bối cảnh:
Quyết định:
Hệ quả:
Evidence (spike, số đo):
Thay thế / sửa ADR:
```
