# Biên bản Spike S7 — Đường realtime: SurrealDB → Tauri → React (PHẦN DỮ LIỆU ONLY; UI NOT RUN)

```text
Spike: S7 — Đường realtime
Ngày chạy: 2026-10-23 (Asia/Saigon) / Grok Bot / máy: box Linux 8 vCPU, SurrealDB 3.3.4 SurrealKV, tokio 1.x; chạy trong MỘT process (không có Tauri/webview)
Ngưỡng đã chốt ngày: 2026-10-23 08:59 +07 (commit ac2bb61) — trước khi chạy
Kết quả: PASS cho tầng dữ liệu (cả hai đường)  |  UI Kanban/dnd-kit/FPS, WebView2, WebKitGTK, sleep máy: NOT RUN (chỉ giả lập sleep bằng drop subscriber/restart DB)
```

## Số liệu chính (thô: `S7/results/s7.txt`; mã: `S7/s7/src/main.rs`)
Độ trễ = từ ngay trước khi ghi comment vào DB đến lúc subscriber nhận (đường broadcast: emit sau khi commit xong; đường live: LIVE SELECT trên bảng `event`).

| Phép đo | Live query | `tokio::broadcast` |
|---|---|---|
| Độ trễ 500 mẫu (20 ev/s) p50 / p95 / max | 1,82 / 2,60 / 26,9 ms | 1,80 / 2,56 / 56,9 ms |
| Burst 200 ev/s × 10 s (2000 event): mất / trùng / sai thứ tự | 0 / 0 / 0 | 0 / 0 / 0 |
| Burst: p95 trễ, khoảng lặng lớn nhất giữa 2 event, bắt kịp sau lần ghi cuối | 2,28 ms, 51,5 ms, 0,0 ms | 2,13 ms, 62,3 ms, 0,1 ms |
| 100 thay đổi đồng thời (100 ghi song song) | 100/100 nhận trong 14 ms | — |
| Ngắt subscriber + ghi 150 event vắng mặt → đối chiếu event log (`seq > last_seen`) | live KHÔNG phát lại (đã biết từ S1: 0/100); sau đối chiếu log: **UI == DB 100%** (khôi phục 150 event từ log) | không cần phát lại — cùng cơ chế đối chiếu log |
| Tắt/bật lại (đóng DB, mở lại) | mở + đếm 7 ms; 5200/5200 event còn nguyên, trạng thái == DB | giống |

## So với ngưỡng
| Chỉ số | Ngưỡng | Đo được | Kết luận |
|---|---|---|---|
| Độ trễ comment → "UI" (tầng dữ liệu) | p95 ≤ 300 ms (Kill > 1 s) | p95 2,6 ms (cả hai đường) | PASS — chưa gồm IPC Tauri + render React |
| Burst 200 ev/s | 0 mất ở tầng dữ liệu; UI bắt kịp ≤ 3 s | 0 mất, bắt kịp ~0 ms ở tầng dữ liệu | PASS (tầng dữ liệu); UI/FPS NOT RUN |
| Sau sleep/restart | tự đồng bộ 100% | 100% nhờ đối chiếu event log | PASS (mô phỏng) |

Tiêu chí Kill đã chạm: không.

## Điều bất ngờ / kết luận thiết kế
- Hai đường **tương đương về độ trễ**; khác biệt nằm ở ngữ nghĩa: live query **không phát lại** event bị lỡ khi ngắt/nối lại (đo ở S1 và S7) và ghi nóng cùng bản ghi gây `Transaction conflict` cần retry (S1: 155–238 lần/1000 ghi song song 4 writer) → dù chọn đường nào vẫn **bắt buộc có đường đối chiếu từ event log theo `seq`**.
- Đề xuất chọn **`tokio::broadcast` (emit sau commit) làm đường chính** + event log `seq` làm nguồn đối chiếu khi (re)connect; bỏ live query khỏi đường chính (đơn giản hơn, không phụ thuộc hành vi live query của SurrealDB, ghép được batch 100–250 ms nếu cần). Live query chỉ giữ cho tiến trình ngoài (CLI/MCP) nếu sau này cần.
- Lưu ý `broadcast` có thể `Lagged` nếu kênh đầy — phải xử lý bằng đối chiếu log (đã dựng đường đó).

## Hạn chế (trung thực)
- **Không có UI**: không đo FPS kéo thả 500 ticket (dnd-kit), không đo WebKitGTK/WebView2, không đo IPC Tauri→React. Box không có GPU (WebKitGTK dưới Xvfb không đại diện) → cần chạy tay trên máy thật; phần này giữ trạng thái NOT RUN.
- "Sleep máy 1 phút" chưa thực hiện thật; chỉ mô phỏng ngắt subscriber + đóng/mở DB.
- 1 writer có nhịp đều, 1 subscriber; chưa đo nhiều subscriber hoặc payload lớn.

## Quyết định đề xuất
Đi tiếp: broadcast + event log `seq` là đường chính (tầng dữ liệu đã chứng minh). Phần UI cần chạy trên máy thật trước khi chốt Go cuối.
Ảnh hưởng quyết định (đường realtime song song trong thiết kế): đơn giản hóa còn một đường chính.
Evidence: `spikes/S7/`.
