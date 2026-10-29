//! MCP tool calls the user approves first (ADR-143). A shell command waits for the user by the command policy
//! (ADR-64); an MCP tool acts outside the sandbox, on the server's side (a refund, a deleted repository, a message
//! sent), so a call waits by what the server says of the tool: under `[sandbox] mcp_tools` (or the server's own
//! `approval`), the tools it marks destructive by default, or every tool it does not mark read-only. A call allowed
//! always is kept on the project's allowlist as `mcp:<server>/<tool>`.

use agenium_tools::{GateDecision, ToolAsk};

use super::bridge::{Approval, RunGate};

/// Characters of the arguments shown with the call.
const ARGUMENTS: usize = 600;

/// The allowlist entry of a tool allowed always.
fn entry(ask: &ToolAsk) -> String {
    format!("mcp:{}/{}", ask.server, ask.tool)
}

/// The call in one line, as a user reads it: `MCP github/delete_repo {"repo":"x"}`.
pub(super) fn line(ask: &ToolAsk) -> String {
    let mut args = ask.arguments.to_string();
    if args.chars().count() > ARGUMENTS {
        args = args.chars().take(ARGUMENTS).collect::<String>() + "…";
    }
    format!("MCP {}/{} {args}", ask.server, ask.tool)
}

pub(super) async fn check(gate: &RunGate, ask: &ToolAsk) -> GateDecision {
    let cfg = gate.app.config();
    let policy = cfg
        .mcp_server(&ask.server)
        .and_then(|s| s.approval)
        .unwrap_or(cfg.sandbox.mcp_tools);
    if !policy.asks(ask.read_only, ask.destructive) {
        return GateDecision::Allow;
    }
    let list = match gate.app.project(gate.project) {
        Ok(p) => crate::engine::allowlist(&gate.app, &p),
        Err(e) => return GateDecision::Deny(format!("cannot read the project allowlist: {e}")),
    };
    let entry = entry(ask);
    if list.contains(&entry) {
        return GateDecision::Allow;
    }
    match gate
        .approver
        .approve_tool(gate.project, gate.ticket.as_ref(), &gate.agent, ask)
        .await
    {
        Approval::Allow => GateDecision::Allow,
        Approval::AllowAlways => {
            let project = gate.project;
            let r = gate.app.store().commit(None, "user", |w| {
                agenium_store::core::projects::add_allowlist(w.tx, project, &[entry])
            });
            if let Err(e) = r {
                tracing::warn!(error = %e, "could not save always-allow entry");
            }
            GateDecision::Allow
        }
        Approval::Deny(reason) => GateDecision::Deny(format!(
            "the user did not allow {}/{}: {reason}. Do not call it again for this; say in your report what it \
             would have done",
            ask.server, ask.tool
        )),
    }
}
