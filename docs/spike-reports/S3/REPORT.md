# Biên bản Spike S3 — Sandbox Linux (macOS: NOT RUN)

```text
Spike: S3 — Sandbox macOS + Linux (đây chỉ chạy phần Linux)
Ngày chạy: 2026-10-23 (Asia/Saigon) / Grok Bot / máy: box Linux Debian 13, kernel 6.12.94 (Landlock ABI 6; LSM: capability,landlock,selinux,bpf), 8 vCPU; bubblewrap 0.12.0; container/VM của box (một số test bị chặn sẵn bởi môi trường được đánh dấu)
Ngưỡng đã chốt ngày: 2026-10-23 08:59 +07 (commit ac2bb61) — trước khi chạy
Kết quả: PASS (Linux, backend `sbx` = Landlock + seccomp; bwrap cũng PASS) — kèm 2 khoảng hở đã ghi (kernel < 6.2, kill cứng supervisor)  |  macOS (sandbox-exec/Seatbelt): NOT RUN - needs macOS
```

## Cách đo
- `sbx` (S3/sbx/src/main.rs): launcher Rust tự viết, gọi thẳng syscall Landlock (có thể giả lập ABI thấp bằng `--abi N` — **mô phỏng**, không phải kernel 5.13 thật), seccomp-bpf (chặn `socket()` mọi họ; chặn x32 ABI; io_uring, mount, unshare, ptrace…), `PR_SET_DUMPABLE=0`, đóng fd ≥ 3, env allowlist (PATH/LANG/TERM/LC_ALL + `--env` rõ ràng), subreaper để diệt cả cây tiến trình, **fail-closed**.
- Bộ test xâm nhập (`S3/tests/payload.py`, 45 mục = 36 tấn công + 8 việc hợp lệ + 1 daemon; chạy với baseline "none" để chứng minh test có sức phân biệt): ghi/đọc ngoài vùng, `..`, symlink, hardlink, rename, `.git` chính (hook/config), mạng TCP/UDP/DNS/unix path/abstract/netlink/raw, chmod/truncate/utime/xóa file ngoài, đọc khóa SSH/`/etc/shadow`/liệt kê home, canary trong env, `/proc/*/environ`, `/proc/*/mem`, ptrace, mount, userns, io_uring, bypass x32, daemon sống sót.
- Chạy ở 7 chế độ: none, sbx, sbx-abi1, sbx-abi3, sbx-unix (`--allow-unix`), sbx-procall, bwrap. Ma trận: `S3/results/pentest_matrix.txt`; JSON thô: `pentest_*.json`.

## Số liệu chính
| Tiêu chí | Ngưỡng | Kết quả |
|---|---|---|
| Thủng nghiêm trọng mặc định (`sbx`, ABI 6) | 0 | **0**: 36 test tấn công → 34 bị chặn, 2 lọt: `chmod_outside`, `utime_outside` (chỉ đổi metadata file ngoài; Landlock không kiểm soát chmod/chown/utime — không đọc/ghi nội dung). Mọi đọc/ghi/xóa/truncate/rename/hardlink/symlink ngoài vùng, mạng (TCP/UDP/DNS/unix/netlink/raw), canary env, `/proc/*/environ`, ptrace, userns, đều bị chặn. Baseline "none" cho OK ở 29/36 test (7 test còn lại — write_etc, read_etc_shadow, proc_sibling_mem, mount_tmpfs, io_uring, x32, raw_socket — đã bị chặn sẵn bởi môi trường nên **không có giá trị phân biệt**, không tính vào kết luận) |
| Việc hợp lệ trong sandbox | không vỡ | 8/8 ở ABI mặc định (ABI1 giả lập: 7/8, hardlink trong project bị chặn): ghi/mkdir/rename trong project, /dev/null, socketpair, subprocess, `git commit`, hardlink trong project (trừ ABI1) |
| bwrap (`--unshare-all`, root tmpfs, chỉ bind project) | 0 thủng | 0 thủng ra host; các dòng "OK" của bwrap (write_tmp, write_dotdot, netlink, /proc env, userns) xảy ra **bên trong namespace riêng** (tmpfs/pidns riêng; đã kiểm file /tmp không xuất hiện trên host) — bwrap còn chặn cả chmod/utime ngoài |
| Overhead (git ×10 lệnh, cargo build crate nhỏ, python, node; trung vị) | ≤ 30% | **sbx: +7,8% (git), +6,7% (cargo), +4,3% (python), +4,4% (node)**; bwrap: −4,4% / −1,8% / +1,6% / +1,1% (`S3/results/overhead.csv`) → PASS |
| Fail-closed khi thiếu cơ chế | từ chối chạy | PASS: `SBX_SIM_NO_LANDLOCK=1` → `REFUSING TO RUN (fail-closed)` rc=126; `--require-abi 99` trên host ABI 6 → rc=126, lệnh KHÔNG chạy (`S3/results/failclosed.txt`) |
| Tiêu chí diệt cây tiến trình (20 lần, cây: con, cháu, `setsid` double-fork daemon) | 0 sống sót | **SIGTERM supervisor: sbx 0/20 sống sót, bwrap 0/20; SIGKILL supervisor (Agenium chết cứng): sbx = 5 tiến trình mồ côi mỗi lần (100/100 lần sống sót), bwrap 0** (`S3/results/killtree.txt`) |

Tiêu chí Kill đã chạm: không (xem khoảng hở). Cách phân loại: chmod/utime ngoài vùng coi là rủi ro metadata (ghi rõ, không phải thủng nội dung); `truncate` ở ABI < 3 là thủng thật nhưng chỉ ở kernel < 6.2 và được chặn bằng fail-closed (`--require-abi 3`). Nếu giữ nguyên mức tối thiểu kernel 5.13 và không fail-closed thì tiêu chí "0 thủng" **không** đạt ở kernel cũ. Parity với Windows: NOT RUN (S2 chưa chạy).

## Khoảng hở / điều bất ngờ
1. **Kernel < 6.2 (Landlock ABI < 3)**: `truncate_outside` KHÔNG bị chặn (giả lập ABI1) → có thể cắt rỗng file ngoài vùng. Thiết kế nhắm kernel ≥ 5.13 (ABI 1) nên đây là khoảng hở thật với Ubuntu 22.04 kernel 5.15. Cần: fail-closed khi ABI < 3 (`--require-abi 3`) hoặc dùng bwrap/Docker thay thế; ABI1 còn chặn hardlink hợp lệ trong project (thiếu quyền REFER).
2. **Landlock không kiểm soát chmod/chown/utime** → file ngoài vùng vẫn bị đổi quyền/mtime (không phải nội dung). Bwrap chặn được. Ghi vào tài liệu như rủi ro còn lại, hoặc ưu tiên bwrap khi có.
3. **Kết nối unix socket theo đường dẫn chỉ chặn được từ Landlock ABI mới**: `sbx` chọn chặn bằng seccomp (cấm `socket()` hoàn toàn); `--allow-unix` mở lại thì thủng tới daemon host (đã đo: unix_socket_path_host_daemon OK ở chế độ sbx-unix) → không bao giờ bật mặc định.
4. `/proc/<pid>/environ` và ptrace bị chặn trong `sbx` (Landlock không cấp đọc `/proc` ngoài + `PR_SET_DUMPABLE=0`); env allowlist là lớp phụ quan trọng — canary không xuất hiện trong env của sandbox.
5. **Kill cứng supervisor để lại daemon mồ côi** (sbx; không có PID namespace): cần Agenium dọn khi khởi động (đánh dấu tiến trình bằng env/cgroup run-id rồi diệt) hoặc dùng bwrap `--die-with-parent` (pidns, 0 sống sót). Đã sửa 1 lỗi của launcher trong lúc đo: supervisor không `waitpid` zombie nên mỗi lần chạy `git commit` bị trễ ~1 s do `git maintenance --detach` (đã sửa; overhead trên là sau sửa).
6. Công cụ cần thư mục tạm ghi được: `cargo`/`rustc` fail nếu `/tmp` không ghi được → launcher phải đặt `TMPDIR` riêng trong vùng rw của run.
7. Worktree: `.git` của worktree trỏ về `.git` chính (gitdir/commondir) → phải cấp quyền đọc/ghi hẹp tới `.git/worktrees/<id>` và `.git/objects`, `refs` nhưng **không** hooks/config (đã test write_main_git_hook/config = BLOCKED) — cấp thêm thì thành đường thoát (hook chạy ngoài sandbox).
8. Tách bạch: env canary không lọt (allowlist) — khớp yêu cầu S5.

## Hạn chế (trung thực)
- Không có kernel 5.13/5.15 thật; ABI thấp chỉ được giả lập qua `--abi`. Không thử Ubuntu AppArmor userns restriction, SELinux enforcing, hay container lồng; một số test (raw socket, `/etc/shadow`, mount…) bị chặn sẵn ở baseline (đánh dấu inconclusive, không tính vào kết luận).
- Chưa thử Landlock với network rules thật (ABI 4+) cho cho phép cổng chọn lọc; không đo bypass bằng io_uring thật ngoài seccomp.
- macOS Seatbelt: **NOT RUN - needs macOS**. Workflow `.github/workflows/spike.yml` chuẩn bị phần S0 trên runner macOS/Windows; S3 macOS cần viết profile `sandbox-exec` riêng (chưa viết — không giả lập).
- S2 (Windows): NOT RUN.

## Quyết định đề xuất
Đi tiếp: `sbx`/Landlock+seccomp làm backend Linux mặc định với fail-closed **và yêu cầu ABI ≥ 3 (kernel ≥ 6.2)**; kernel cũ hơn → dùng bwrap nếu có, nếu không từ chối chạy agent không sandbox (trừ khi người dùng bật rõ). Bổ sung dọn tiến trình mồ côi lúc khởi động.
Ảnh hưởng quyết định sandbox Linux trong Agenium-Kien-truc.md: nâng yêu cầu tối thiểu từ "kernel ≥ 5.13" lên ≥ 6.2 cho backend Landlock (hoặc bắt buộc bwrap ở kernel cũ).
Evidence: `spikes/S3/`.
