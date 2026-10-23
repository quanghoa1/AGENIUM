# ADR-43: macOS Seatbelt profile v2

- **Status:** Accepted (owner steering 10:46/11:05); implementation EXPERIMENTAL and NOT RUN
- **Date:** 2026-10-23 (Asia/Saigon)
- **Context / decision / consequences:**

- Profile = vendored Codex `.sbpl` (`include_str!`) + broad `file-read*` + explicit deny exceptions (must deny `~/.ssh`, keychains, browser data); paths passed with `-D` params.
- Spawn through `env -i` with an explicit env allowlist and wrap with `ulimit` (Seatbelt does not limit fork/RAM).
- No macOS machine was available: the code compiles only on macOS (cfg-gated) and `macos_seatbelt_attack_suite` has never been executed by the author. The owner's 30-case macOS suite must be ported before the backend is called supported.
- Packaging note: the raw `target/release` binary shows a blank webview on macOS; only the `.app` bundle renders. Unsigned app: Settings > Privacy & Security > Open Anyway.
