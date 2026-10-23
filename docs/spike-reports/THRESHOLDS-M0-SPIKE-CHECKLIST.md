> **Agenium v1.0 – FINAL (2026-10-22).** Checklist này là nguồn chi tiết cho mốc M0-spike (quyết định 37). Phương án B đã ghi ngược vào quyết định 1 (SQLite + sqlite-vec), 5 (khóa chủ + vault mã hóa), 9 (WSL2 → Docker/VM → experimental). Kết quả S6 dùng để xem lại quyết định 2b/2c (giữ English-only cho tới khi có số đo).

# Agenium — Checklist Spike M0

> **Mục đích:** Trả lời bằng code thật, trong tối đa **10 ngày làm việc**, hai câu hỏi quyết định kiến trúc Agenium có đứng vững không: (1) SurrealDB embedded có đáng tin làm kho lưu trữ duy nhất không, và (2) sandbox kiểu Codex có thật sự nhốt được agent YOLO trên 3 hệ điều hành không — cộng 5 rủi ro phụ có khả năng làm vỡ M0/M1 nếu phát hiện muộn.
>
> **Sản phẩm của spike không phải code dùng lâu dài.** Sản phẩm là *quyết định có bằng chứng*: mỗi spike kết thúc bằng một biên bản 1 trang (Pass / Yellow / Red + số liệu + link evidence) và một ADR (Architecture Decision Record) chốt đi tiếp hay chuyển phương án B.

**Ngày soạn:** 22/10/2026 · **Liên quan:** Agenium-Kien-truc.md, quyết định 1, 2b, 5, 9, 12, 14, 22, 23, 29

---

## 0. Luật chơi — đọc trước khi bắt đầu

- [ ] **Chốt ngưỡng trước khi chạy.** Mọi ngưỡng Pass/Kill trong tài liệu này phải được duyệt (hoặc sửa) *trước* khi chạy spike tương ứng, và ghi ngày chốt vào biên bản. Không nới ngưỡng sau khi thấy kết quả — đó là tự lừa mình.
- [ ] **Timebox cứng.** Hết timebox mà chưa có kết luận = tính là **Yellow** (chưa chứng minh được), không tự gia hạn. Gia hạn phải có quyết định riêng và trừ vào ngân sách của spike khác.
- [ ] **Code spike là code bỏ.** Repo riêng (`agenium-spikes/`), không dùng khung production, không refactor đẹp. Chỉ giữ lại: fixture, script đo, và kết quả đo.
- [ ] **Đo trên máy yếu nhất đã công bố.** Mọi con số hiệu năng đo trên cấu hình tối thiểu trong quyết định 24 (4 nhân, 8 GB RAM, SSD) *và* ghi kèm cấu hình máy đo thực tế. Đo trên máy mạnh của dev rồi ngoại suy là không chấp nhận.
- [ ] **Môi trường sạch.** Các spike build/cài đặt phải chạy được trên máy ảo/OS cài mới chỉ có những gì tài liệu cài đặt của Agenium sẽ yêu cầu người dùng cài. "Máy dev của tôi chạy được" không phải bằng chứng.
- [ ] **Mọi kết quả đều phải có evidence tái lập được:** lệnh chạy, commit của repo spike, file kết quả thô (CSV/log), và với sandbox là video hoặc log của từng test xâm nhập.
- [ ] **Red ở tiêu chí toàn vẹn dữ liệu (S1) hoặc thoát sandbox nghiêm trọng (S2/S3) = dừng M0 ngay**, áp dụng phương án B và ước lượng lại lộ trình, không "sửa tiếp xem sao" trong âm thầm.

### Thứ tự ưu tiên và lịch 10 ngày (1 dev; 2 dev thì chạy song song Track A/B)

| Ngày | Track chính | Nội dung |
|---|---|---|
| 1 | A | S0 môi trường + bắt đầu S1 (build SurrealDB trên Windows sạch) |
| 2–3 | A | S1 SurrealDB: tải, vector, live query, crash, backup/restore |
| 1–3 | B (nếu có dev 2) | S2 Sandbox Windows: audit code Codex hiện tại + test xâm nhập |
| 4–6 | A/B | S2 tiếp + S4 Git worktree/merge queue/crash reconcile |
| 7 | A/B | S3 Sandbox macOS + Linux (ma trận rút gọn của S2) |
| 8 | A | S5 Keyring & secret trên 3 OS |
| 9 | A | S6 Embedding/memory + S7 Đường live query → UI |
| 10 | Chung | Tổng hợp biên bản, cổng quyết định Go/No-Go (mục 9), viết ADR |

> Nếu chỉ có 1 dev và phải cắt: giữ **S1, S2, S4, S5**. S3, S6, S7 có thể trượt sang đầu M1 — nhưng S3 phải xong trước khi công bố "đa nền tảng" ở bất kỳ đâu.

### Thông tin đã kiểm chứng lại khi soạn checklist này (22/10/2026) — làm thay đổi giả định gốc

1. **SurrealKV đã ra 1.0** (13/10/2026) và SurrealDB mới nhất là **3.3.4** (13/10/2026). Giả định trong tài liệu gốc "SurrealKV ổn định ở mốc 3.4/1.0" đã tới — spike phải so trực tiếp RocksDB vs SurrealKV trên cùng fixture, không mặc định dùng RocksDB nữa.
2. Nhưng nhịp sửa lỗi của 3.x vẫn rất dày và đáng chú ý: 3.3.4 sửa lỗi truy vấn *trả sai dòng/trả thiếu dòng*, 3.2.5 sửa lỗi vector search *bỏ sót record* và REBUILD INDEX không kết thúc, 3.1 sửa một loạt lỗi live query. Đây đúng là các lỗi spike phải cố tái hiện — đặc biệt là kiểm chứng kết quả vector bằng brute-force đối chiếu.
3. **Sandbox Windows của Codex đã thay đổi kiến trúc trong 2026:** bản hiện tại dùng *hai local user riêng* (offline/online), một binary `codex-command-runner`, một bước setup cần quyền nâng cao để tạo user và kiểm tra firewall, và DPAPI. Bản thử đầu tiên (restricted token + ép proxy bằng biến môi trường) bị chính OpenAI kết luận là *chặn mạng kiểu "advisory" — process mở socket trực tiếp là qua mặt được*. Hệ quả: phần Windows trong quyết định 9 đang mô tả một thiết kế cũ và nhẹ hơn thực tế nhiều; S2 phải bắt đầu bằng audit lại code Codex hiện tại.
4. **Windows Credential Manager có giới hạn blob rất nhỏ** (CRED_MAX_CREDENTIAL_BLOB_SIZE — 2.560 byte cho generic credential; qua mã hóa UTF-16 của crate `keyring` giới hạn ký tự hiệu dụng còn thấp hơn). Khóa SSH ed25519 (~400 byte) nhiều khả năng vừa, nhưng token OAuth JSON vài KB hoặc khóa RSA thì không. Phải *đo bằng đúng crate `keyring` và đúng loại secret Agenium định lưu*, không suy luận.

---

## S0 — Môi trường & baseline build (0,5 ngày, điều kiện tiên quyết)

**Câu hỏi:** Một máy sạch có dựng được toàn bộ toolchain Agenium không, và cái giá (thời gian build, kích thước binary) là bao nhiêu?

- [ ] Dựng 3 môi trường sạch: Windows 11 (VM), macOS 12+, Ubuntu LTS (kernel ≥ 5.13). Ghi lại từng bước cài đặt phải làm bằng tay.
- [ ] `cargo build --release` một crate Tauri 2 rỗng + dependency `surrealdb` (bật lần lượt `kv-rocksdb` và `kv-surrealkv`), `fastembed`, `keyring`, `git2/gix` — trên cả 3 OS.
- [ ] Đo: thời gian build sạch, thời gian build tăng dần (incremental), kích thước binary release, dung lượng `target/`.
- [ ] Ghi lại mọi yêu cầu hệ thống *phát sinh thêm* ngoài quyết định 24 (MSVC Build Tools, LLVM/clang cho RocksDB, WebKitGTK trên Linux...).

| Tiêu chí | Pass | Kill |
|---|---|---|
| Build trên máy sạch | Cả 3 OS build thành công chỉ với các bước ghi trong tài liệu cài đặt dự kiến | Bất kỳ OS nào cần workaround không ghi thành tài liệu được, hoặc build hỏng lặp lại |
| Thời gian build sạch (máy tối thiểu) | ≤ 45 phút | > 90 phút (CI và vòng lặp dev không chịu nổi) |
| Tăng kích thước binary do SurrealDB | Đo được và ≤ 150 MB | > 250 MB mà không có cách cắt feature để giảm |

**Phương án B nếu Kill:** nếu thủ phạm là RocksDB → thử SurrealKV thuần Rust trước khi kết tội SurrealDB; nếu vẫn Kill → chuyển S1 sang đánh giá SQLite (xem phương án B của S1).

---

## S1 — SurrealDB embedded: kho sự thật duy nhất có đáng tin không (3 ngày) — **CỔNG QUYẾT ĐỊNH**

**Quyết định cần trả lời:** Chốt engine nào (RocksDB hay SurrealKV) — hay rời SurrealDB.
**Giả thuyết:** SurrealDB 3.3.x embedded đáp ứng đồng thời: truy vấn điều phối, vector search cho memory, live query cho UI, và khôi phục sau crash — ở quy mô một người dùng trên máy tối thiểu.

**Fixture (dùng chung cho mọi phép đo, commit vào repo spike):**
- `core`: 200 ticket, 5.000 comment, 50.000 event, 20 agent/team/project.
- `agent_x`: 10.000 fact, mỗi fact có embedding 384 chiều (sinh ngẫu nhiên có seed cố định + một tập 200 fact "thật" có nội dung để đo recall), quan hệ `superseded_by`.
- Script ghi liên tục (writer loop) để dùng cho test crash.

### Checklist

- [ ] **Đúng/sai của truy vấn:** chạy bộ truy vấn mẫu lấy từ use-case thật (ticket của worker X qua RELATE, comment theo ticket sắp theo thời gian, ticket theo plan_item, event theo seq). Đối chiếu kết quả với cùng dữ liệu nạp vào SQLite. Mọi sai lệch = ghi bug, phân loại mức độ.
- [ ] **Vector search có đối chiếu:** với 200 query, so kết quả HNSW top-10 với brute-force cosine trên cùng tập fact. Đo **recall@10**. Lặp lại sau khi restart DB và sau khi nạp thêm 1.000 fact.
- [ ] **BM25 + hybrid:** 50 query từ khóa trên fact tiếng Anh; kiểm tra kết quả có khớp mong đợi thủ công không.
- [ ] **Hiệu năng:** p50/p95 của: truy vấn ticket+comment, vector search top-10, hybrid search, ghi 1 comment + 1 event trong transaction. Đo lúc DB lạnh (vừa mở) và ấm.
- [ ] **Live query:** đăng ký `LIVE SELECT` trên ticket/comment; bơm 1.000 thay đổi liên tiếp; đếm số notification nhận được, độ trễ p95, có mất/trùng không; ngắt kết nối subscriber rồi nối lại.
- [ ] **Crash giữa chừng:** trong lúc writer loop đang ghi, `kill -9` (Windows: End task) 20 lần ở thời điểm ngẫu nhiên. Sau mỗi lần: mở lại DB, kiểm tra mở được không, dữ liệu đã commit có mất không, transaction dở có để lại trạng thái nửa vời không (đối chiếu event log vs trạng thái ticket).
- [ ] **Backup/restore:** export JSONL toàn bộ `core` + `agent_x` theo thiết kế quyết định 23, xóa DB, import vào root mới, so số bản ghi và hash nội dung từng bảng.
- [ ] **Hai engine đối đầu:** chạy lại toàn bộ các mục trên với `kv-surrealkv` và `kv-rocksdb`, lập bảng so sánh (đúng, nhanh, kích thước thư mục DB, thời gian mở DB).
- [ ] **Khóa file & đa tiến trình:** mở app lần hai khi DB đang bị tiến trình đầu giữ — thông điệp lỗi có hiểu được không, có làm hỏng dữ liệu không.

| Chỉ số | Pass | Yellow | Kill |
|---|---|---|---|
| Toàn vẹn sau crash (20 lần kill) | 0 lần hỏng DB, 0 lần mất dữ liệu đã commit | — | **Bất kỳ** lần nào hỏng/mất → Kill ngay |
| Sai lệch truy vấn so với đối chiếu | 0 | Sai ở truy vấn ít dùng, có workaround | Sai ở truy vấn điều phối cốt lõi |
| Recall@10 vector (sau restart) | ≥ 0,90 | 0,80–0,90 | < 0,80 hoặc sụt mạnh sau restart/nạp thêm |
| p95 vector search top-10, 10k fact, máy tối thiểu | ≤ 50 ms | 50–150 ms | > 150 ms |
| p95 truy vấn điều phối thường gặp | ≤ 100 ms | 100–300 ms | > 300 ms |
| Live query khi bơm 1.000 thay đổi | Mất 0, trễ p95 ≤ 200 ms | Mất < 1% và phát hiện được | Mất âm thầm ≥ 1% (UI sai mà không ai biết) |
| Export → import JSONL | Khớp 100% số bản ghi + hash | Chậm nhưng đúng | Mất/sai dữ liệu |
| Thời gian mở DB (10k fact + core như fixture) | ≤ 3 giây | 3–8 giây | > 8 giây |

**Phương án B (chốt sẵn để khỏi tranh luận khi Kill):** SQLite qua `rusqlite`/`sqlx` + `sqlite-vec` cho vector + FTS5 cho BM25; graph edge làm bằng bảng thường; realtime bằng `tokio::broadcast` của chính orchestrator (bỏ live query). Repo trait trong thiết kế gốc giữ nguyên — spike phải ghi rõ ước lượng công sức chuyển đổi đo được từ fixture này (số query phải viết lại).
**Evidence:** repo fixture + script đo, CSV kết quả từng phép đo cho cả 2 engine, log 20 lần crash test, biên bản 1 trang.

---

## S2 — Sandbox Windows (3 ngày) — **CỔNG QUYẾT ĐỊNH, rủi ro cao nhất**

**Quyết định cần trả lời:** Fork Codex hiện tại cần bao nhiêu công, có cần setup nâng quyền/tạo local user không, và mô hình đó có chấp nhận được với UX của Agenium không.
**Giả thuyết:** Một process chạy trong sandbox có thể làm trọn vẹn việc dev thường gặp trong worktree, nhưng *không* đọc/ghi được ra ngoài, *không* mở được mạng trực tiếp, và *không* sống sót khi run bị hủy.

### Bước 0 — Audit trước khi code (0,5 ngày, bắt buộc)

- [ ] Đọc code `codex-windows-sandbox` và tài liệu thiết kế Windows hiện tại của Codex; liệt kê: các thành phần (setup exe, command runner, sandbox users, firewall rule, DPAPI), bước nào cần admin, phần nào là mã nguồn mở dùng lại được, ước lượng số dòng phải fork/adapt.
- [ ] Chốt: Agenium chấp nhận cài đặt cần admin một lần không? Tạo thêm local user trên máy người dùng có chấp nhận được không? **Ghi thành quyết định trước khi làm tiếp** — đây là quyết định sản phẩm, không phải kỹ thuật.

### Ma trận test xâm nhập (mỗi test: chạy, ghi kết quả Đạt thủng/Chặn được + log)

- [ ] Ghi file ra ngoài worktree: `%USERPROFILE%\Documents`, Desktop, thư mục Agenium root, workspace của agent khác, worktree của ticket khác.
- [ ] Ghi vào `.git` của project; đọc file ngoài worktree: `~/.ssh`, config/DB của Agenium, tài liệu người dùng.
- [ ] Thoát qua đường vòng: symlink/junction trong worktree trỏ ra ngoài, đường dẫn UNC `\\localhost\C$`, đường dẫn dài > 260 ký tự, `..\` sau khi canonicalize.
- [ ] Mạng trực tiếp (test quan trọng nhất): mở raw socket tới IP ngoài (bỏ qua mọi biến môi trường proxy), phân giải DNS tới server tự chọn (kênh rò rỉ DNS), kết nối tới `localhost` các cổng dịch vụ nội bộ, IPv6.
- [ ] Sống sót: tiến trình con tách khỏi cây cha (detached/daemon), lên lịch chạy sau (scheduled task nếu quyền cho phép), ghi Run key registry.
- [ ] Rò rỉ môi trường: in toàn bộ env — có API key/secret nào của harness lọt vào không (đúng thiết kế là *không*).
- [ ] Hủy run: kill run giữa chừng 20 lần — cả cây tiến trình có chết hết không, có tiến trình mồ côi nào giữ file worktree không.

### Ma trận việc hợp lệ (sandbox chặt mà không làm được việc thì vô dụng)

- [ ] `git status/commit` trong worktree; `cargo build` một crate mẫu (có tiến trình con); `node`/`python` chạy script và test; shell tương tác qua PTY.
- [ ] Đo overhead: thời gian `cargo build` crate mẫu trong vs ngoài sandbox.

| Tiêu chí | Pass | Kill |
|---|---|---|
| Thoát ghi/đọc ra ngoài (gồm symlink/junction/UNC) | 0 test thủng | **Bất kỳ** test nào thủng ở cấu hình mặc định |
| Mạng trực tiếp (raw socket, DNS, localhost) | Tất cả bị chặn ở tầng OS/firewall, không dựa vào env/proxy | Raw socket hoặc DNS lọt → Kill (đây là lỗi bản Codex đời đầu đã mắc) |
| Kill cả cây tiến trình | 20/20 lần sạch | Còn tiến trình mồ côi ≥ 1 lần mà không tự dọn được |
| Việc hợp lệ | Tất cả chạy được | Việc dev cơ bản (cargo/node) không chạy nổi trong sandbox |
| Overhead build | ≤ 30% so với ngoài sandbox | > 60% |
| Điều kiện cài đặt | Admin (nếu cần) chỉ 1 lần lúc cài; chạy hằng ngày không cần | Cần admin mỗi run, hoặc phải tắt antivirus/firewall |

**Phương án B (theo thứ tự ưu tiên):** (1) chạy backend thực thi trong **WSL2** cho project Windows cần cách ly mạnh; (2) **Docker/VM nhẹ** theo project; (3) phát hành Windows với nhãn *"experimental — chỉ có policy ghi-trong-workspace, không tuyên bố cách ly mạng"* và tắt YOLO mặc định trên Windows cho tới khi sandbox đạt. Chọn (3) thì phải sửa cả thông điệp sản phẩm — không được bán "YOLO an toàn" trên Windows.
**Evidence:** bảng kết quả từng test xâm nhập kèm log, báo cáo audit Bước 0 (số dòng fork, điều kiện admin/user), số đo overhead.

---

## S3 — Sandbox macOS + Linux (2 ngày, ma trận rút gọn của S2)

**Quyết định cần trả lời:** Seatbelt (macOS) và Landlock + seccomp (Linux) có đạt cùng mức chặn như Windows không, và máy Linux "yếu"/kernel cũ xử lý thế nào.

- [ ] Chạy lại toàn bộ ma trận xâm nhập của S2 (ghi ngoài, symlink, mạng trực tiếp/DNS/localhost, sống sót sau kill) trên macOS (profile Seatbelt sinh động) và Linux (Landlock + seccomp).
- [ ] Linux: kiểm tra kernel 5.13 (mức tối thiểu công bố) — Landlock ABI ở mức đó thiếu gì so với kernel mới; hành vi khi Landlock không khả dụng: **fail-closed (từ chối chạy YOLO) hay fail-open (chạy không sandbox)?** Chốt fail-closed cho mọi trường hợp thiếu cơ chế.
- [ ] Linux: thử phương án bubblewrap dự phòng — có sẵn trên các bản phân phối phổ biến không, cài thêm có cần root không.
- [ ] macOS: kiểm tra `sandbox-exec` trên macOS 12 (mức tối thiểu) và bản macOS hiện hành — profile có hoạt động giống nhau không (Apple coi đây là API deprecated nhưng Codex vẫn dùng; ghi rõ rủi ro dài hạn).
- [ ] Việc hợp lệ + overhead: cùng bộ test như S2, ngưỡng overhead ≤ 30%.

| Tiêu chí | Pass | Kill |
|---|---|---|
| Test xâm nhập nghiêm trọng | 0 thủng trên cả 2 OS | Bất kỳ thủng nào ở cấu hình mặc định |
| Thiếu cơ chế (kernel cũ, không Landlock) | Fail-closed + thông báo rõ cho người dùng | Fail-open âm thầm |
| Parity với Windows | Cùng danh sách hành vi bị chặn | Lệch nhau ở hành vi nghiêm trọng (vd. mạng) mà không ghi thành tài liệu được |

**Phương án B:** Linux không đạt → bubblewrap/nsjail làm backend chính; macOS không đạt → chỉ hỗ trợ macOS từ bản có Seatbelt hoạt động đúng, nâng mức OS tối thiểu (sửa quyết định 24).

---

## S4 — Git worktree, merge queue & reconcile sau crash (2 ngày) — **CỔNG QUYẾT ĐỊNH**

**Quyết định cần trả lời:** Mô hình "mỗi ticket một worktree + merge queue tuần tự + ghi event trước, thao tác git idempotent" có thực sự tự lành sau crash ở mọi điểm rơi không.
**Giả thuyết:** Với 8 điểm tiêm lỗi dưới đây, hệ thống luôn quay về được trạng thái nhất quán mà không mất việc đã commit và không cần người dùng sửa git bằng tay.

**Fixture:** repo mẫu 500 file, 4 worktree đồng thời, một hàng đợi merge có chủ đích tạo: 1 merge sạch, 1 conflict nhỏ (1 file), 1 conflict lớn, 1 pre-merge check fail.

### Checklist

- [ ] Tạo/xóa worktree hàng loạt (50 lần): có rác branch/thư mục không; đường dẫn worktree trên Windows có chạm giới hạn 260 ký tự với tên project dài không; `.agenium/` đã vào `.git/info/exclude` đúng chưa.
- [ ] 4 ticket làm song song sửa các file không giao nhau → merge tuần tự: thứ tự đúng, không race.
- [ ] Hai ticket sửa cùng file mà plan *không* khai `depends_on`: hệ thống phát hiện ở bước nào (assign? commit? merge?) — ghi lại, vì thiết kế gốc chỉ phòng ngừa bằng `scope_paths` khai báo tay.
- [ ] **Tiêm lỗi tại 8 điểm** (kill tiến trình/hệ thống mô phỏng ngay sau từng bước): (1) sau khi ghi event `MergeStarted`, (2) giữa rebase, (3) sau rebase trước pre-merge check, (4) giữa pre-merge check, (5) sau merge trước khi ghi `MergeDone`, (6) giữa auto-commit khi Resolved, (7) giữa chừng xóa worktree khi Closed, (8) khi đang push remote. Sau mỗi điểm: khởi động lại, chạy reconcile, ghi kết quả (tự lành / báo người dùng / hỏng).
- [ ] File lớn: thêm file 100 MB vào worktree — qua git thường, Git LFS, hay store content-hash (quyết định 11 cho cả 3 đường); đo thời gian commit/merge và phình dung lượng repo sau 10 ticket như vậy.
- [ ] Sửa ngoài: người dùng sửa file trong worktree bằng editor ngoài giữa run — file watcher phát hiện trong bao lâu, cơ chế revert có làm mất thay đổi hợp lệ nào không.
- [ ] Remote: test đủ luồng quyết định trong mục Tạo project (ls-remote, fetch, dry-run push ref tạm rồi xóa) với GitHub và một GitLab tự host.

| Chỉ số | Pass | Kill |
|---|---|---|
| 8 điểm tiêm lỗi | 8/8 tự lành hoặc báo người dùng kèm hướng xử lý cụ thể; **0 mất việc đã commit** | Bất kỳ điểm nào mất việc đã commit, hoặc cần người dùng tự sửa git thủ công |
| Merge queue 50 lượt chạy | 0 race, 0 merge sai thứ tự | Có race lặp lại được |
| Worktree Windows | Không lỗi đường dẫn dài ở tên project/ticket thực tế | Lỗi đường dẫn mà không có cách đặt tên thay thế |
| Phát hiện sửa ngoài | ≤ 5 giây, revert đúng file | Không phát hiện, hoặc revert cả thay đổi hợp lệ |

**Phương án B:** nếu worktree là nguồn lỗi chính → workspace theo ticket bằng *copy + diff/patch* (bỏ worktree), vẫn giữ merge queue; nếu reconcile quá phức tạp → giảm cam kết: crash giữa merge thì đưa ticket về trạng thái "Cần kiểm tra" cho người dùng bấm một nút xử lý, thay vì tự lành hoàn toàn — nhưng phải sửa quyết định 22 cho khớp.
**Evidence:** bảng kết quả 8 điểm tiêm lỗi, log reconcile từng lần, số đo file lớn.

---

## S5 — Keyring & secret trên 3 OS (1 ngày)

**Quyết định cần trả lời:** OS keyring có chứa nổi *đúng các loại secret Agenium định lưu* không, hay phải chuyển sang mô hình vault mã hóa.
**Các loại secret phải thử (lấy đúng định dạng thật):** API key LLM (~50 ký tự), token OAuth dạng JSON (1–4 KB), credential MCP, **khóa riêng SSH ed25519** dạng OpenSSH (~400 byte), và — để biết giới hạn — khóa RSA 4096 dạng PEM (~3,2 KB).

- [ ] Qua đúng crate `keyring`: ghi → đọc → so khớp từng byte → xóa, cho từng loại secret trên, trên Windows / macOS / Linux (GNOME Keyring và KWallet nếu có thể).
- [ ] Ghi lại giới hạn thực tế gặp phải trên Windows (blob size, hệ quả mã hóa UTF-16 của crate) — secret nào vượt giới hạn, lỗi trả về có rõ ràng không.
- [ ] Linux không có Secret Service chạy (máy chủ giả lập, phiên SSH, bản phân phối tối thiểu): hành vi là gì — lỗi rõ ràng hay âm thầm lưu chỗ khác/từ chối khởi động.
- [ ] macOS: cài bản app mới đè lên bản cũ (kịch bản update thật, chữ ký khác/chưa ký) → Keychain có hỏi lại quyền truy cập hàng loạt không.
- [ ] Quét toàn bộ Agenium root, DB, log, và git history của project sau một phiên làm việc mẫu: có secret nào xuất hiện ở bất cứ đâu không (dùng gitleaks + tìm kiếm thủ công theo giá trị canary).
- [ ] Secret dạng canary trong env của tiến trình sandbox: xác nhận lại kết quả ở S2 (env sạch).

| Tiêu chí | Pass | Kill |
|---|---|---|
| Round-trip ed25519 + API key + MCP credential | Khớp từng byte trên cả 3 OS | Bất kỳ loại bắt buộc nào không lưu/đọc lại đúng |
| Token OAuth 1–4 KB | Lưu được, hoặc có cơ chế chia nhỏ (chunking) hoạt động trong spike | Không lưu được và không có cơ chế thay thế |
| Linux thiếu Secret Service | Lỗi rõ ràng + hướng dẫn cụ thể | Âm thầm lưu plaintext hoặc crash không rõ nguyên nhân |
| Rò rỉ secret | 0 canary xuất hiện ngoài keyring | Bất kỳ rò rỉ nào → sửa xong phải quét lại đạt 0 mới Pass |

**Phương án B (rất có khả năng phải dùng một phần):** mô hình lai — keyring chỉ giữ *một khóa chủ*, toàn bộ secret lưu trong file vault mã hóa (XChaCha20-Poly1305 hoặc age) tại Agenium root. Mô hình này gỡ được giới hạn kích thước, Linux headless, và backup nhất quán; spike nên dựng thử tối thiểu (ghi/đọc 5 secret) để có số so sánh.

---

## S6 — Embedding & memory retrieval (1 ngày)

**Quyết định cần trả lời:** `bge-small-en-v1.5` qua fastembed có đủ nhanh/rẻ trên máy tối thiểu không; và dữ kiện cho tranh luận "tiếng Anh duy nhất" (quyết định 2b/2c) — thay vì quyết định bằng cảm tính.

- [ ] Đo với fastembed trên máy tối thiểu: thời gian tải model lần đầu (dung lượng tải, có resume được khi đứt mạng không), thời gian nạp model vào RAM (cold start), RAM chiếm, throughput embed (fact/giây, lô 1 và lô 32).
- [ ] Bộ kiểm tra 60 cặp (fact, câu truy vấn) do người soạn viết tay: 30 tiếng Anh, 30 tiếng Việt cùng nội dung tương đương. Đo recall@5 của bge-small-en trên cả hai.
- [ ] Chạy cùng bộ kiểm tra với một model đa ngữ ứng viên (vd. multilingual-e5-small hoặc bge-m3) — đo recall và cái giá (kích thước, tốc độ, RAM) so với bge-small-en.
- [ ] Mô phỏng truy xuất trước run: top-k fact cho 50 ticket mẫu, đo độ trễ toàn đường (embed query + search trong kết quả S1).

| Chỉ số | Pass | Kill / hệ quả |
|---|---|---|
| Cold start nạp model | ≤ 5 giây, RAM tăng ≤ 600 MB | > 15 giây hoặc > 1,5 GB (máy 8 GB không chịu nổi cùng lúc với app + agent) |
| Throughput embed | ≥ 20 fact/giây (lô) trên CPU tối thiểu | < 5 fact/giây (đóng ticket nào cũng phải chờ trích xuất memory) |
| Recall@5 tiếng Anh | ≥ 0,85 | < 0,70 |
| Recall@5 tiếng Việt (bge-small-en) | Ghi số, không Pass/Kill | Nếu kém tiếng Anh > 20 điểm % và model đa ngữ bù được với giá chấp nhận được → **bằng chứng để sửa quyết định 2b/2c** trước M1.5 |

**Phương án B:** model đa ngữ làm mặc định; hoặc Embedder qua API cho máy yếu (đổi lại: memory rời máy — phải được người dùng đồng ý rõ ràng, sửa quyết định "riêng tư tuyệt đối").

---

## S7 — Đường realtime: SurrealDB → Tauri → React (1 ngày)

**Quyết định cần trả lời:** Dùng live query của SurrealDB, hay `tokio::broadcast` của orchestrator, hay cả hai — thiết kế gốc đang có *hai đường song song*, spike phải chọn một đường chính có bằng chứng.

- [ ] Dựng màn Kanban tối thiểu (Tauri 2 + React): 500 ticket trên board, kéo thả bằng dnd-kit; đo FPS khi kéo và khi có 100 ticket thay đổi cùng lúc từ "phía agent" (script ghi vào DB/bus).
- [ ] Đo độ trễ đầu-cuối: một comment được ghi → hiện trên UI, theo cả hai đường (live query và broadcast → Tauri event), p50/p95, mỗi đường 500 mẫu.
- [ ] Burst: 200 event/giây trong 10 giây — UI có mất event, có giật đứng > 1 giây, có render sai thứ tự không.
- [ ] Ngắt quãng: cho máy ngủ (sleep) 1 phút rồi mở lại; tắt/bật lại app giữa burst — UI có tự đồng bộ về đúng trạng thái DB không (đây là lý do event log tồn tại; kiểm chứng nó).
- [ ] WebView khác nhau: lặp lại phép đo FPS trên Windows (WebView2) và Linux (WebKitGTK) — WebKitGTK là điểm thường yếu nhất.

| Chỉ số | Pass | Kill |
|---|---|---|
| Độ trễ comment → UI (đường được chọn) | p95 ≤ 300 ms | p95 > 1 giây |
| Burst 200 event/giây | 0 event mất ở tầng dữ liệu; UI tụt khung hình nhưng bắt kịp ≤ 3 giây | Mất event hoặc trạng thái UI lệch DB vĩnh viễn |
| Sau sleep/restart | Tự đồng bộ đúng 100% | Phải tải lại thủ công mới đúng |

**Phương án B:** nếu live query yếu → broadcast làm đường chính, live query bỏ hẳn (đơn giản hóa thiết kế); nếu cả hai đường đều yếu ở burst → gom lô (batch 100–250 ms) phía orchestrator trước khi emit.

---

## 8. Mẫu biên bản cho mỗi spike (điền trong ≤ 30 phút sau khi chạy xong)

```text
Spike: S_ — <tên>
Ngày chạy / người chạy / máy đo (CPU, RAM, OS, phiên bản):
Ngưỡng đã chốt ngày: ____ (trước khi chạy)
Kết quả: PASS / YELLOW / RED
Số liệu chính: (3–5 con số quan trọng nhất, kèm link file kết quả thô)
Tiêu chí Kill nào đã chạm (nếu có):
Điều bất ngờ học được:
Quyết định đề xuất: Đi tiếp như thiết kế / Đi tiếp kèm điều kiện ___ / Chuyển phương án B ___
Ảnh hưởng tới quyết định số ___ trong Agenium-Kien-truc.md và tới lộ trình:
Evidence: <link repo spike @ commit, log, video>
```

## 9. Cổng quyết định cuối M0-spike (ngày 10 — không có cổng này thì spike vô nghĩa)

- [ ] Mọi spike đều có biên bản; spike nào thiếu biên bản tính là Yellow.
- [ ] Lập bảng tổng: Spike × Kết quả × Quyết định × Thay đổi cần áp vào tài liệu kiến trúc.
- [ ] **Điều kiện Go cho M0 thật:** S1 không Red ở toàn vẹn dữ liệu; S2 không Red ở thoát sandbox nghiêm trọng *hoặc* đã chọn phương án B và sửa thông điệp sản phẩm tương ứng; S4 đạt 8/8 điểm tiêm lỗi không mất việc.
- [ ] **Điều kiện No-Go / quay lại thiết kế:** ≥ 2 Red, hoặc 1 Red không có phương án B khả thi trong ngân sách 2 tuần — lúc đó làm bước "chắt lọc MVP v1" và "sửa 38 quyết định" *trước* khi viết thêm code.
- [ ] Mọi thay đổi rút ra từ spike được ghi thành ADR và cập nhật ngược vào Agenium-Kien-truc.md trong cùng tuần — spike xong mà tài liệu không đổi là spike phí.

---

## Phụ lục — Những gì spike này cố tình *không* kiểm chứng

- **Chất lượng hành vi agent** (plan tốt không, review kỹ không): thuộc về bộ golden-task/eval của M1, không phải M0. Đừng để spike M0 phình thành thử nghiệm LLM.
- **CLI backend** (quyết định 3c/30): rủi ro thật nhưng ở M2.5; kiểm chứng bây giờ là sớm và tốn. Chỉ cần một ghi chú: khi tới M2.5, lặp lại đúng mẫu spike này cho từng CLI.
- **Prompt caching theo vendor:** tối ưu sau khi có số đo cache-hit thật từ M1; không spike trước.
