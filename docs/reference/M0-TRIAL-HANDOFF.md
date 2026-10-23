# HANDOFF — Agenium M0 (tóm tắt ngắn, tiếng Việt)

Cập nhật: 2026-10-23 (giờ Asia/Saigon). Repo git cục bộ, **chưa push**. Chi tiết: `PROGRESS.md`, `docs/STORE-PARITY.md`, `docs/STORE-SCALE.md`, `docs/adr/`.

## Đã thay đổi lớn
- **Store:** SQLite là mặc định (rusqlite bundled + sqlite-vec + FTS5). SurrealDB chỉ còn là tính năng tùy chọn `--features surrealkv` (ADR-49/50). Có kiểm thử so sánh hành vi SQLite ↔ SurrealKV (0 khác biệt).
- **Vector:** tìm chính xác (exact) mặc định; tự chuyển sang HNSW (usearch) khi ≥ **100.000 facts/agent** (cấu hình được). Từ 200k vector, `ef_search` tự nâng lên 256. Số đo: `docs/STORE-SCALE.md` (Linux đầy đủ; macOS là số bạn gửi).
- **Nền tảng:** Linux + macOS (ADR-39). Windows hoãn: xem `docs/WINDOWS-GUIDE.md`.
- **CI:** GitHub Actions hết quota → mọi thứ chạy cục bộ bằng `scripts/ci-local.sh` (Linux/macOS) hoặc `scripts/ci-local.ps1` (Windows). Các workflow trong `.github/workflows/` giữ lại nhưng **chỉ chạy thủ công** (`workflow_dispatch`).

## Chạy trên macOS (từng bước)
Cần: Xcode Command Line Tools (`xcode-select --install`), Rust (rustup), Node 20, pnpm 10 (`corepack enable`), git ≥ 2.30.

```bash
cd agenium-app
# 1) Một lệnh chạy tất cả (khuyến nghị) — in bảng PASS/FAIL/SKIP, log ở target/ci-local/<bước>.log
scripts/ci-local.sh
#    thêm --heavy để chạy bench/scale nặng (chậm, ~3 GB đĩa; dùng corpus tổng hợp nếu không có dữ liệu thật)
#    --quick = chỉ fmt + clippy + test;  --skip-ui, --skip-surreal, --skip-sandbox, --fail-fast

# 2) Hoặc từng lệnh
cargo test --workspace                          # toàn bộ test Rust (SQLite mặc định)
cargo test -p agenium-store --features surrealkv # bộ test SurrealKV (tùy chọn, build lâu)
pnpm --dir app/ui install --frozen-lockfile
pnpm --dir app/ui build                          # build UI (vitest: pnpm --dir app/ui test)
cargo xtask schemas --check                      # kiểm tra schema/SQL sinh ra khớp

# 3) Gói .app / .dmg
cd app && ui/node_modules/.bin/tauri build       # kết quả trong target/release/bundle/macos/*.app và dmg/
open ../target/release/bundle/macos/Agenium.app  # (tên file có thể khác một chút)

# 4) Sandbox attack tests (bắt buộc có backend Seatbelt, nếu không sẽ FAIL — fail-closed)
AGENIUM_REQUIRE_SANDBOX=1 cargo xtask sandbox-attack-tests
```

**Lưu ý macOS:** backend Seatbelt đã viết và có test nhưng **chưa từng chạy thật** (chưa có Mac) — đây là mục kiểm tra quan trọng nhất của bạn. Nếu `sandbox-attack-tests` FAIL, gửi lại `target/ci-local/sandbox-attack-tests.log`. Bộ 30 case của bạn chưa được import.

## Windows
Không hỗ trợ ở v1 (sandbox không có backend → fail-closed). Gói `Agenium-complete-pack.zip` có `docs/WINDOWS-GUIDE.md` và `WINDOWS-PROMPT.md` để AI trên Windows chạy thử và báo cáo theo mẫu cố định.

## Chưa chạy / chưa kiểm chứng (NOT RUN)
- macOS: toàn bộ test (Seatbelt, ANN, bench) — chưa chạy lại sau khi sửa; Windows: chưa từng build.
- Mất điện thật (power loss); SurrealKV ở 500k/1M facts; Tauri crate (`agenium-app`) không biên dịch được trên máy build Linux của tôi (thiếu GTK/WebKit dev) — `ci-local.sh --no-tauri` bỏ qua nó; hãy chạy bản đầy đủ trên Mac.
- Chi tiết số lần chạy xác minh cuối: mục "Final verification" trong `PROGRESS.md`.

## Cần bạn xác nhận (deviations)
git CLI thay vì git2/gix (ADR-44) · tauri-specta hoãn (ADR-48) · xtask là workspace member · CI chỉ Linux+macOS và chỉ thủ công · mã Rust của Codex sandbox chỉ để tham khảo (ADR-41) · embedder mặc định (ADR-47) · chuyển sang SQLite (ADR-49) · phụ thuộc C++ của usearch · retention chỉ SQLite · ngưỡng ANN 100k · `ef` tự nâng 256 từ 200k · mmap 1 GiB / WAL gate 64 MB mặc định.

**Graph memory (ADR-51, Proposed, chỉ thiết kế):** cần bạn duyệt §10 (cách chia sẻ tri thức giữa agent: ghi chú Lead qua ticket / gói tri thức project `.agenium/knowledge`, đụng quyết định 2 "memory riêng tư"), quy tắc scope mặc định của lesson, trần hub và giới hạn thời gian truy vấn. Số liệu CTE là probe python trên đồ thị tổng hợp, không phải store Rust. Xem `docs/adr/ADR-51-graph-memory-on-sqlite.md`, `schemas/sql/drafts/`.
