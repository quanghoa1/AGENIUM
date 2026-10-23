# ADR-48: xtask workspace member; hand-written TS IPC types in M0

- **Status:** Proposed — tauri-specta deferred; needs owner confirmation
- **Date:** 2026-10-23 (Asia/Saigon)
- **Context / decision / consequences:**

- `xtask` (cargo alias) is an extra workspace member: `schemas [--check]`, `sandbox-attack-tests`, `ci`.
- REPO-LAYOUT asks for tauri-specta generated bindings. M0 ships hand-written `app/ui/src/api/types.ts` plus a Rust contract test (`ipc_json_shapes_match_the_ui_contract`) that fails when the JSON shape drifts. Switch to tauri-specta once its Tauri 2 release is stable enough for CI.
- Dashboard/Board/Settings/Diagnostics routes exist; Ticket detail, Agents, Monaco/xterm/dnd-kit components arrive with M1+.
