# Biên bản Spike S4 — Git worktree, merge queue & reconcile sau crash

```text
Spike: S4 — Git worktree / merge queue / crash reconcile (CỔNG QUYẾT ĐỊNH)
Ngày chạy: 2026-10-23 (Asia/Saigon) / Grok Bot / máy: box Linux 8 vCPU / 15 GB, git 2.47, git-lfs có sẵn; không phải máy tối thiểu
Ngưỡng đã chốt ngày: 2026-10-23 08:59 +07 (commit ac2bb61) — trước khi chạy
Kết quả: PASS trên Linux (mô hình, SIGKILL) — Windows (đường dẫn dài), GitHub ghi thật, GitLab tự host: NOT RUN
```

## Cách đo
- `S4/am.py`: mô hình tham chiếu bằng Python + git thật. Mỗi ticket một worktree (`<project>/.agenium/worktrees/<id>`, đã vào `.git/info/exclude`), merge queue tuần tự (rebase → pre-merge check → `merge --ff-only`), **event ghi trước trạng thái** trong cùng một transaction SQLite, thao tác git idempotent (trailer `Agenium-Intent: <op-id>` trong commit để reconcile nhận biết), hàm `reconcile()` chạy lúc khởi động.
- **Lưu ý trung thực:** SQLite đóng vai SurrealDB (tính atomic của transaction tương đương; chưa chứng minh trên SurrealDB — S1 đã kiểm riêng độ bền SurrealDB khi kill). Repo mẫu 500 file. Remote thật chỉ có bare repo cục bộ (người dùng cấm push ra remote thật).
- Tiêm lỗi: 8 điểm × 3 lần xuất hiện, tiến trình bị `SIGKILL` (không thử mất điện/fsync lỗi); sau đó khởi động lại + `reconcile()` + kiểm bất biến (không mất commit đã có, event/trạng thái khớp, không còn worktree/branch rác, main sạch).
- Tập lệnh: `S4/am.py`, `S4/extra.py`; log: `S4/results/`.

## Số liệu chính
| Chỉ số | Ngưỡng Pass | Kết quả |
|---|---|---|
| 8 điểm tiêm lỗi | 8/8 tự lành/báo kèm hướng xử lý; 0 mất việc đã commit | **8/8 điểm, 24/24 lần tự lành, 0 mất commit, 0 can thiệp tay** (`injection.log`). Ví dụ log: `RECONCILE: T3: finished worktree removal`; điểm 8 `RECONCILE: push: remote already has main -> wrote PushDone` |
| Merge queue 50 lượt | 0 race, 0 sai thứ tự | **50 lượt, 0 vi phạm**, 29,6 s tổng (`queue50.log`) |
| Worktree Windows | Không lỗi đường dẫn dài | **NOT RUN - needs Windows.** Chỉ có ước lượng số học (KHÔNG phải kết quả đo): độ dài tệ nhất 149 (project 10 ký tự) → 245 (project 80 ký tự, ticket dài) so với giới hạn 260; cần tên thư mục ticket ngắn + `core.longpaths=true` + tiền tố `\\?\` |
| Phát hiện sửa ngoài | ≤ 5 s, revert đúng file | **1,0 ms**, 0 dương tính giả trên ghi của agent, chỉ file bị sửa bị gắn cờ/revert (`extra_part1_*.json`) |

Các mục checklist khác:
- **Tạo/xóa worktree 50 lần:** 7,1 s; 0 thư mục thừa, 0 đăng ký worktree cũ, `.agenium` đã nằm trong exclude, main không dơ. **Rác:** đóng ticket chưa merge để lại 50 branch (`git branch -d` từ chối) → đường "abandon" phải dùng `-D`, sau đó 0.
- **Hàng đợi có chủ đích (1 sạch, 1 conflict nhỏ, 1 conflict lớn, 1 check hỏng):** K1 merged; K2 conflict (1 file `shared.txt`); K3 conflict (2 file); K4 check fail (ticket về InProgress). Rebase được abort, worktree sạch, main chỉ chứa K1; event `conflict` ghi danh sách file.
- **Hai ticket cùng file, không khai `depends_on`:** phát hiện **ở bước merge** (rebase conflict) nếu trùng dòng; nếu sửa các hunk không giao nhau thì git merge sạch (cả A, B vào) — không cần chặn. Có thể phát hiện sớm hơn **ở commit** bằng so `git diff --name-only` giữa các ticket đang chạy (2,9 ms) — nên thêm làm cảnh báo mềm, không chặn cứng.
- **File lớn 100 MB × 10 ticket (dữ liệu ngẫu nhiên, không nén được):**
  | Đường | commit TB | merge TB | `.git` sau 10 ticket | store ngoài |
  |---|---|---|---|---|
  | git thường | 3,09 s | 0,67 s | 1049 MB | 0 |
  | git LFS | 1,17 s | 0,79 s | 1049 MB (kho LFS nằm trong `.git/lfs`) | 0 |
  | content-hash store (ngoài git) | 0,02 s | 0,01 s | 0 | 1049 MB |
  → git thường phình đúng bằng dữ liệu; LFS nhanh hơn khi commit nhưng vẫn phình cục bộ; hash-store nhanh nhất và giữ repo nhỏ nhưng cần Agenium tự chịu trách nhiệm GC/đồng bộ. Khuyến nghị: ngưỡng kích thước → đẩy sang hash-store (đúng quyết định 11), cảnh báo người dùng.
- **Remote:** `ls-remote` bare cục bộ 13 ms; `push --dry-run` OK; push ref tạm + thấy + xóa OK (chỉ trên bare cục bộ); GitHub công khai `ls-remote` OK 0,24 s; repo không tồn tại/riêng tư → rc=128 "Repository not found / Authentication failed" (thông báo rõ). **GitHub ghi thật và GitLab tự host: NOT RUN** (không có credential/instance, và bị cấm push remote thật).

## Điều bất ngờ / phát hiện
1. Reconcile chỉ đúng nếu **mọi thao tác git có ý định (op-id) ghi trước** vào event và dùng trailer `Agenium-Intent`; điểm 5 (merge xong chưa ghi `MergeDone`) và điểm 8 (push xong chưa ghi `PushDone`) chỉ tự lành nhờ kiểm tra trạng thái thật của git (HEAD/remote) chứ không tin DB.
2. inotify không biết ai ghi: cơ chế revert cần tool-layer của agent ghi hash các ghi của chính nó; nếu người dùng và agent ghi cùng file đồng thời thì mơ hồ → thiết kế phải **khóa/hỏi người dùng**, không revert im lặng.
3. `git gc --auto` có thể chạy giữa test với repo lớn (đã thấy một lần) — không ảnh hưởng đúng/sai nhưng làm số đo commit lớn nhiễu.
4. Cần đường `-D` khi abandon ticket và dọn `refs` ticket đã bỏ.

## Hạn chế (trung thực)
- SQLite thay SurrealDB; chỉ SIGKILL tiến trình, không mất điện, không ENOSPC/EIO; một người dùng, một máy; không có pre-merge check thật (dùng script giả).
- Windows/macOS: NOT RUN (đặc biệt đường dẫn dài, khóa file khi xóa worktree, CRLF/autocrlf).
- GitHub ghi/GitLab: NOT RUN.

## Quyết định đề xuất
Đi tiếp với mô hình worktree + merge queue tuần tự + event-trước-state + reconcile idempotent (phương án chính; không cần phương án B copy+diff ở Linux). Phải thêm: `branch -D` ở abandon, cảnh báo trùng file bằng `--name-only`, đặt tên worktree ngắn trên Windows, kiểm lại trên Windows trước khi công bố đa nền tảng.
Ảnh hưởng quyết định 22/11 trong Agenium-Kien-truc.md: giữ nguyên; bổ sung hash-store cho file lớn.
Evidence: `spikes/S4/results/`.
