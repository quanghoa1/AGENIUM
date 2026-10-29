# ADR-143: MCP tools that act on the server's side ask the user first

Status: Accepted (2026-10-29). Owner: "làm 1,2,4,6 đi" (1: a tool that only reads runs; a destructive one asks the user
in Needs you, or the user sets it per server).

## Context

- A shell command waits for the user by the command policy (ADR-64): sandboxed it runs, else it is asked. An MCP tool
  never waited: it runs on the server's side, outside every sandbox, with the agent's key — a refund, a deleted
  repository, a message sent to a customer happen at once.
- A server says what each tool does in its annotations: `readOnlyHint` (it changes nothing) and `destructiveHint` (it
  may destroy or send). Many servers say nothing.
- Vendor CLIs reach MCP tools through the run's own tools (ADR-117, ADR-129), so one gate covers API and CLI agents.

## Decision

1. **What asks** (`agenium_config::McpApproval`, `[sandbox] mcp_tools`, or a server's own `approval` on its `[[mcp]]`):
   `destructive` (the default) — the tools the server marks destructive; `writes` — every tool it does not mark
   read-only (for a server that says nothing of its tools); `never`.
2. **The gate**: an MCP tool call goes through `CommandGate::check_tool` (default: allow) with the server, the tool,
   the arguments and the annotations (`agenium_tools::ToolAsk`). The runtime's gate (`runtime/mcp_gate.rs`) applies the
   setting; a call that asks goes to the user through `Approver::approve_tool` — by default the command approval with
   a line `MCP <server>/<tool> <arguments>`; the desktop shows it in Needs you with the tool, its arguments and whether
   it is destructive. Allowed always, the tool is kept on the project's allowlist as `mcp:<server>/<tool>`. Refused,
   the agent is told not to call it again for this and to say in its report what it would have done; the server is
   not called.
3. **Where it is set**: the MCP tab of Skills & MCP — the default for every server (each choice described), and a
   select on each server's row ("As the default", or its own); a server's test shows which tools it marks
   destructive. A server saved again from the form keeps its setting.

## Verified (2026-10-29)

- Tests: `mcp_approval::tests::what_waits_for_the_user`; runtime
  `a_destructive_mcp_tool_waits_for_the_user` (a destructive tool refused by the user is not called and the agent is
  told; a read-only tool runs without asking; the server set to never ask, it runs; allowed always for the project,
  it runs).
- Desktop UI smoke (`ui-tests/smoke.mjs`): the default set from the MCP tab is saved; no console errors.

## Consequences

- A server that marks nothing destructive asks nothing under the default; the owner sets `writes` on it.
- An agent waiting on an MCP approval waits as for a command: 30 minutes in the app, then refused.
