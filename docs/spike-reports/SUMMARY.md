# Tổng hợp M0-spike Agenium — kết quả trên Linux (2026-10-23, giờ Asia/Saigon)

> **Phạm vi trung thực:** toàn bộ số đo chạy trên **một box Linux** (Debian 13, kernel 6.12.94, 8 vCPU, 15 GB RAM, không GPU) — *không phải máy tối thiểu*. **Windows và macOS chưa chạy** (đánh dấu NOT RUN, không có số giả). Ngưỡng Pass/Kill lấy từ `THRESHOLDS-M0-SPIKE-CHECKLIST.md` (commit ac2bb61, 2026-10-23 08:59 +07, trước khi đo) và **không bị nới**.

## Bảng kết quả

| Spike | Kết quả | Số liệu chính | Engine / phương án chọn · Plan B | Ghi chú |
|---|---|---|---|---|
| **S0** Build baseline | **PASS (Linux)** · Win/macOS NOT RUN | Build sạch: Tauri rỗng 63 s; +SurrealKV 320 s (5,3 ph); +RocksDB 515 s (8,6 ph) (ngưỡng ≤ 45 ph). Binary strip: 148 / 157 MB (rỗng 7 MB); riêng SurrealDB +114 / +124 MB (ngưỡng ≤ 150 MB **chỉ khi `strip=true`**; không strip 151–162 MB). Build tăng dần ~124 s. RSS rustc 5,2 GB | SurrealKV nhanh build hơn ~38% | Cần ghi `strip=true` + danh sách apt. Máy build 8 GB RAM chưa kiểm |
| **S1** SurrealDB embedded (cổng) | **PASS cả 2 engine (Linux)** · Win/macOS NOT RUN | Đúng 890/890 truy vấn so oracle; truy vấn điều phối p95 ~12 ms (≤ 100); KNN top-10 p95 5,8 / 5,0 ms (≤ 50); recall@10 HNSW 0,997 (fixture) và 0,966 (embedding thật, ef=40); live query 0 mất/0 trùng; **crash SIGKILL 20/20 sạch cả hai** (0 lỗi toàn vẹn); mở DB 0,8 / 0,6 s; export→import→export 0 lệch | **Chọn SurrealKV để bắt đầu**, RocksDB là dự phòng (cùng tính đúng; import nhanh hơn, nhưng nhiều transaction conflict hơn ở ghi nóng 238 vs 155 và nặng build). Plan B (SQLite+sqlite-vec+FTS5+broadcast) đã có prototype: KNN/BM25 đủ nhanh, ~12 hạng mục truy vấn/cơ chế phải viết lại | **Schema thiết kế có lỗi cú pháp SurrealQL 3.x** → `S1/schema-patch.diff`. Cần retry khi `Transaction conflict`; live query không phát lại event lỡ; cần index `event.ticket`. Crash chỉ SIGKILL, không mất điện |
| **S2** Sandbox Windows | **NOT RUN** (cần Windows 11) | — | Plan B: chưa đánh giá được | Làm thiếu điều kiện Go "S2 không Red hoặc có plan B" |
| **S3** Sandbox Linux (macOS: NOT RUN) | **PASS có điều kiện (Linux)** · macOS NOT RUN | 36 test tấn công: `sbx` chặn 34, lọt 2 (chmod/utime metadata ngoài; bwrap chặn cả 2); 8/8 việc hợp lệ; overhead sbx +4–8%, bwrap −4…+2% (ngưỡng ≤ 30%); fail-closed OK (rc 126); kill cây tiến trình: SIGTERM 0 sống sót; **SIGKILL supervisor: sbx 5 tiến trình mồ côi/lần**, bwrap 0 | Landlock+seccomp (`sbx`) làm backend Linux, **bwrap ưu tiên khi có**. **Yêu cầu Landlock ABI ≥ 3 (kernel ≥ 6.2)**: ở ABI1/2 (giả lập) `truncate` file ngoài không bị chặn | Mức kernel tối thiểu 5.13 trong tài liệu cần nâng hoặc dùng bwrap. Cần dọn tiến trình mồ côi lúc khởi động. ABI thấp chỉ **giả lập** |
| **S4** Git worktree / merge queue (cổng) | **PASS (Linux, mô hình)** · Windows long path NOT RUN | **8/8 điểm tiêm lỗi, 24/24 lần tự lành, 0 mất commit**; merge queue 50 lượt 0 race; sửa ngoài phát hiện 1 ms; 100 MB × 10: git thường +1049 MB, LFS +1049 MB (commit nhanh hơn 2,6×), hash-store 0 MB trong git | Giữ worktree + merge queue + event-trước-state (không cần plan B copy+diff trên Linux) | SQLite thay SurrealDB; chỉ SIGKILL. Trùng file không khai `depends_on` chỉ lộ ở merge (có thể thêm cảnh báo `--name-only`). Branch cần `-D`. GitHub ghi thật & GitLab tự host: NOT RUN |
| **S5** Keyring & secret | **PASS (Linux/GNOME Keyring)** · Win/macOS/KWallet NOT RUN | 7/7 secret round-trip khớp byte (kể cả OAuth 4 KB, ed25519 411 B, RSA-4096 3272 B); không có Secret Service → lỗi rõ, không lưu chỗ khác; 0 canary rò | Mặc định **vault lai** (XChaCha20-Poly1305, khóa chủ trong keyring; 0,5 ms) — keyring chỉ giữ khóa chủ | **Keyring bị khóa + headless → treo** (cần timeout riêng). keyring 4.x cần default store; lỗi thật nằm ở `store_status()`. Giới hạn blob Windows chưa đo |
| **S6** Embedding / memory | **YELLOW** | bge-small-en: cold start 0,52 s, +196 MB RAM, 437 fact/s, recall@5 EN 0,867 (≥ 0,85) → đạt ngưỡng cứng; **VI 0,400 (kém EN 46,7 điểm)** → kích hoạt rà soát quyết định 2b/2c. e5-small: VI 0,833, 972 MB RAM, cold 2,6 s (Yellow); bge-m3: 1680 MB → chạm Kill | **Đề xuất e5-small (đa ngữ) làm ứng viên mặc định** cho người dùng tiếng Việt; bge-small-en nếu chỉ EN/RAM thấp | Bộ test 60 cặp tự soạn (n=30/ngôn ngữ, ±0,12). Chưa đo máy tối thiểu |
| **S7** Đường realtime | **PASS tầng dữ liệu** · UI/FPS/WebView NOT RUN | Độ trễ comment→subscriber p95 2,6 ms (≤ 300); burst 200 ev/s: 0 mất/0 trùng/0 sai thứ tự; sau ngắt/khởi động lại UI==DB 100% nhờ đối chiếu event log | **`tokio::broadcast` làm đường chính** + event log `seq` để reconcile; live query phụ | Không có Tauri/webview/GPU: Kanban, dnd-kit, FPS, sleep máy thật chưa đo |

## Cổng quyết định (checklist mục 9)

| Điều kiện Go | Đánh giá |
|---|---|
| S1 không Red về toàn vẹn | **Đạt** (20/20 crash sạch cả hai engine; chỉ SIGKILL) |
| S2 không Red, hoặc có plan B | **KHÔNG ĐÁNH GIÁ ĐƯỢC** (NOT RUN) |
| S4 8/8 | **Đạt** (mô hình trên Linux; SQLite thay SurrealDB) |
| No-Go: ≥ 2 Red hoặc 1 Red không plan B | Không spike nào Red trên Linux |

## Khuyến nghị Go / No-Go
**GO CÓ ĐIỀU KIỆN — chỉ dựa trên bằng chứng Linux; chưa phải Go đầy đủ.** Chưa thể ký Go cuối vì S2 (Windows) chưa chạy và các phần Windows/macOS của S0/S3/S4/S5 cũng chưa chạy. Kiến trúc lõi (SurrealDB embedded + worktree/merge queue + vault lai + broadcast+log) không có tín hiệu Kill trên Linux.

Điều kiện để chuyển sang Go đầy đủ:
1. Chạy S2 + các phần Windows/macOS (đã chuẩn bị `.github/workflows/spike.yml` cho S0/S5 và smoke test Seatbelt cho S3 trên runner CI; S2/S3-Windows cần máy Windows 11 thật). Kết quả CI chỉ mang tính chỉ báo, runner không phải máy tối thiểu.
2. Áp `S1/schema-patch.diff` vào SCHEMAS gốc; thêm retry cho `Transaction conflict`, index `event.ticket`, `strip=true`.
3. Realtime: broadcast chính + reconcile theo `seq`; không dựa vào live query.
4. Keyring: timeout bao quanh mọi lời gọi + hướng dẫn khi khóa; dùng vault lai mặc định.
5. Sandbox Linux: fail-closed, yêu cầu ABI ≥ 3 hoặc bwrap; dọn tiến trình mồ côi lúc khởi động; ghi rủi ro chmod/utime.
6. Embedding: quyết định lại 2b/2c (e5-small) sau khi đo trên máy tối thiểu.
7. Lặp lại crash test với mất điện (không chỉ SIGKILL) và S4 trên SurrealDB thật.

## Danh sách NOT RUN (không có số liệu)
- **S2 toàn bộ** (sandbox Windows); phần **Windows/macOS** của S0, S3, S4 (đường dẫn dài 260 ký tự chỉ có ước lượng số học: 149–245), S5 (Credential Manager, Keychain, KWallet).
- **UI của S7**: Kanban/dnd-kit/FPS, WebView2/WebKitGTK, sleep máy thật.
- **GitHub ghi thật, GitLab tự host** (S4 remote mới thử trên bare repo cục bộ + `ls-remote` GitHub công khai).
- Mất điện/ENOSPC, máy tối thiểu (RAM 8 GB/CPU yếu), kernel 5.13/5.15 thật (ABI giả lập), gitleaks, quét phiên Agenium thật.

## Vị trí bằng chứng
`spikes/S0..S7/REPORT.md` (tiếng Việt, mẫu ADR) và `results/` của từng spike; `spikes/S1/schema-patch.diff`; `.github/workflows/spike.yml` (chưa chạy).
