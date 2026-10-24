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
