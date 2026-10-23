# ADR-39: First release is Linux + macOS only

- **Status:** Accepted (owner decision, 10:49)
- **Date:** 2026-10-23 (Asia/Saigon)
- **Context / decision / consequences:**

The owner decided the first release targets Linux and macOS. Windows is deferred.

- M0 CI matrices, Tauri bundle targets and `deny.toml` targets contain no Windows entry.
- Windows-specific code paths stay `cfg`-gated but are unsupported and untested; the Codex Windows sandbox crates are not vendored.
- macOS is **experimental** until the Seatbelt attack suite has run green on a real runner (CI macOS jobs are `continue-on-error` until then).
- Supersedes the 'Windows/macOS/Linux' scope in decision 9 for the first release only; Windows plan B (WSL2 / container / experimental label) remains the future path.
