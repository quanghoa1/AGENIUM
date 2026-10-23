# Biên bản Spike S2 — Sandbox Windows

```text
Spike: S2 — Sandbox Windows (cổng quyết định, rủi ro cao nhất)
Kết quả: NOT RUN - needs Windows
Lý do: box chỉ có Linux; không giả lập kết quả Windows. Không có số liệu nào được ghi cho S2.
```

- Chưa chạy: audit trước khi code, ma trận xâm nhập (AppContainer/Job Object/restricted token), ma trận việc hợp lệ, độ ổn định (kill cây tiến trình), overhead.
- Việc phải làm trên máy Windows 11 thật: theo mục S2 của `THRESHOLDS-M0-SPIKE-CHECKLIST.md`. Có thể dùng bộ test của S3 làm khuôn (`spikes/S3/tests/payload.py` — các test chạy được trên mọi OS ở mức Python; phần syscall Linux phải viết lại cho Windows).
- Gate: theo mục 9, "S2 không Red **hoặc có phương án B**" — S2 chưa chạy nên điều kiện này **chưa được thỏa**; phương án B (WSL2 → Docker/VM → experimental) đã ghi trong thiết kế nhưng chưa được kiểm chứng. → Go/No-Go không thể là Go đầy đủ khi chưa có S2.
- Ước lượng duy nhất đã làm (chỉ số học, S4): đường dẫn worktree dài nhất ~149–245 ký tự cho 6 kịch bản tên project/ticket (giới hạn 260) — xem S4.
