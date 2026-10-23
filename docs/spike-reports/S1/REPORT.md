# Biên bản Spike S1 — SurrealDB embedded: SurrealKV vs RocksDB (Linux)

```text
Spike: S1 — SurrealDB embedded là kho sự thật duy nhất? (CỔNG QUYẾT ĐỊNH)
Ngày chạy: 2026-10-23 (Asia/Saigon) / Grok Bot / máy: box Linux Debian 13, kernel 6.12.94, Xeon 8 vCPU, 15 GB RAM, disk ảo; SurrealDB 3.3.4 embedded (kv-surrealkv và kv-rocksdb), release build
Ngưỡng đã chốt ngày: 2026-10-23 08:59 +07 (commit ac2bb61) — trước khi chạy
Kết quả: PASS cho CẢ HAI engine trên Linux (có điều kiện — xem "Hạn chế")  |  Windows/macOS: NOT RUN
```

Fixture (`S1/s1/src/{fixture,oracle,main}.rs`): schema thiết kế thật (đã vá cú pháp, xem dưới), 200 ticket + comment/event/run/… (66 420 dòng), 10 000 fact vector 384-d (HNSW COSINE) + BM25 FULLTEXT, oracle SQLite độc lập để kiểm đúng/sai, 890 phép so sánh truy vấn. Chạy bằng `S1/run_s1.sh <engine>`; thô: `S1/results/{surrealkv,rocksdb}/{metrics.csv,log.txt,crash_runs.csv}`.

## Số liệu chính
| Chỉ số | Ngưỡng Pass | SurrealKV | RocksDB |
|---|---|---|---|
| Đúng so với oracle (890 truy vấn Q1–Q7) | — | 0 sai khác, 0 lỗi | 0 sai khác, 0 lỗi |
| Truy vấn điều phối p95 (Q1–Q7, lớn nhất = Q7 ticket+comment) | ≤ 100 ms | **11,96 ms** | **11,78 ms** |
| KNN top-10 p95 (10k vector, ef=40) | ≤ 50 ms | **5,8 ms** (6,6 ms sau +1000 fact) | **5,0 ms** (5,6 ms) |
| KNN p95 ef=100 | — | 3,6 ms | 2,2 ms |
| Recall@10 HNSW (fixture phân cụm, ef=40 / 100) | ≥ 0,90 | **0,997 / 1,000** | **0,9975 / 1,000** |
| Recall@10 HNSW trên embedding THẬT (S6: 2000 fact bge-small/e5, ef=40 / 100) | ≥ 0,90 | 0,966 / 0,992 (bge) · 0,966 / 0,979 (e5) | (đo trên SurrealKV) |
| Recall@10 vector NGẪU NHIÊN đều (xấu nhất, không thực tế) | — | 0,60 / 0,82 | 0,64 / 0,83 |
| BM25: đích trong top-10 / trùng top-10 với SQLite FTS5 / đếm khớp | — | 50/50 · 1,000 · 50/50 | 50/50 · 1,000 · 50/50 |
| Hybrid (BM25+KNN, RRF) p95 | — | 4,2 ms (đích 50/50) | 2,6 ms (50/50) |
| Live query 1000 event: mất / trùng / p95 | 0 mất, p95 ≤ 200 ms | 0 / 0 / 4,5 ms (tuần tự); 0 / 0 / 13,8 ms (4 writer) | 0 / 0 / 1,5 ms; 0 / 0 / 7,0 ms |
| Crash test SIGKILL × 20 (ghi event + fact song song; sau kill: mở lại, kiểm) | 0 lỗi toàn vẹn | **20/20 sạch** (32 854 event, 10 710 fact đã ack; 0 mất, 0 lỗ hổng seq, BM25/HNSW tìm lại đủ, counter khớp) | **20/20 sạch** (63 031 event, 17 483 fact đã ack) |
| Mở DB (mở → truy vấn đầu) | ≤ 3 s | 0,82 s (mở lại có dữ liệu: 71 ms) | 0,57 s (208–371 ms) |
| Export → xóa → import → export lại | khớp 100% | 8 bảng, 66 420 dòng, **0 lệch**; export 2,1 s, import 18,0 s | 8 bảng, 66 420 dòng, **0 lệch**; export 1,7 s, import 11,9 s |
| Process thứ hai mở cùng DB | lỗi rõ ràng | `LOCK ... already locked by another process` | `IO error: While lock file ... Resource temporarily unavailable` |
| Dung lượng thư mục DB sau nạp (10k vector HNSW + 66k dòng) | — | 213 MB | 248 MB |

Giá trị tham chiếu plan B: SQLite + sqlite-vec v0.1.9 + FTS5 cùng 10k×384-d: nạp 0,19 s, KNN chính xác (brute force) p50/p95 = 5,1/6,0 ms, recall 1,0, DB 16,7 MB (`S1/results/planb_sqlite_vec.json`, `S1/planb/sqlite_vec_bench.py`).

## So với ngưỡng / Kill
Tất cả ngưỡng cứng đạt trên cả hai engine; **không chạm tiêu chí Kill nào** (đặc biệt: 0 lỗi toàn vẹn crash).

## Điều bất ngờ học được (quan trọng)
1. **Schema giao kèm KHÔNG chạy được trên SurrealDB 3.3.4 nguyên bản**: cú pháp `FLEXIBLE TYPE object` bị từ chối (phải `TYPE object FLEXIBLE`), `TYPE array` rỗng phải thành `array<object> FLEXIBLE`, và các field object/array lồng cần `FLEXIBLE` (project.git, agent.model, agent.mcp, ticket.acceptance, skill_overlay.changelog). Bản vá: `S1/schema-patch.diff` (bản sửa dùng trong spike: `ref/SCHEMAS/`; file gốc trong `agenium-docs/final/SCHEMAS` KHÔNG bị sửa). → cần cập nhật SCHEMAS trước M1.
2. **Live query không phát lại event bị lỡ** khi subscriber ngắt/nối lại (0/100) → event log + đối chiếu theo `seq` là bắt buộc (xem S7).
3. **Ghi nóng cùng bản ghi** từ nhiều writer → `Transaction conflict ... can be retried` (155 lần SurrealKV / 238 lần RocksDB trên 1000 ghi 4 writer) → lớp truy cập DB phải có retry.
4. Export JSON phẳng làm mất kiểu record-link → export/import phải dùng literal SurrealQL (đã làm trong harness); `ticket.updated_at` là `VALUE time::now()` nên đổi mỗi lần import (loại khỏi hash so sánh); `event.ticket` chưa có index → Q5b là quét toàn bảng (~18–23 ms ở 200 ticket; sẽ tăng theo dữ liệu → nên thêm index).
5. HNSW trên vector ngẫu nhiên đều cho recall thấp (0,60) — là artifact của dữ liệu 384-d không có cấu trúc; embedding thật (S6) cho 0,966–0,992. Dùng `ef` 100 nếu cần thêm độ chắc.
6. Dung lượng đĩa HNSW lớn: 213–248 MB cho 10k vector, so với 16,7 MB của sqlite-vec — cần tính cho ngân sách đĩa agent memory.

## Chọn engine
Cả hai đều Pass. SurrealKV: build nhanh hơn 1,6× (S0), không cần clang/cmake, binary nhỏ hơn ~10 MB, mở nhanh hơn, thư mục nhỏ hơn sau nạp; RocksDB: KNN/hybrid nhanh hơn ~30–40%, import nhanh hơn, nhưng nhiều conflict hơn trong test ghi nóng (238 vs 155) và thêm phụ thuộc C++. Đề xuất: **bắt đầu với SurrealKV** (rẻ build/CI, đủ hiệu năng) và giữ RocksDB làm phương án thay thế bằng feature flag (cùng API, spike chứng minh chạy y hệt). Chưa đánh giá độ chín/hỗ trợ dài hạn của từng engine — ngoài phạm vi spike.

## Phương án B (SQLite) — ước lượng công sức
Cần viết lại: 8 họ truy vấn quan hệ (Q1–Q7 + Q5b), 3 truy vấn truy xuất (KNN, BM25, hybrid), 1 cơ chế live → `tokio::broadcast` (đã khuyến nghị ở S7), 1 export/import, và lớp schema = **khoảng 12 mục truy vấn/cơ chế cần viết lại** + bộ test oracle đã có sẵn dùng lại được. Hạ tầng đã khả thi (sqlite-vec KNN p95 6 ms ở 10k; brute-force nên tuyến tính theo số vector — ở 100k vector cần đo lại).

## Hạn chế (trung thực)
- Crash test chỉ **SIGKILL tiến trình**, không phải mất điện/mất cache hệ điều hành; chưa thử các chế độ `sync` khác; kết luận "0 lỗi toàn vẹn" chỉ áp dụng cho kịch bản này.
- Máy đo 8 vCPU/15 GB, disk ảo — không phải máy tối thiểu; dữ liệu 66k dòng + 10k vector, nhỏ hơn dự án lâu năm.
- Dữ liệu fixture là tổng hợp (vector phân cụm); recall embedding thật chỉ đo trên 2000 fact (S6).
- Bước "live reconnect" của lần chạy RocksDB bị gián đoạn bởi một `Transaction conflict` không retry trong harness (không ảnh hưởng kết luận: hành vi không-phát-lại đã đo trên SurrealKV).
- Windows/macOS: NOT RUN (khác biệt hệ tệp/fsync/antivirus chưa biết).

## Quyết định đề xuất
Đi tiếp như thiết kế **kèm điều kiện**: (1) sửa SCHEMAS theo `schema-patch.diff`; (2) retry giao dịch; (3) event log + đối chiếu `seq` là đường chính cho UI; (4) thêm index `event.ticket`; (5) lặp lại crash test trên Windows/macOS và với mất điện giả lập ở M1.
Ảnh hưởng tới quyết định số 1 (SurrealDB) trong Agenium-Kien-truc.md: giữ; phương án B chỉ cần khi Windows/macOS thất bại.
Evidence: `spikes/S1/`.
