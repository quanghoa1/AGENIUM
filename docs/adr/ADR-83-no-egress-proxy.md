# ADR-83: No egress proxy — the codex-derived sandbox is the containment boundary, agents run YOLO

Status: Accepted (2026-10-25; owner: "applying a proxy controls the agent too much — we're building a multi-agent
orchestrator; the other concerns are already handled by codex's sandbox, and we run agents in YOLO mode").

## Context

The roadmap carried an "egress proxy" item: route every agent command's outbound network through a proxy that
enforces a domain **allowlist** (only npm/pypi/crates/github, etc.). That is a second containment layer on top of the
sandbox Agenium already vendors from codex (`security/sandbox` + `vendor/codex-sandbox`: read-only roots, a writable
worktree, a cleared environment, seccomp, and network on/off via a namespace/Seatbelt profile).

## Decision

**Drop the egress proxy.** It over-controls the agent for no clear gain in a multi-agent orchestrator:

- Containment is the **codex-derived sandbox** alone. Network is a single boolean (`SandboxPolicy.network` →
  Allow/Deny the whole namespace), not a proxied allowlist. When a task needs the network, it is turned on; otherwise
  it is off. There is no per-domain proxy, no TLS interception.
- Backends that bring their own sandbox (codex, etc.) keep using it; Agenium does not wrap a proxy around them.
- Agents run **YOLO** (`ApprovalPolicy::Never`/`Sandboxed`): commands run without asking inside the sandbox. The
  sandbox is the boundary, not an approval prompt or a proxy allowlist.

## Consequences

- The "egress proxy" item is removed from the roadmap (PROGRESS "Next steps"). No proxy code, config, or allowlist is
  added.
- Network policy stays the existing on/off boolean; the `fetch` tool keeps its own domain allowlist (a tool-level
  capability, unrelated to shell-command egress).
- If a future need for per-domain command egress ever appears, it is a new decision, not a pending task.
