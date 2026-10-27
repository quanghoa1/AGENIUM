<p align="center">
  <img src="docs/brand/agenium-logo-banner-dark.svg" alt="AGENIUM — Ticket-Driven Multi-Agent Orchestrator" width="100%" />
</p>

# Agenium

**Agenium điều phối một nhóm agent AI viết code theo cách một đội kỹ sư giỏi tự vận hành: qua ticket.**
Bạn nói với Lead của nhóm điều bạn muốn, bằng ngôn ngữ của bạn. Lead viết bản đặc tả (spec) để bạn duyệt, chia thành
kế hoạch và ticket, rồi giao mỗi ticket cho agent có vai trò phù hợp. Mỗi agent làm trong một git worktree riêng;
không gì vào được `main` nếu chưa được review theo tiêu chí nghiệm thu của ticket và merge qua một hàng đợi duy nhất.
Bạn theo dõi mọi thứ trong ứng dụng desktop, trả lời khi agent hỏi, và luôn là người quyết định cuối cùng.

Một nhóm có thể trộn nhiều loại model: model qua API (Anthropic, OpenAI, Gemini, DeepSeek, OpenRouter, Azure, Bedrock,
LM Studio / Ollama chạy trên máy, hoặc endpoint OpenAI-compatible của riêng bạn) và các CLI lập trình bạn đã đăng nhập
sẵn (Claude Code, Codex, Gemini/Antigravity, Grok, OpenCode). Ví dụ: Lead là Claude, worker là Codex, Grok và DeepSeek.

[English](README.md) · [Tình trạng và giới hạn](docs/PROGRESS.md) · [Đã kiểm chứng những gì](docs/VERIFICATION.md) ·
[Jev](docs/JEV.md) · [Changelog](CHANGELOG.md) · [Các quyết định (ADR)](docs/adr/README.md)

> **Tình trạng: 0.1.0, bản tiền phát hành.** Đủ tính năng cho bản phát hành đầu trên Windows và Linux (Ubuntu); bộ cài
> đang được build và thử trên máy ảo sạch. macOS đã chạy đủ bộ test; bản desktop cho macOS đi sau.

## Cách hoạt động

```
 Bạn ──chat──▶ Lead ──spec──▶ Bạn duyệt ──▶ Lead: kế hoạch + ticket ──▶ Worker (mỗi ticket một worktree)
                 ▲                                                          │
                 │          review theo tiêu chí nghiệm thu ◀── báo cáo ────┘
                 └── câu hỏi, xin quyền ◀── Worker                          │
                                                  đạt ──▶ hàng đợi merge ──▶ main ──▶ ticket Closed
```

- **Ticket là kênh duy nhất để giao việc.** Agent chỉ đổi trạng thái qua các lệnh có kiểu và được kiểm tra
  (`create_ticket`, `submit_report`, `ask_question`, `review_ticket` …). Mọi thay đổi là một sự kiện trong log, nên bảng
  Kanban, lịch sử kiểm toán và màn hình trực tiếp luôn khớp nhau, và app có sập cũng không mất gì.
- **Kế hoạch đo được.** Spec có các yêu cầu đánh số; mỗi mục kế hoạch truy về một yêu cầu; mỗi ticket có tiêu chí nghiệm
  thu; review kiểm từng tiêu chí với bằng chứng. Ticket không đạt bị mở lại kèm nhận xét từng tiêu chí; trượt nhiều lần
  thì được chuyển lên bạn.
- **Phụ thuộc là thật.** Ticket cần ticket khác sẽ chờ tới khi ticket kia được merge; bảng hiển thị ticket nào chặn
  ticket nào.
- **Bạn được hỏi, không bị bất ngờ.** Agent hỏi bạn qua Lead, xin quyền cho mọi thứ ngoài sandbox, và thay đổi phạm vi
  luôn chờ bạn đồng ý.

## Tính năng

| Mảng | Nội dung |
|---|---|
| **Nhóm** | Agent Solo (vừa Lead vừa worker) hoặc Lead cùng các worker và một planner tùy chọn. Mẫu nhóm cho lập trình, nghiên cứu, nội dung và kiểm thử bảo mật; 27 vai trò dựng sẵn (backend, frontend, kiểm thử API, kiểm thử giao diện, review code, viết tài liệu, SRE …); tự ghép nhóm từ các agent của bạn. |
| **Mọi model, trộn được** | Nhà cung cấp API với cấu hình sẵn và endpoint OpenAI-compatible tùy chỉnh (header riêng, cài đặt theo model, trường bổ sung trong request — [ADR-115](docs/adr/ADR-115-custom-endpoint-requests.md)). CLI của các hãng được điều khiển không giao diện, mọi lệnh gọi tool do Agenium quyết. Mức effort theo từng agent; model dự phòng khi CLI không chạy được. |
| **Ứng dụng desktop** | Tổng quan, chat với Lead, bảng Kanban, trang ticket với diff hai cột (Monaco), văn phòng trực tiếp xem ai đang làm gì, log từng lượt chạy dạng terminal, trình xem memory, sắp xếp nhóm, cài đặt. Tiếng Anh và tiếng Việt. |
| **An toàn** | Lệnh của agent chạy trong sandbox của hệ điều hành (Windows MXC, Linux bubblewrap, macOS Seatbelt): chỉ ghi được trong worktree của ticket, không chạm được bí mật của bạn. Bí mật nằm trong vault mã hóa, khóa giữ bởi keyring của hệ điều hành, không bao giờ tới agent, log hay git; mọi commit được quét bí mật. |
| **Memory biết học, có kiểm soát** | Mỗi agent có memory (sự kiện, bài học, đồ thị tri thức), tìm bằng model embedding chạy trên máy (bge-small, cài cùng app) cộng tìm từ khóa và tìm theo đồ thị. Skill và bài học mới phải có bằng chứng và qua các cổng kiểm tra chặt, để agent không bị trôi. |
| **Skill và tool** | Skill `SKILL.md` theo vai trò hoặc theo agent (13 skill có sẵn), MCP server (stdio, Streamable HTTP, SSE cũ, OAuth) gắn cho từng agent với thông tin đăng nhập riêng, xem ảnh cho model có thị giác (`view_image`). |
| **Chi phí và kiểm soát** | Token và chi phí ước tính theo lượt chạy, ticket và agent; prompt caching; nén ngữ cảnh cho lượt chạy dài; tạm dừng, hủy, chạy lại. |
| **Vận hành** | Sao lưu theo giờ/ngày có giữ hạn, khôi phục sang root mới, migration database có sao lưu trước, gói hỗ trợ không chứa bí mật, báo cáo dự án dạng Markdown, tùy chọn push lên remote khi ticket đóng. |
| **Bộ phán đoán tùy chọn (Jev)** | Model Jev của TypeSafe nhận làm các quyết định nhỏ thay agent chính — ticket có đáng nhớ không, skill nào hợp — để agent chính đỡ tốn token. Mặc định tắt, lỗi thì bỏ qua. Xem [docs/JEV.md](docs/JEV.md). |

## Cài đặt và lần chạy đầu

**Ứng dụng desktop.** Tải bộ cài cho hệ điều hành của bạn ở trang Releases và chạy. Lần mở đầu tiên app chuẩn bị model
embedding (có sẵn trong bộ cài, không phải tải), sau đó một bước thiết lập ngắn hỏi ngôn ngữ và nơi lưu dữ liệu. Thêm
nhà cung cấp model (API key đi thẳng vào vault mã hóa) hoặc dùng CLI bạn đã đăng nhập, tạo dự án, chọn nhóm và bắt đầu
chat với Lead.

**Build từ mã nguồn** và **dùng bằng dòng lệnh**: xem [README tiếng Anh](README.md#install-and-first-run) — các lệnh
giống hệt nhau.

## Tình trạng

0.1.0 là bản ứng viên phát hành đầu tiên. Đã kiểm chứng (chi tiết trong [docs/PROGRESS.md](docs/PROGRESS.md) và
[docs/VERIFICATION.md](docs/VERIFICATION.md)):

- **438 test tự động** (unit, contract test với server HTTP giả, test runtime trọn đường với model giả lập, test giết
  tiến trình giữa lúc ghi), clippy sạch, chính sách dependency kiểm bằng `cargo-deny`, smoke test 25 bước điều khiển app
  thật.
- **Model thật:** bài coding mẫu 5/5 trên DeepSeek (API) và qua CLI của Claude, Codex, Gemini (Antigravity), Grok và
  OpenCode; chạy trọn một nhóm (spec → kế hoạch → ticket → review → merge) trên DeepSeek và trên nhóm hỗn hợp Claude Opus
  (Lead), Codex, Grok, Gemini và DeepSeek.
- **Nền tảng:** Windows 11 (máy phát triển và một máy ảo sạch), Ubuntu 25.10 (máy ảo thật, test thoát sandbox: 0 lần
  thoát), macOS (máy ảo Apple Silicon: đủ bộ test, kiểm tra cô lập sandbox).

Giới hạn của 0.1.0: API Anthropic, Gemini và OpenAI mới qua contract test, chưa gọi bằng key thật; trường bổ sung trong
request (ADR-115) chỉ áp dụng cho endpoint OpenAI-compatible; CLI của Devin không điều khiển được; bộ cài chưa được ký
số.

## Giấy phép

Apache-2.0 — xem [LICENSE](LICENSE) và [NOTICE](NOTICE).
