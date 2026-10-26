<p align="center">
  <img src="docs/brand/agenium-logo-banner-dark.svg" alt="AGENIUM — Ticket-Driven Multi-Agent Orchestrator" width="100%" />
</p>

# Agenium

Rust application that orchestrates AI agents — a Solo agent, or a Lead with Workers — through a built-in ticket system.
Every change of state goes through typed, validated tool calls; work happens in a git worktree per ticket and reaches
`main` only through review and a sequential merge queue.

**Status:** milestones M0 (backend), M1 (Solo agent) and most of M1.5 (skills, MCP, local embedding model + memory
pipeline, agent export/import/delete) are implemented and tested; the desktop UI is not part of this
phase — the headless CLI `agenium` is the front-end. Details and the honest NOT RUN list: [`docs/PROGRESS.md`](docs/PROGRESS.md).
This is a fresh implementation of the design in [`docs/design/`](docs/design) (priority on conflict: `DECISIONS.md`);
its own decisions are in [`docs/adr/ADR-52-fresh-implementation.md`](docs/adr/ADR-52-fresh-implementation.md).

## Build and test

Requirements: Rust stable (>= 1.89), a C/C++ compiler (bundled SQLite, sqlite-vec, zstd), git >= 2.30.
On Windows this repository is developed with the GNU toolchain and MSYS2 gcc; `mxc-sdk`'s build script needs an
`rc.exe`, which `.cargo/config.toml` maps to MinGW `windres` (`devtools/scripts/rc-windres.cmd`).

```sh
cargo build --workspace
.\devtools\scripts\ci-local.ps1        # Windows: fmt, clippy -D warnings, tests, schema check, scripted golden tasks
devtools/scripts/ci-local.sh           # Linux/macOS
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
agenium models download                              # local embedding model for agent memory (~130 MB, sha256-checked)
agenium skill import ./my-skill --agent ada          # SKILL.md folder, .zip or https URL
agenium mcp import mcp.json                          # {"mcpServers": {...}}; literal secrets move to the vault
agenium agent mcp ada --server github --cred TOKEN=gh-dev --tools create_issue
agenium agent export ada ada.agent.jsonl.zst         # import: agenium agent import <file> --project <p>
```

Safety model in this phase: on Windows builds where Microsoft's MXC process containers work, every agent command runs
in a container (writes only in the ticket worktree, reads limited to toolchains, no network; PowerShell as the shell —
ADR-53, `agenium doctor` shows it). Without a working sandbox backend YOLO mode is refused and **every shell command an
agent wants to run is shown to you for approval** (`[y]es / [a]lways / [N]o`). Linux/macOS backends are milestone M2.
Secrets live in an encrypted vault whose master key is in the OS keyring; they never reach agent commands, the
databases, git or logs. Language: talk to the Lead/Solo in any language; everything below the Lead is English (ADR-40).

## Layout (grouped by function)

```
apps/            what users run
  cli/             the `agenium` command (headless front-end; the desktop UI comes later)
orchestration/   coordinating agents and work
  orchestrator/    app service, intent engine, scheduler, agent runtime, git effects + reconcile, export/import
agent/           what an agent is made of
  loop/            the agent loop: work order, model turns, tool execution, limits   (package agenium-backend)
  llm/             providers: Anthropic, Gemini, OpenAI-compatible (OpenCode, DeepSeek, ...), retry, pricing
  tools/           work tools: files, patch, grep/glob, shell, fetch, todo
  memory/          local embedding model, retrieval (vector + BM25 + graph), memory pipeline
  skills/          SKILL.md folders: validation, role/agent resolution, import
  mcp/             MCP client hub (stdio, Streamable HTTP), credential binding
security/        keeping agents contained
  sandbox/         OS sandbox backends (Windows MXC, Linux bubblewrap, macOS Seatbelt), clean env, tree kill
    src/codex_derived/   the only code adapted from OpenAI Codex (see vendor/codex-sandbox)
  secrets/         encrypted vault + OS keyring, redaction, secret scanning
foundation/      shared building blocks
  core/            domain types, forms, protocol tools, ticket state machine, dependency rule (pure)
  config/          root resolution, config.toml, setup backend, system requirements
  store/           SQLite: core.db (state + event log), per-agent memory + graph, migrations, backups
  bus/             realtime events with exactly-once resync from the log
  vcs/             git CLI: worktrees, commits, merge queue
quality/         evaluation
  eval/            golden tasks, runner, score table; results/ holds real-model runs
devtools/        developer tooling
  xtask/           `cargo xtask schemas | vendor-check | ci`
  scripts/         local CI (ci-local.ps1/.sh), Codex sandbox sync, rc.exe shim
vendor/          third-party sources kept byte-for-byte (Codex sandbox, for maintenance)
schemas/         generated JSON Schemas of protocol tools + SQL migrations
docs/            design pack, ADRs, progress, spike reports, references
```

Each folder with a `Cargo.toml` is one crate of the Cargo workspace; package names keep the `agenium-` prefix
(`cargo test -p agenium-memory`).

License: Apache-2.0 (`LICENSE`, `NOTICE`).
