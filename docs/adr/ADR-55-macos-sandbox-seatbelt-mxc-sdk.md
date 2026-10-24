# ADR-55: macOS sandbox backend = Seatbelt through `mxc-sdk` (experimental)

- **Status:** Accepted for implementation; **not run on a Mac yet** (compiles and passes clippy for
  `x86_64-apple-darwin`)
- **Date:** 2026-10-24 (Asia/Saigon)
- **Amends:** ADR-43 (macOS Seatbelt v2, experimental) — implementation choice only.

## Decision

macOS shares the Linux module (`crates/agenium-sandbox/src/unix.rs`, `mxc_sdk::v1::spawn`) with
`Containment::Seatbelt(SeatbeltConfig::default())`: no GUI access, no keychain, no nested PTYs. Same allow-list policy as
Linux: worktree + temp writable; `/usr`, `/bin`, `/sbin`, `/System`, the Command Line Tools, Xcode, Homebrew, `/etc`
and the timezone database, PATH toolchains, the repository's git directory and skill folders read-only; network
denied; cleared environment. `detect()` reports `Seatbelt` only when `mxc_sdk::v1::available_backends()` lists it.
Codex's Seatbelt profiles (vendored, `vendor/codex-sandbox/upstream/codex-rs/sandboxing/src/*.sbpl`) allow reading the
whole disk except denied paths; they stay a reference for deny rules.

## Verification plan

Run `cargo test --workspace` on a macOS host (the owner proposed Devin cloud with the macOS platform); until then the
backend is labelled experimental in `doctor` and the docs.
