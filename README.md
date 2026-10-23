# Agenium

Rust application that orchestrates AI agents — a Solo agent, or a Lead with Workers — through a built-in ticket system.
Every change of state goes through typed, validated tool calls; work happens in a git worktree per ticket and reaches
`main` only through review and a sequential merge queue.

**Status:** milestones M0 (backend) and M1 (Solo agent) are implemented and tested; the desktop UI is not part of this
phase — the headless CLI `agenium` is the front-end. Details and the honest NOT RUN list: [`docs/PROGRESS.md`](docs/PROGRESS.md).
This is a fresh implementation of the design in [`docs/design/`](docs/design) (priority on conflict: `DECISIONS.md`);
its own decisions are in [`docs/adr/ADR-52-fresh-implementation.md`](docs/adr/ADR-52-fresh-implementation.md).

## Build and test

Requirements: Rust stable (>= 1.89), a C/C++ compiler (bundled SQLite, sqlite-vec, zstd), git >= 2.30.
On Windows this repository is developed with the GNU toolchain and MSYS2 gcc.

```sh
cargo build --workspace
.\scripts\ci-local.ps1        # Windows: fmt, clippy -D warnings, tests, schema check, scripted golden tasks
scripts/ci-local.sh           # Linux/macOS
cargo xtask schemas           # regenerate schemas/ (tool argument schemas, SQL)
cargo run -p agenium-eval     # golden tasks with the scripted model
```

## Use (CLI)

```sh
agenium setup --user-language vi                     # root: AGENIUM_HOME > chosen root > ~/.agenium
agenium doctor                                       # requirements, sandbox, keyring
agenium provider add anthropic --kind anthropic --key-env ANTHROPIC_API_KEY   # key goes to the encrypted vault
agenium project create demo
agenium agent solo --project demo --name ada --provider anthropic --model claude-opus-5-5 --effort high
agenium chat --project demo "Thêm trang đăng nhập bằng Google"   # the Solo agent plans and creates tickets
agenium run --project demo                           # work runs + self-review + merge + close
agenium ticket list --project demo
agenium answer --project demo AGN-1 <question-id> "Use port 8080"
agenium events --follow
agenium backup full && agenium backup list
```

Safety model in this phase: there is no OS sandbox on Windows (v1 targets Linux + macOS, ADR-39), so YOLO mode is
refused and **every shell command an agent wants to run is shown to you for approval** (`[y]es / [a]lways / [N]o`).
Secrets live in an encrypted vault whose master key is in the OS keyring; they never reach agent commands, the
databases, git or logs. Language: talk to the Lead/Solo in any language; everything below the Lead is English (ADR-40).

## Layout

| Crate | Role |
|---|---|
| `agenium-core` | Domain types, forms, protocol tool catalogue, state machine, dependency rule (pure) |
| `agenium-config` | Root resolution, `config.toml`, setup wizard backend, system requirements |
| `agenium-store` | SQLite: `core.db` (state + event log), per-agent memory DBs (vector + FTS5), migrations, backups |
| `agenium-bus` | Realtime events with exactly-once resync from the log |
| `agenium-secrets` | Vault + keyring, redaction, secret scanning |
| `agenium-vcs` | git CLI: worktrees, commits, merge queue, remote test |
| `agenium-sandbox` | Command execution: fail-closed sandbox detection, clean env, process-tree kill |
| `agenium-llm` | Provider trait, Anthropic / Gemini / OpenAI-compatible adapters, retry, pricing |
| `agenium-tools` | Work tools (files, patch, grep/glob, shell, fetch, todo) |
| `agenium-memory` | Embedder trait, memory retrieval for work orders |
| `agenium-backend` | The agent loop (work order, model turns, tool execution, limits) |
| `agenium-orchestrator` | App service, intent engine, git effects + reconcile, scheduler, agent runtime |
| `agenium-eval` | Golden tasks and score table |
| `agenium-cli` | The `agenium` command |

License: Apache-2.0 (`LICENSE`, `NOTICE`).
