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
13. **OpenCode model routing.** On opencode.ai bases the wire is chosen per model: Anthropic messages for `claude-*`,
    `minimax-*`, `qwen*`, `union-alpha`; Responses-only models (`gpt-*`, `grok-*`, `muse-spark`) are refused; the rest
    use chat/completions.
14. **Skills (M15-4).** An agent's skills = its role's skills (`roles/<role>/skills`) plus its own
    (`agents/<id>/skills`); an own skill with a role skill's name is its overlay (tagged `[updated]`). The agent's
    `skills` binding filters by name; empty = all. `load_skill` is answered by the run (no event); skill folders are
    extra read-only roots of `read_file`. The frontmatter parser supports the subset skills use (scalars, quoted
    values, block scalars; nested keys ignored) instead of a YAML dependency.
15. **Agent export (M15-5)** is a JSONL file (header, profile, agent-folder files, memory tables as typed rows, end line
    with line count + sha256); vectors are not exported (re-embedded on import); a stored secret value anywhere
    refuses the export. Import creates a new Solo agent of a project (team import arrives with M4). **Deletion** keeps
    the agent row as a tombstone (`core_002`: `deleted_at`, renamed `<name>~<id>`, Offline) so runs, tickets and
    events keep valid references; memory DB, folder and MCP working folder are removed (locked files are purged at
    the next start). Backups made before the deletion still contain the agent until retention prunes them.
16. **MCP (M15-3)** is Agenium's own client (no SDK dependency): JSON-RPC over stdio or Streamable HTTP, revision
    2025-06-18 requested. Servers live in `config.toml` (`[[mcp]]`, `command` xor `url`; https except localhost);
    values use `${secret:<name>}` or `${cred:<KEY>}` (resolved from the agent's binding, so agents can hold different
    keys for one server); secret-looking literals are refused, pasted JSON moves them to the vault. Sessions are per
    agent and server; read-only tools (`readOnlyHint`) retry once after a reconnect, others never; output is
    untrusted; review/chat runs only get read-only MCP tools. Stdio servers run with a clean environment in
    `<root>/mcp/<agent>/<server>` and a process-tree guard (job object / process group); there is no OS sandbox on
    Windows (ADR-39); the user configures every server.
17. **Embedding (M15-2)**: `bge-small-en-v1.5` (decision 2b) run with candle 0.11, CLS pooling, L2 norm, the BGE query
    instruction; files pinned to upstream revision `5c38ec7` and checked by sha256; installed by `agenium models
    download`. Until installed the hashing embedder is used; databases are re-embedded from text once the model is
    present. The default model was not changed (ADR-47 remains Proposed).
18. **Memory pipeline**: runs for the assignee when a ticket reaches Closed or Reopened (once per state), with the
    agent's own model at low effort; extraction and decision answer in JSON; ids outside the shown candidates,
    secret-looking facts and empty facts are rejected; facts from external content are stored untrusted and never
    recalled.
19. **Not yet built (explicitly):** graph memory (ADR-51), lessons/decay/overlay updates (M4.5), the ANN sidecar
    (exact sqlite-vec search is correct and below the 100 000-fact threshold), team templates, Linux/macOS sandbox
    backends (M2).

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
