# Biên bản Spike S6 — Embedding & memory retrieval (Linux, CPU)

```text
Spike: S6 — Embedding & memory retrieval
Ngày chạy: 2026-10-23 (Asia/Saigon) / Grok Bot / máy: box Linux Debian 13, Intel Xeon 8 vCPU, 15 GB RAM, không GPU; fastembed 7.x (ONNX Runtime CPU), SurrealDB 3.3.4 SurrealKV cho phần truy xuất end-to-end
Ngưỡng đã chốt ngày: 2026-10-23 08:59 +07 (commit ac2bb61) — trước khi chạy
Kết quả: bge-small-en (mặc định thiết kế): PASS theo từng ngưỡng cứng  |  Tổng thể: YELLOW — khoảng cách tiếng Việt chạm điều kiện "sửa quyết định 2b/2c"
```

## Số liệu chính (thô: `S6/results/{download,bench,e2e}_*.txt`; bộ kiểm tra: `S6/data/pairs.json`, sinh bởi `make_set.py`; mã: `S6/s6/src/main.rs`)

Bộ 60 cặp tự soạn: 30 cặp EN + 30 cặp VI tương đương (truy vấn viết lại, ít trùng từ). Corpus mỗi ngôn ngữ = 30 fact đích + 30 fact gây nhiễu cùng miền (60 fact); recall@5 = fact đích nằm trong top-5 trên 60.

| Chỉ số | bge-small-en-v1.5 (384-d) | multilingual-e5-small (384-d) | bge-m3 (1024-d) |
|---|---|---|---|
| Tải lần đầu (dung lượng / thời gian trên mạng datacenter) | 134 MB / 2,2 s | 487 MB / 4,9 s | 2285 MB / 12,1 s |
| Cold start (đã drop page cache) | **0,52 s** | 2,60 s | 4,08 s |
| RAM tăng khi nạp model | **196 MB** (252 MB sau bench) | 972 MB (1020 MB) | 1680 MB (1813 MB) |
| Throughput lô 32 / lô 1 (fact/s) | **437 / 224** (506 / 238 không instruction) | 390 / 177 | 38,5 / 18,4 |
| Embed 1 truy vấn p50 / p95 | 4,7 / 5,7 ms | 5,6 / 6,6 ms | 56,8 / 66,2 ms |
| **Recall@5 EN** | **0,867** (recall@1 0,70) | 0,967 | 1,000 |
| **Recall@5 VI** | **0,400** (recall@1 0,20) | 0,833 | 0,967 |
| Truy vấn VI → fact EN (recall@5) | 0,167 | 0,733 | 0,900 |
| Truy vấn EN → fact VI (recall@5) | 0,500 | 0,933 | 1,000 |
| Đường đầy đủ 50 ticket (embed query + HNSW top-5 trong 2000 fact, SurrealKV): p50 / p95 | **5,7 / 7,4 ms** | 5,3 / 6,4 ms | 34,2 / 44,7 ms |
| **HNSW recall@10 so với brute-force trên embedding THẬT** (2000 fact, 100 truy vấn), ef=40 / ef=100 | 0,966 / 0,992 | 0,966 / 0,979 | — |
| Embed 2000 fact (nạp corpus) | 3,7 s (544/s) | 4,2 s (477/s) | 35,7 s (56/s) |

bge-small-en dùng tiền tố instruction của BGE cho truy vấn (khuyến nghị); biến thể không instruction: EN recall@5 vẫn 0,867, VI 0,400.

## So với ngưỡng
| Chỉ số | Ngưỡng | bge-small-en | Kết luận |
|---|---|---|---|
| Cold start | Pass ≤ 5 s & RAM ≤ 600 MB; Kill > 15 s hoặc > 1,5 GB | 0,52 s, 196 MB | PASS |
| Throughput | Pass ≥ 20 fact/s; Kill < 5 | 437 (lô), 224 (lô 1) | PASS (máy 8 vCPU; chưa đo máy tối thiểu) |
| Recall@5 EN | Pass ≥ 0,85; Kill < 0,70 | 0,867 | PASS — sát ngưỡng; n=30 nên khoảng tin cậy ±~0,12 |
| Recall@5 VI | chỉ ghi số; nếu kém EN > 20 điểm % và model đa ngữ bù được → sửa quyết định 2b/2c | 0,400 (kém EN 46,7 điểm %) | **ĐIỀU KIỆN KÍCH HOẠT** |
| Ứng viên đa ngữ | cùng bảng ngưỡng | e5-small: VI 0,833 (kém EN 13,4 điểm), cold 2,6 s, **972 MB (> 600 MB Pass, < 1,5 GB Kill → Yellow)**; bge-m3: **1680 MB > 1,5 GB → chạm Kill**, 38 fact/s | e5-small là ứng viên bù được với giá chấp nhận được; bge-m3 quá nặng cho máy 8 GB |

Tiêu chí Kill đã chạm: chỉ ở ứng viên bge-m3 (RAM > 1,5 GB); mặc định bge-small-en không chạm.

## Điều bất ngờ học được
- bge-small-en gần như **mù tiếng Việt** (recall@5 0,40; tìm VI→EN chỉ 0,17): "memory tiếng Anh duy nhất" nghĩa là fact/query tiếng Việt gần như không truy xuất được.
- multilingual-e5-small tốt hơn cả bge-small-en trên tiếng Anh trong bộ này (0,967 vs 0,867) với tốc độ gần tương đương (390 vs 437 fact/s) — cái giá chính là RAM (+776 MB) và đĩa (+353 MB), cold start 2,6 s.
- Độ trễ truy xuất toàn đường chỉ ~6–7 ms (embed 4–5 ms + HNSW ~1 ms): không phải nút cổ chai.
- Recall "e2e trong 2000 fact" (0,20/0,26/0,40) **không dùng làm chỉ số chất lượng**: 1940 fact gây nhiễu tổng hợp giống nhau theo mẫu và gần ngữ nghĩa với truy vấn (thứ tự top-5 bị fact mẫu chiếm; đã kiểm bằng log debug) — đó là hạn chế của fixture, không phải lỗi HNSW (S1 đo recall HNSW riêng: 0,997 trên fixture phân cụm).

## Hạn chế (trung thực)
- Bộ 30+30 cặp do Grok Bot soạn (không phải người dùng thật), nhiễu cùng miền chỉ 30 fact; kết quả là chỉ báo định hướng, cần người thật bổ sung trước khi chốt.
- Chưa đo máy tối thiểu (4 core/8 GB); mạng tải nhanh bất thường; **chưa thử resume tải khi đứt mạng** (hf-hub); chưa đo RAM đỉnh khi đồng thời với app + agent.
- Không thử multilingual model nào khác ngoài e5-small/bge-m3 (ví dụ model Việt chuyên biệt).

## Quyết định đề xuất
Đi tiếp kèm điều kiện: **đưa multilingual-e5-small thành ứng viên mặc định thay bge-small-en** (hoặc cho người dùng chọn "English-only nhẹ" vs "Đa ngữ") nếu sản phẩm nhắm người dùng Việt; nếu giữ English-only thì ghi rõ giới hạn trong UI. Không dùng bge-m3 mặc định (RAM > 1,5 GB, 38 fact/s).
Ảnh hưởng tới quyết định số 2b/2c trong Agenium-Kien-truc.md: có số đo để xem lại trước M1.5 (khoảng cách VI–EN 46,7 điểm % với bge-small-en vs 13,4 với e5-small). Nếu đổi sang e5-small, kích thước vector vẫn 384 → schema HNSW không đổi, nhưng phải re-embed khi đổi model.
Evidence: `spikes/S6/`.
