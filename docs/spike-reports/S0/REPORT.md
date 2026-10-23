# Biên bản Spike S0 — Môi trường & baseline build (chỉ Linux)

```text
Spike: S0 — Môi trường & baseline build
Ngày chạy: 2026-10-23 (giờ Asia/Saigon) / người chạy: Grok Bot (tự động) / máy đo: box Linux — Intel Xeon 8 vCPU, ~15 GB RAM (+8 GB swap), Debian 13 (kernel 6.12.94), rustc/cargo 1.99.1 (rustup), clang 19.1.7, gcc 14.2
Ngưỡng đã chốt ngày: 2026-10-23 08:59 +07 (commit ac2bb61, spikes/THRESHOLDS-M0-SPIKE-CHECKLIST.md) — trước khi chạy
Kết quả: PASS (Linux)  |  Windows 11: NOT RUN - needs Windows  |  macOS: NOT RUN - needs macOS
```

## Số liệu chính (thô: `S0/results/s0_builds.csv`, `*.time`, `*.build.log`, `strip_sizes.txt`, `incremental.txt`)

| Biến thể (release, thin-LTO, `opt-level=3`) | Build sạch | Peak RSS rustc | Binary (raw) | Binary (strip) | `target/` |
|---|---|---|---|---|---|
| Tauri 2 rỗng (baseline) | 63 s | 1,36 GB | 10,8 MB | 7,0 MB | 1,2 GB |
| Tauri + surrealdb(kv-surrealkv) + fastembed + keyring + git2 | **320 s (5,3 phút)** | 5,2 GB | 196,3 MB | 148,3 MB | 3,5 GB |
| Tauri + surrealdb(kv-rocksdb) + fastembed + keyring + git2 | **515 s (8,6 phút)** | 5,1 GB | 207,5 MB | 157,4 MB | 3,7 GB |
| Build tăng dần (sửa `main.rs`, cả hai engine) | ~124 s / ~123 s | | | | |

Tăng kích thước binary do SurrealDB (đo cô lập bằng crate `s1` = surrealdb + tokio + rusqlite, không Tauri): **SurrealKV 114,3 MB (strip) / 151,1 MB (raw); RocksDB 123,5 MB (strip) / 162,4 MB (raw)**. Hiệu "full − baseline" (gồm cả fastembed/ort, git2, keyring): 141 MB (SurrealKV) / 150 MB (RocksDB) khi strip.

## So với ngưỡng
| Tiêu chí | Ngưỡng | Đo được | Kết luận |
|---|---|---|---|
| Build sạch | Pass ≤ 45 phút, Kill > 90 | 5,3 / 8,6 phút (8 vCPU) | PASS — nhưng KHÔNG phải máy tối thiểu (xem hạn chế) |
| Tăng binary do SurrealDB | Pass ≤ 150 MB, Kill > 250 MB | 114–124 MB (strip); 151–162 MB nếu không strip | PASS khi strip (release nên `strip = true`); không strip thì chạm sát/vượt 150 MB → **đặt `strip=true` thành yêu cầu** |
| Build trên máy sạch chỉ với bước có trong tài liệu | không workaround | Cần danh sách apt dưới đây; 1 workaround riêng của box (conffile prompt của `fuse3` khi `apt upgrade`, không liên quan Agenium) | PASS (Linux) |

Tiêu chí Kill đã chạm: không.

## Yêu cầu hệ thống phát sinh (cần ghi vào tài liệu cài đặt)
- Rust toolchain mới qua **rustup** (stable 1.99.1). Không dùng cargo của distro (1.85.1).
- apt: `build-essential clang libclang-dev cmake pkg-config libssl-dev libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev libsoup-3.0-dev libsecret-1-dev libdbus-1-dev libgit2-dev` (+ `gnome-keyring dbus-x11 libsecret-tools` cho runtime keyring/test).
- **clang/libclang + cmake chỉ bắt buộc cho đường RocksDB** (bindgen/cc); SurrealKV thuần Rust không cần.
- **RAM build đỉnh ~5,2 GB** (link thin-LTO các crate surrealdb) → máy tối thiểu 8 GB sẽ chật khi vừa build vừa chạy IDE; khuyến nghị `CARGO_BUILD_JOBS=2–4`, hoặc CI build.

## Điều bất ngờ
- RocksDB chậm hơn SurrealKV ~1,6× khi build, +10 MB binary, thêm yêu cầu clang/cmake.
- Build tăng dần vẫn ~2 phút (thin-LTO + link binary 150–200 MB) — vòng lặp dev nên dùng profile `dev` riêng/`lto=off`.
- RAM "used" dao động mạnh khi build (đã thêm swap 8 GB để an toàn).

## Hạn chế (trung thực)
- 8 vCPU / 15 GB là mạnh hơn "máy tối thiểu" → thời gian cần ngoại suy (ước ~2× trên 4 core vẫn << 45 phút), chưa đo.
- Không đo Windows (MSVC Build Tools, LLVM cho RocksDB) và macOS → `NOT RUN`. Workflow CI `.github/workflows/spike.yml` đã chuẩn bị để chạy phần S0 trên `macos-latest`/`windows-latest`.
- Không đo build *thật sự sạch* (đã có cache registry cargo sau lần đầu; thời gian tải crate không tính).

## Quyết định đề xuất
Đi tiếp như thiết kế (Linux). Điều kiện: `strip=true`, profile dev nhẹ, ghi danh sách apt/clang vào tài liệu. Ưu tiên SurrealKV trước nếu S1 cho phép (build nhanh hơn, ít phụ thuộc).
Ảnh hưởng tới quyết định số 24 (yêu cầu hệ thống) trong Agenium-Kien-truc.md: thêm clang/cmake (RocksDB), WebKitGTK 4.1 dev, RAM build.
Evidence: `spikes/S0/` @ git repo `/workspace/agenium`.
