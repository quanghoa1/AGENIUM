# ADR-84: Network posture — containment is the boundary; one audited `fetch` for every agent

Status: Accepted (2026-10-25; owner: "the agent must work comfortably, but we cannot let it or the system be attacked
by malware, prompt injection, etc.; unify URL reads through our own fetch"). Builds on [ADR-83](ADR-83-no-egress-proxy.md)
(no egress proxy).

## Context

Before this ADR the three agent kinds behaved inconsistently, and one was broken:

| Agent kind | Shell network | Read a URL |
| --- | --- | --- |
| API / protocol (Agenium's own loop) | **off** (`SandboxPolicy::workspace` → `network:false`) | our `fetch` tool (allowlist, runs in the Agenium process) |
| codex (`Control::Sandbox`) | **on** (`run.rs` forces `network = true`) | codex native web (its config disables web search) |
| claude / agy / opencode (`Control::Hook`) | host-run, gated by command checks | native web **denied** — and no `fetch` offered → **no URL reachable at all** |

So an API agent could not `npm install` (no shell network), and a vendor-CLI agent could not read any URL.

## Decision

**The security boundary is containment and data discipline, not network blocking.** Concretely:

1. **Reads go through one audited path — our `fetch` — for every agent.** A vendor CLI's native web stays off; from
   Agenium it now receives the `fetch` tool through the same `agenium` MCP server it already uses for `submit_report`.
   `fetch` runs in the Agenium process (never inside the sandbox, so it is never cut), enforces the project domain
   **allowlist**, logs the call, and marks the result untrusted. This is the one place a domain allowlist is kept —
   cheaply, at the app layer, not as network infrastructure (no proxy; consistent with ADR-83). The hook drivers'
   web-deny messages now point the agent to `fetch`.
2. **Sandboxed shell commands may reach the network by default** (`[sandbox].network`, default `true`), only inside a
   working sandbox backend (fail-closed: no backend → no network). The sandbox contains the blast radius — writes stay
   in the worktree, the host home is not mounted, the environment is cleared of secrets, and it dies after the run — so
   an injected command cannot read host secrets, and the repo code it could exfiltrate is already held by the model.
   Letting the agent install dependencies is worth more than blocking an egress channel the model already has. A strict
   project sets `[sandbox].network = false`; reads then still work through the `fetch` allowlist.
3. **An in-sandbox vendor CLI (codex) keeps container network on** regardless of `[sandbox].network`, because its model
   API runs inside the same container. `[sandbox].network` governs the in-sandbox *command* path (API agents); it does
   not touch a `Control::Hook` CLI's host-run commands, which the command gate governs.

### Why not just block egress

The things that actually stop an attack are already in place and are what we rely on: filesystem containment, a
cleared environment (no secrets in the sandbox), ephemeral per-run homes, and treating all fetched/MCP content as
untrusted data. Blocking shell egress does not reliably stop exfiltration (the model already receives the code; DNS and
allowed hosts remain), and the only thorough egress control — a domain-filtering proxy — was rejected as over-control
(ADR-83). Hook-mode CLIs run on the host and are therefore only partly contained; prefer them for trusted use, and the
command gate plus the unified `fetch` are their controls.

## Consequences

- `foundation/config`: `SandboxSection.network: bool` (default `true`); manual `Default`.
- `orchestration/orchestrator/runtime`: a CLI run is offered `agenium_tools::fetch_tool()` (plus the agent's MCP
  tools) instead of nothing; the work-run command policy takes `network` from config; the manifest states the real
  network state.
- `agent/tools`: `fetch_tool()` helper.
- `agent/cli-backend`: claude and agy web-deny messages point to the `agenium` `fetch` tool; `stream_run` test updated.
- Tests: config default asserts `sandbox.network`; `stream_run` asserts the web tool is denied toward `fetch`.
- Not done: per-domain command egress (would need the proxy ADR-83 rejected); moving hook-mode CLIs in-sandbox (blocked
  by their host login model).

## Addendum (2026-10-27): host loopback on Windows

With `network = true`, a command in the Windows sandbox reaches services on the host's `127.0.0.1` (a review found it;
tested: the sandbox cannot tell the worker's own loopback from the host's, so denying loopback also breaks the dev
servers and test suites a worker starts for itself). The owner's decision: keep it, as Codex's sandbox does with its
network on; no extra switch. The effort goes into the orchestration instead (agents doing the plan fully and right).
The app's own window is protected separately: it refuses any navigation away from the app (`nav-guard` in the
desktop), after a worker's verification script drove the app's DevTools port.
