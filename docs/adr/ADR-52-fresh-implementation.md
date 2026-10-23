# ADR-52: Fresh implementation on the design pack; Windows as the verification machine

- **Status:** Accepted (owner instruction 2026-10-23: "triển khai mới hoàn toàn, M0 chỉ là thử nghiệm, đừng dùng code cũ, chỉ dùng kết quả cũ, kiểm chứng lại")
- **Date:** 2026-10-23 (Asia/Saigon)

## Context

The design pack (`docs/design/`, `docs/adr/ADR-39…51`, `docs/spike-reports/`) came with an M0 trial codebase. The owner
treats that code as an experiment. This repository is a new implementation that reuses only the decisions and the
measured results, and re-verifies them where this machine allows. The development and verification machine is
Windows 11 (Rust GNU toolchain + MSYS2 gcc, no MSVC); the first release still targets Linux + macOS (ADR-39 stands).

## Decisions

1. **No trial code is reused.** Schemas, SQL and tool schemas are rewritten from the design documents; generated
   copies live in `schemas/` (`cargo xtask schemas --check`).
2. **Windows policy.** Everything builds and is tested on Windows. There is no sandbox backend on Windows, so YOLO
   (approval `never`) is refused (fail-closed) and every agent command needs the user's approval (M1-7). Process trees
   are killed through a Job Object (`KILL_ON_JOB_CLOSE`); without it Git-Bash grandchildren survived `taskkill /T`.
3. **Idempotency key assigned by the harness.** A protocol call's `intent_id` is `<run id>:<vendor tool-call id>`, so a
   replayed model turn maps to the same intent. Models do not invent ULIDs; the published tool schemas therefore omit
   `intent_id` (deviation from `docs/design/SCHEMAS/tools/*`).
4. **Idempotency table.** One intent may produce several events, so idempotency lives in an `intent` table
   (id -> stored result) committed in the same transaction as the events, instead of `UNIQUE(event.intent_id)`.
5. **Git journal table.** Journal-first git operations (ADR-44) are rows of `vcs_op` (pending/done/failed) written with
   their `VcsOp` event; reconcile replays every pending row. Operations: create_worktree, commit, merge,
   remove_worktree.
6. **Incremental backups.** Append-only tables (`event`, `intent`, `run_log`) carry only rows past the base full's cut;
   the small mutable tables are dumped whole. Agent databases are snapshotted in full in every full set (their
   incremental form waits for the memory pipeline, M1.5). Restore verifies sha256 and a per-table content hash.
7. **Secrets.** The hybrid vault is the default (spike S5). Re-measured on Windows Credential Manager: blobs up to
   2560 bytes round-trip, 4096 bytes are rejected; through the vault every size works.
8. **Permission grants "per run"** are bound to the ticket's current work cycle (a parked run continues in a new
   run) and revoked when the ticket becomes Resolved/Closed/Cancelled. Always-allow (`scope = project`) stays
   user-only, enforced by a schema trigger.
9. **Runs.** Three kinds: *work* (assignee, work + worker tools), *review* (Lead/Solo: report + diff, approve -> merge
   queue -> close), *chat* (Lead with the user). A parked or interrupted ticket gets a continuation run that resumes
   the stored conversation; `max_steps` or ~85 % of the context window ends the run with a tool-less checkpoint note
   that seeds the next run. Two stopped/failed runs in a row stop auto-restarts and raise "Needs you" (first step of
   the M4 stall-nudge).
10. **Anthropic adapter** (raw HTTP; no official Rust SDK): API version `2026-10-21`, no sampling parameters,
    `output_config.effort`, adaptive thinking on 4.6+ models (budget only on older ones), `tool_choice` left `auto`,
    thinking blocks replayed with signatures (append-only history), cache breakpoints after tools / stable system /
    last message, 1 h TTL first. OpenAI-compatible and Gemini adapters follow the architecture document.
11. **Interim front-end:** a headless CLI (`crates/agenium-cli`, binary `agenium`) drives the same App/Runtime API the
    desktop UI will use. The UI is out of scope for now (owner instruction).
12. **OpenCode relay flavor.** For `opencode.ai` base URLs the OpenAI-compatible adapter sends `x-opencode-session` =
    the conversation cache key (OpenCode Go rejects requests without it; same value per conversation keeps the upstream
    cache warm), and DeepSeek models get DeepSeek effort levels and `reasoning_content` replay.
13. **Not yet built (explicitly):** the real local embedding model (a deterministic hashing embedder stands in), the ANN
    sidecar (exact sqlite-vec search is correct and below the 100 000-fact threshold), MCP, skills, team templates,
    Linux/macOS sandbox backends (M2).

## Re-verification on this machine (Windows 11 Pro, 10 cores, 32 GB)

| Earlier result (Linux) | Re-verified here | Evidence |
|---|---|---|
| S1/ADR-49: 20x `kill -9` -> 0 lost, 0 seq gaps | 20 hard kills (TerminateProcess) of a writer: integrity ok, gap-free log, every acknowledged intent present, no half-applied intent | `crates/agenium-store/tests/crash.rs` (`AGENIUM_CRASH_ROUNDS=20`) |
| S4: 8 injection points self-heal, 0 lost commits | 8 points (before/after git for create-worktree, commit, merge, remove-worktree) heal on restart; merge applied exactly once | `crates/agenium-orchestrator/tests/reconcile.rs` |
| S4: sequential merge queue without races | 20 tickets merged sequentially, linear history, no lost file | `crates/agenium-vcs/tests/vcs.rs` |
| S5: secrets round-trip, no plaintext | 7 secret types through the vault; canaries absent from disk; Credential Manager limit 2560 B | `crates/agenium-secrets/tests/secrets.rs` |
| S7: broadcast + log resync, 0 lost/dup | 300-event burst with a 2-slot channel: exactly once, in order, for 3 subscribers; restart resumes from cursor | `crates/agenium-bus/tests/bus.rs` |
| S3: tree kill without orphans | Windows Job Object: timeout kills background grandchildren (30 s -> 1 s test) | `crates/agenium-sandbox` |

Not re-verifiable here: Linux bubblewrap/Landlock and macOS Seatbelt (M2), embedding quality (S6), UI/FPS (S7).
