# ADR index

ADR-1 … ADR-38 are summarised in [`../design/DECISIONS.md`](../design/DECISIONS.md) (highest priority on conflict).
ADR-39 … ADR-51 were written during the M0 trial; they are decisions and results, kept as written.
ADR-52 records this fresh implementation.

| ADR | Topic |
|---|---|
| [39](ADR-39-windows-deferred.md) | First release Linux + macOS; Windows deferred |
| [40](ADR-40-english-below-lead.md) | Any language with the user, English from the Lead down |
| [41](ADR-41-sandbox-vendoring-and-layering.md) | Codex sandbox vendoring and wrapper crate |
| [42](ADR-42-linux-sandbox-bwrap-landlock.md) | Linux sandbox: bubblewrap / Landlock >= 3, fail-closed |
| [43](ADR-43-macos-seatbelt-design.md) | macOS Seatbelt profile v2 |
| [44](ADR-44-vcs-git-cli.md) | VCS through the git CLI, journal-first |
| [45](ADR-45-journal-and-event-sink-in-core.md) | Event sink in core, broadcast bus |
| [46](ADR-46-backup-and-migration-design.md) | Backups and migrations |
| [47](ADR-47-embedder-trait.md) | Embedder trait |
| [48](ADR-48-xtask-and-ipc-types.md) | xtask, IPC types |
| [49](ADR-49-sqlite-primary-store.md) | SQLite primary store |
| [50](ADR-50-vector-index-scale-and-retention.md) | Vector index, 100k ANN threshold, retention |
| [51](ADR-51-graph-memory-on-sqlite.md) | Graph memory on SQLite (Proposed) |
| [52](ADR-52-fresh-implementation.md) | Fresh implementation; Windows verification; deviations |
| [53](ADR-53-windows-sandbox-mxc.md) | Windows sandbox backend: MXC process containers via `mxc-sdk` (amends ADR-39) |
| [54](ADR-54-linux-sandbox-bubblewrap-mxc-sdk.md) | Linux sandbox backend: bubblewrap via `mxc-sdk` (S3 matrix not run yet) |
| [55](ADR-55-macos-sandbox-seatbelt-mxc-sdk.md) | macOS sandbox backend: Seatbelt via `mxc-sdk` (experimental, not run on a Mac yet) |
| [56](ADR-56-cli-backend-headless.md) | CliBackend: headless JSON stream in an Agenium-owned per-run home; agy driven |
| [57](ADR-57-team-mode-lead-inbox.md) | Team mode: Lead inbox (derived view), inbox runs per round, SLA, escalation flag |
| [58](ADR-58-team-templates-and-write-scope.md) | Team templates in TOML (4 built-in + user overrides); write scope = ticket scope AND role paths, checked at write and at submit |
| [59](ADR-59-planner-role.md) | Planner role: drafts plan v1 (submit_plan_draft), the Lead reviews and creates the plan; no subagents in team mode |
| [60](ADR-60-desktop-app.md) | Desktop app: Tauri 2 in its own workspace, thin IPC over the orchestrator, UI adapted from the AGENIUM-UX mockup |
| [61](ADR-61-agent-pool-and-custom-teams.md) | Agent pool (agents before teams), custom teams with a suggested Lead, system skill library with per-agent switches, providers from the UI |
| [62](ADR-62-mcp-keys-projects-live-chat.md) | Per-agent MCP keys (system key by default, Lead tool set_member_mcp_key, shown in the app, masked in logs), projects by folder name or repository, Live tab as a chat with a plan tab |
| [63](ADR-63-team-runs-until-done.md) | From the agreed spec to the end the team runs on its own (run state, team loop, stop/resume per project); the Lead approves plans of an agreed spec |
| [64](ADR-64-sandboxed-commands-roles-review.md) | Commands run in the sandbox without asking; no plan items for the Lead's role; workers get their requirements; skeptical review; ROLE.md sections and a specialist role library |
| [65](ADR-65-agent-names-audit-only.md) | Agent names are role names without the team, unique ignoring case; the Activity page folds into the audit trail |
| [66](ADR-66-model-prices-catalogue.md) | Model prices from the models.dev catalogue (cached, refreshed weekly, earlier runs priced again); FREE models marked; OpenCode free models need the OpenCode client |
| [67](ADR-67-vendor-usage-footer.md) | Vendor subscription usage in the footer (Claude from its run events, Codex app-server RPC, OpenCode Go endpoint; Antigravity listed without numbers); only honest channels |
