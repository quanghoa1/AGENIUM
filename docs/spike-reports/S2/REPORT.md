# Biên bản Spike S2 — Sandbox Windows

```text
Spike: S2 — Sandbox Windows (cổng quyết định)
Ngày chạy: 2026-10-24 (Asia/Saigon), máy: Windows 11 Pro 10.0.26300.9711, 10 nhân, 32 GB (máy dev, KHÔNG phải cấu hình
tối thiểu 4 nhân/8 GB); Codex CLI 0.163.0 (cài sẵn, không build từ nguồn); Python 3 (miniconda), Git for Windows,
Node 24.19, Rust 1.99.1 GNU.
Ngưỡng: bảng S2 trong docs/design/M0-SPIKE-CHECKLIST.md (chốt 2026-10-22, trước khi chạy; không nới).
Kết quả: MXC + profile quyền của Agenium: PASS mọi tiêu chí đo được (có điều kiện, xem "Hạn chế")
         MXC với quyền mặc định của Codex: KILL (đọc được ngoài worktree: ~/.ssh, DB Agenium, Documents)
         Restricted token "unelevated": KILL (raw TCP/DNS ra ngoài được; tiến trình tách rời sống sót sau run)
         "elevated" (user sandbox + firewall WFP): NOT RUN (cần quyền admin, chưa được chủ sở hữu duyệt)
```

Bản trước của biên bản này là "NOT RUN - needs Windows" (máy M0 chỉ có Linux). Đây là lần chạy đầu tiên.

## Bước 0 — Audit Codex hiện tại

- `codex-rs/windows-sandbox-rs` (upstream main, 2026-10-24): **121 file Rust, ~1,1 MB**, cộng `windows-sandbox-service`
  (Windows service, IPC, package identity). Phụ thuộc crate nội bộ `codex-protocol`, `codex-utils-*`, `codex-otel`,
  `codex-http-client`. Có binary setup cần nâng quyền (tạo local user sandbox, luật firewall WFP, DPAPI) cho chế độ
  **elevated**. Fork/vendoring nguyên trạng theo ADR-41 = kéo theo cả các crate nội bộ đó: công lớn, không làm trong spike.
- Codex có 3 backend Windows (`windows.sandbox` = `unelevated` | `elevated` | `mxc`, file
  `codex-rs/core/src/config/windows_sandbox_config.rs`):
  - `unelevated`: restricted token + ACL, **không** cần admin.
  - `elevated`: user sandbox riêng + firewall, cần admin một lần.
  - `mxc`: chuyển lệnh vào `BaseContainerRunner` của Microsoft (MXC/PSEC); README của crate `mxc-sandbox`: không sửa ACL
    máy, không tạo user, không chạy setup, không xin nâng quyền; job kill-on-close; chặn egress không phải loopback.
- Cách dùng trong spike: **không fork**, gọi binary Codex đã cài: `codex sandbox -c windows.sandbox="mxc" -P agenium --
  <lệnh>` với `CODEX_HOME` thuộc Agenium (không đụng `~/.codex` của người dùng) chứa profile quyền `agenium`
  (`evidence/agenium-profile.config.toml`): `:minimal` = read, `:project_roots` (= worktree) = write, toolchain
  (Python, Node, Git, `~/.cargo`, `~/.rustup`, MSYS2) = read, network tắt.
- Quyết định sản phẩm về admin/local user: chế độ đề xuất (MXC) **không cần cả hai**, nên không phải chốt trong spike này.

## Ma trận xâm nhập (worktree = cwd; mỗi dòng: kết quả theo chế độ)

Payload chạy trong sandbox (`evidence/payload.py`), harness ngoài sandbox (`evidence/harness.py`), kết quả thô
`evidence/s2-results.json` (danh sách file `~/.ssh` đã che).

| Tấn công | MXC + profile Agenium | MXC mặc định | unelevated |
|---|---|---|---|
| Ghi ra Documents, Desktop, `%USERPROFILE%`, Agenium root, worktree ticket khác, `.git` của project, `%TEMP%` | chặn | chặn | chặn |
| Ghi qua `..\`, UNC `\\localhost\C$`, đường dẫn dài `\\?\`, junction có sẵn, junction/symlink tự tạo | chặn | chặn | chặn |
| Đọc `~/.ssh`, config + DB Agenium, file trong Documents, worktree ticket khác | **chặn** | **thủng** | **thủng** |
| TCP thẳng tới IP ngoài (1.1.1.1:443, 8.8.8.8:53), IPv6 | chặn (WinError 10013) | chặn | **thủng** (IPv6 không có mạng) |
| DNS: resolver hệ thống, UDP thẳng tới 8.8.8.8:53 | chặn | chặn | **thủng** |
| Kết nối tới dịch vụ trên `127.0.0.1` của máy | chặn | chặn | **thủng** |
| Mở cổng nghe 0.0.0.0 | bind được (MXC công bố chặn inbound; chưa đo kết nối vào) | như trái | được |
| Tiến trình con tách rời (DETACHED + BREAKAWAY_FROM_JOB) | breakaway bị từ chối; con chết khi run kết thúc | như trái | **sống sót sau run** |
| Scheduled task, khóa Run trong registry | chặn | chặn | chặn |
| Biến môi trường bí mật của tiến trình cha | **lọt** (Codex chuyển nguyên env) | lọt | lọt |

Biến môi trường: Agenium luôn gọi lệnh với env sạch (`agenium_sandbox::clean_env`), nên điểm này thuộc trách nhiệm của
Agenium, không phải sandbox; phải giữ khi tích hợp.

## Kill cây tiến trình (20 lần mỗi chế độ)

Cây 7 tiến trình (cha + 2 con + 4 cháu, ghi heartbeat), kill như executor của Agenium (Job Object `KILL_ON_JOB_CLOSE`
gắn vào tiến trình `codex` + `taskkill /T /F`): **20/20 sạch ở cả 3 chế độ**, 0 tiến trình mồ côi, 0 heartbeat sau kill.

## Việc hợp lệ (trong sandbox, cwd = worktree)

| Việc | MXC + profile | Ghi chú |
|---|---|---|
| `git status`, `git diff`, `git log` | chạy | cần thêm quyền đọc thư mục `.git` của repo chính (git dir của worktree nằm ngoài worktree) và một gitconfig riêng cho agent (`GIT_CONFIG_GLOBAL`, `GIT_CONFIG_NOSYSTEM=1`) — `evidence/fixcheck.txt` |
| `git commit` | bị chặn | đúng thiết kế: Agenium tự commit ngoài sandbox (secret scan, trailer) |
| `python -m unittest` | chạy | |
| `node script.js` | chạy với `NODE_OPTIONS=--preserve-symlinks-main` | node mặc định gọi `lstat C:\` (realpath) → EPERM khi gốc ổ không đọc được |
| `cargo test` (crate 6 000 hàm, `CARGO_TARGET_DIR` trong worktree, `--offline`) | chạy | |
| Shell tương tác qua PTY | NOT RUN | |

## Overhead

| | ngoài sandbox | trong sandbox (MXC + profile) |
|---|---|---|
| `cargo build` crate mẫu (trung vị 3 lần, build sạch) | 15,57 s | 15,84 s (**+1,7 %**) |
| Khởi động một lệnh (`python -c pass`, trung vị 5) | 0,048 s | 0,319 s (+0,27 s/lệnh) |

## So với ngưỡng (chế độ đề xuất: MXC + profile Agenium)

| Tiêu chí | Ngưỡng Pass | Kết quả | Kết luận |
|---|---|---|---|
| Thoát ghi/đọc (gồm symlink/junction/UNC) | 0 thủng | 0 thủng (profile Agenium); MXC mặc định thủng đọc | **PASS** với profile, KILL nếu dùng quyền mặc định của Codex |
| Mạng trực tiếp (raw socket, DNS, localhost) | chặn ở tầng OS | chặn (WinError 10013), không dựa vào proxy/env | **PASS** |
| Kill cây | 20/20 | 20/20 | **PASS** |
| Việc hợp lệ | tất cả chạy | chạy, với 3 điều chỉnh (quyền đọc `.git`, gitconfig agent, cờ node) | **PASS** (PTY chưa đo) |
| Overhead build | ≤ 30 % | +1,7 % (+0,27 s mỗi lệnh) | **PASS** |
| Điều kiện cài đặt | admin chỉ 1 lần | không cần admin, không tạo user | **PASS** |

## Hạn chế (trung thực)

- Máy đo là Windows 11 build **26300** (bản Insider/Canary); MXC chỉ chạy khi máy có môi trường bảo mật tiến trình (PSEC).
  **Chưa kiểm tra trên Windows 11 bản phát hành ổn định** (23H2/24H2/25H2) — có thể MXC không có ở đó, khi đó phải
  fail-closed như hiện nay.
- Phụ thuộc binary Codex CLI do người dùng cài (đo 0.163.0): cần ghim phiên bản tối thiểu + contract test mỗi lần
  nâng cấp; Codex có thể đổi cờ/hành vi (MXC còn mới, upstream ghi rollout qua `features.prefer_mxc`).
- Chưa đo: kết nối vào cổng mà tiến trình trong sandbox mở; PTY; cấu hình máy tối thiểu; cache lạnh; antivirus khác
  Defender; đường dẫn mạng thật (SMB server khác `localhost`).
- `elevated` không chạy (cần admin). `unelevated` KILL nên không dùng.
- Payload chỉ kiểm tra những gì liệt kê; không phải audit bảo mật đầy đủ của MXC.

## Quyết định đề xuất (cần chủ sở hữu duyệt — sửa một phần ADR-39)

Windows có backend sandbox **`codex-mxc`**: Agenium gọi `codex sandbox` (MXC) với `CODEX_HOME` riêng và profile quyền do
Agenium sinh cho từng run (worktree ghi; `.git` của repo, gitconfig agent, toolchain đọc; mạng tắt), env sạch, Job Object
như hiện tại. `detect()` chỉ báo backend khi: có `codex` ≥ phiên bản đã kiểm, MXC khả dụng (probe chạy được) và một
self-test nhỏ (đọc `~/.ssh` bị chặn, TCP ra ngoài bị chặn) đạt; thiếu một điều kiện → không có backend → fail-closed như
hiện nay (YOLO bị từ chối, mọi lệnh hỏi người dùng). Dùng chung cho lệnh shell của agent, MCP stdio server và
`CliBackend` (M2.5). Vendoring mã Codex theo ADR-41 cho Windows để sau.
