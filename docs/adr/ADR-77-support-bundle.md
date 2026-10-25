# ADR-77: Support bundle

Status: Accepted (2026-10-25, M5).

## Decision

`agenium support [--out DIR]` and the desktop Settings button write a folder `agenium-support-<time>/` to a location
the user chooses, holding diagnostics safe to share: `versions.txt` (Agenium, OS/arch, core/agent schema versions,
detected CLIs with login state, sandbox backend, providers by id/kind), `config.toml` (the configuration — secrets are
stored by reference, so only their names appear), `stats.txt` (project, agent, ticket-by-status and run counts), and
`logs/` (the tail of the five most recent log files with secret values masked by the redactor). A README notes it is
diagnostics only and to review before sharing. No secret value, memory content, or deliverable is included.

## Consequences

- `App::support_bundle(out)`; CLI `support`; desktop IPC `support_bundle` and a Settings button.
- Test: `team.rs::support_bundle_has_diagnostics_without_secrets`.
