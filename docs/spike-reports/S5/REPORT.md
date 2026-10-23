# Biên bản Spike S5 — Keyring & secret (chỉ Linux)

```text
Spike: S5 — Keyring & secret trên 3 OS
Ngày chạy: 2026-10-23 (Asia/Saigon) / Grok Bot / máy: box Linux Debian 13, kernel 6.12.94, 8 vCPU; keyring crate 4.2.0 (zbus-secret-service-keyring-store 1.0.1), gnome-keyring-daemon; không có GNOME session đồ họa
Ngưỡng đã chốt ngày: 2026-10-23 08:59 +07 (commit ac2bb61) — trước khi chạy
Kết quả: PASS (Linux/GNOME Keyring, có điều kiện về UX khi keyring bị khóa)  |  Windows, macOS, KWallet: NOT RUN
```

## Số liệu chính (thô: `S5/results/*.txt`, mã: `S5/s5/src/main.rs`)
| Kịch bản | Kết quả |
|---|---|
| C. D-Bus + gnome-keyring mở khóa | **7/7 round-trip khớp từng byte** (ghi→đọc→so→xóa→xác nhận xóa): API key 49 B, OAuth JSON 1044/2052/4116 B (cả `set_secret` byte lẫn `set_password` UTF-8), MCP credential JSON 87 B, **khóa riêng ed25519 OpenSSH 411 B**, **RSA-4096 PEM 3272 B** |
| A. Không có D-Bus session | Lỗi rõ ràng, không lưu đâu khác: `Entry::store_status()` = `PlatformFailure(Zbus(Connection(NotFound /run/user/1000/bus)))`; 7/7 `Entry::new` trả lỗi `No default store has been set` |
| B. Có D-Bus, không có Secret Service | Lỗi rõ ràng: `ServiceUnknown: org.freedesktop.secrets was not provided by any .service files`; 0 secret bị lưu ở nơi khác |
| D. gnome-keyring chạy nhưng **bị khóa**, không có UI prompt (headless) | `store_status` = Ok nhưng thao tác ghi **treo** đến khi timeout 20 s (kẹt chờ prompt) → ứng dụng PHẢI đặt timeout riêng và hiển thị hướng dẫn |
| Vault lai (plan B, XChaCha20-Poly1305, khóa chủ 32 B trong keyring) | 7 secret ghi 0,48 ms, đọc+giải mã 0,05 ms, 7/7 đúng; file vault không chứa canary dạng rõ |
| Quét canary (`CANARY`, `ghp_CANARY`, `sk-ant-CANARY`, dòng đầu khóa ed25519) trong vault + data dir + results | **0 hit** ngoài file khóa nguồn (`E_scan.txt`) |
| Canary trong env tiến trình sandbox | Xác nhận ở S3: test `env_has_canary` = blocked (env allowlist), không có canary trong `/proc/<parent>/environ` đọc được |

## So với ngưỡng
| Tiêu chí | Kết luận |
|---|---|
| Round-trip ed25519 + API key + MCP | PASS (Linux) |
| OAuth 1–4 KB | PASS — Secret Service không giới hạn 4 KB; (giới hạn blob Windows: NOT RUN, chưa biết) |
| Linux thiếu Secret Service | PASS: lỗi rõ ràng, không lưu plaintext, không crash. Ghi chú: thông điệp `No default store has been set` (từ `Entry::new`) gây hiểu nhầm — nguyên nhân thật chỉ có trong `Entry::store_status()` → Agenium phải gọi `store_status()` lúc khởi động và dịch lỗi ra hướng dẫn |
| Rò rỉ secret | 0 canary — nhưng phạm vi quét chỉ là các artefact của spike (không có phiên làm việc Agenium thật) |

Tiêu chí Kill đã chạm: không.

## Điều bất ngờ học được
- keyring 4.x **không tự có default store**: phải khởi tạo store trước (ở đây `store_status()` làm việc đó); nếu quên thì mọi `Entry::new` lỗi.
- Keyring bị khóa + không GUI → treo, không lỗi. Đây là kịch bản thật với SSH/headless; cần timeout + hướng dẫn (`gnome-keyring-daemon --unlock`) hoặc fallback sang vault + passphrase.
- Vault lai rẻ (dưới 1 ms) và gỡ được mọi giới hạn kích thước/Windows blob — đề xuất dùng ngay làm mặc định, keyring chỉ giữ khóa chủ.

## Hạn chế (trung thực)
- NOT RUN: Windows Credential Manager (giới hạn blob/UTF-16), macOS Keychain (update đè app, hỏi quyền hàng loạt), KWallet.
- gitleaks không có trên box → chỉ quét canary thủ công bằng grep.
- Chưa quét git history/log của một phiên Agenium thật (chưa có app); phải lặp lại ở M1.

## Quyết định đề xuất
Đi tiếp theo kiến trúc lai (keyring giữ khóa chủ + vault mã hóa) vì đã chứng minh chạy được; thêm timeout cho mọi thao tác keyring và kiểm tra `store_status()` khi khởi động. Phần Windows/macOS phải chạy tay hoặc qua CI (`spike.yml` chạy round-trip trên macOS/Windows runner — lưu ý runner CI không có phiên GUI keychain mở khóa thật nên chỉ là chỉ báo).
Ảnh hưởng tới quyết định số 5 (secret) trong Agenium-Kien-truc.md: xác nhận phương án B làm mặc định.
Evidence: `spikes/S5/`.
