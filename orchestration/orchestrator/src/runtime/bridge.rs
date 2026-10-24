//! Adapters between the agent loop (agenium-backend) and the orchestrator: protocol calls become intents, shell
//! commands pass the approval gate, and everything the loop does is persisted (run log, conversation, heartbeat).

use std::path::Path;
use std::sync::{Arc, Mutex};

use agenium_backend::{Effect, Observer, Protocol};
use agenium_config::ApprovalPolicy;
use agenium_core::protocol::{IntentError, ToolGroup};
use agenium_core::*;
use agenium_llm::{ChatEvent, Message, Usage};
use agenium_store::AgentStore;
use agenium_tools::{CommandGate, GateDecision, ToolError};
use async_trait::async_trait;

use crate::app::App;
use crate::engine::{CallCtx, RunEffect};
use crate::OrchError;

/// How the user answers a command approval request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Approval {
    Allow,
    /// Allow and add the exact command to the project allowlist (only the user can grant this, decision 4).
    AllowAlways,
    Deny(String),
}

/// The user's approval channel (CLI prompt, UI dialog, or a policy in tests).
#[async_trait]
pub trait Approver: Send + Sync {
    async fn approve_command(
        &self,
        ticket: Option<&TicketKey>,
        agent: &str,
        command: &str,
        cwd: &Path,
    ) -> Approval;
}

/// Refuses every command (headless runs where nobody can approve).
pub struct DenyApprover;

#[async_trait]
impl Approver for DenyApprover {
    async fn approve_command(&self, _: Option<&TicketKey>, _: &str, _: &str, _: &Path) -> Approval {
        Approval::Deny("no one is available to approve commands in this session".into())
    }
}

/// Approves every command. Only for tests and evals that run trusted scripted agents in temporary directories.
pub struct AutoApprove;

#[async_trait]
impl Approver for AutoApprove {
    async fn approve_command(&self, _: Option<&TicketKey>, _: &str, _: &str, _: &Path) -> Approval {
        Approval::Allow
    }
}

/// Command gate of a run (M1-7): with approval policy `on-request` every command not on the project allowlist
/// goes to the user; with `never` (YOLO) commands run only when a sandbox backend works, otherwise they are refused
/// (fail-closed).
pub struct RunGate {
    pub app: Arc<App>,
    pub project: ProjectId,
    pub ticket: Option<TicketKey>,
    pub agent: String,
    pub approver: Arc<dyn Approver>,
}

/// Exact entries match the whole command; `prefix*` entries match commands starting with the prefix that contain
/// no shell control characters (so `npm run *` cannot be extended into `npm run x; curl ...`).
fn allowlisted(command: &str, list: &[String]) -> bool {
    let c = command.trim();
    let plain = !c.contains([';', '&', '|', '`', '$', '>', '<', '\n', '(', ')']);
    list.iter()
        .filter_map(|e| e.strip_prefix("cmd:"))
        .any(|e| match e.strip_suffix('*') {
            Some(prefix) => plain && c.starts_with(prefix),
            None => c == e,
        })
}

#[async_trait]
impl CommandGate for RunGate {
    async fn check(&self, command: &str, cwd: &Path) -> GateDecision {
        let policy = self.app.config().sandbox.approval;
        let sandbox = agenium_sandbox::detect();
        if policy == ApprovalPolicy::Never {
            return match sandbox.backend {
                Some(_) => GateDecision::Allow,
                None => GateDecision::Deny(format!(
                    "YOLO mode is refused: {} (fail-closed)",
                    sandbox.reason
                )),
            };
        }
        let list = match self.app.project(self.project) {
            Ok(p) => crate::engine::allowlist(&self.app, &p),
            Err(e) => return GateDecision::Deny(format!("cannot read the project allowlist: {e}")),
        };
        if allowlisted(command, &list) {
            return GateDecision::Allow;
        }
        match self
            .approver
            .approve_command(self.ticket.as_ref(), &self.agent, command, cwd)
            .await
        {
            Approval::Allow => GateDecision::Allow,
            Approval::AllowAlways => {
                let entry = format!("cmd:{}", command.trim());
                let project = self.project;
                let r = self.app.store().commit(None, "user", |w| {
                    agenium_store::core::projects::add_allowlist(w.tx, project, &[entry])
                });
                if let Err(e) = r {
                    tracing::warn!(error = %e, "could not save always-allow entry");
                }
                GateDecision::Allow
            }
            Approval::Deny(reason) => {
                GateDecision::Deny(format!("the user denied the command: {reason}"))
            }
        }
    }
}

/// Which way a run ends early, decided from the ticket state after a protocol call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunKind {
    Work,
    Review,
    Chat,
}

/// Routes protocol tool calls of one run to the intent engine.
pub struct RunProtocol {
    pub app: Arc<App>,
    pub project: ProjectId,
    pub agent: AgentId,
    pub role: RoleKind,
    pub solo: bool,
    pub run: RunId,
    pub kind: RunKind,
    pub ticket: Option<TicketKey>,
    /// Skills of this run's agent (`load_skill` is answered here; it changes no state).
    pub skills: Arc<agenium_skills::SkillSet>,
}

fn tool_error(e: &OrchError) -> ToolError {
    let te = e.to_tool_error();
    let prefix = format!("{}: ", te.code);
    let mut message = te
        .message
        .strip_prefix(&prefix)
        .map(str::to_owned)
        .unwrap_or(te.message);
    if let Some(d) = te.detail {
        message.push_str(&format!(" | detail: {d}"));
    }
    ToolError::new(&te.code, message)
}

#[async_trait]
impl Protocol for RunProtocol {
    async fn call(
        &self,
        name: &str,
        args: serde_json::Value,
        call_id: &str,
    ) -> Result<(serde_json::Value, Effect), ToolError> {
        let intent = Intent::parse(name, args, self.role, self.solo).map_err(|e| {
            let code = match &e {
                IntentError::UnknownTool { .. } => "unknown_tool",
                IntentError::NotAllowed { .. } => "tool_not_allowed",
                _ => "invalid_arguments",
            };
            let msg = match &e {
                IntentError::Fields { errors, .. } => format!(
                    "{e} | detail: {}",
                    serde_json::to_string(errors).unwrap_or_default()
                ),
                _ => e.to_string(),
            };
            ToolError::new(code, msg)
        })?;
        if let Intent::LoadSkill(l) = &intent {
            return match self.skills.get(&l.name) {
                Some(s) => Ok((
                    serde_json::Value::String(s.instructions()),
                    Effect::Continue,
                )),
                None => Err(ToolError::new(
                    "invalid",
                    format!(
                        "no skill named {:?} is installed for you; available: {}",
                        l.name,
                        self.skills.names().join(", ")
                    ),
                )),
            };
        }
        let tool = intent.tool();
        let actor = match (tool.group(), self.role) {
            (ToolGroup::Lead, _) => Actor::Lead(self.agent),
            (ToolGroup::Worker, _) => Actor::Worker(self.agent),
            (ToolGroup::Shared, RoleKind::Lead) => Actor::Lead(self.agent),
            (ToolGroup::Shared, RoleKind::Worker) => Actor::Worker(self.agent),
        };
        let ctx = CallCtx {
            project: self.project,
            actor,
            intent_id: Some(format!("{}:{call_id}", self.run)),
            run: Some(self.run),
        };
        let app = self.app.clone();
        // The engine runs git (merge queue) synchronously: keep it off the async workers.
        let out = tokio::task::spawn_blocking(move || app.apply(&ctx, intent))
            .await
            .map_err(|e| ToolError::new("internal", e.to_string()))?
            .map_err(|e| tool_error(&e))?;
        let mut effect = match out.run {
            RunEffect::Continue => Effect::Continue,
            RunEffect::End => Effect::End,
            RunEffect::Park => Effect::Park,
        };
        if self.kind == RunKind::Review && effect == Effect::Continue {
            if let Some(k) = &self.ticket {
                if let Ok(t) = self.app.ticket(self.project, k) {
                    let waiting_merge =
                        t.status == TicketStatus::Resolved && t.approved && t.has_code && !t.merged;
                    if t.status != TicketStatus::Resolved || waiting_merge {
                        effect = Effect::End;
                    }
                }
            }
        }
        Ok((out.result, effect))
    }
}

/// Live event callback of one run.
pub type LiveFn = Arc<dyn Fn(&ChatEvent) + Send + Sync>;

/// Persists what happens in a run. Conversation messages go to the agent's private database (session = run id)
/// so a continuation run resumes from them; tool calls and errors go to the structured run log (redacted).
pub struct RunObserver {
    pub app: Arc<App>,
    pub run: RunId,
    pub memory: Option<Arc<AgentStore>>,
    pub live: Option<LiveFn>,
    pub steps: Mutex<u32>,
}

impl Observer for RunObserver {
    fn event(&self, e: &ChatEvent) {
        if let Some(l) = &self.live {
            l(e);
        }
    }

    fn message(&self, m: &Message) {
        if let (Some(s), Ok(v)) = (&self.memory, serde_json::to_value(m)) {
            if let Err(e) = s.append_message(&self.run.to_string(), &v) {
                tracing::warn!(error = %e, "could not persist conversation message");
            }
        }
    }

    fn tool(&self, name: &str, args: &serde_json::Value, result: &Result<String, ToolError>) {
        let (ok, preview) = match result {
            Ok(t) => (true, t.chars().take(2000).collect::<String>()),
            Err(e) => (false, e.to_string()),
        };
        let _ = self.app.run_log(
            self.run,
            "tool",
            serde_json::json!({ "tool": name, "args": args, "ok": ok, "result": preview }),
        );
    }

    fn step(&self, step: u32, usage: &Usage) {
        *self.steps.lock().unwrap_or_else(|p| p.into_inner()) = step;
        let _ = self
            .app
            .run_progress(self.run, step, &summary(usage, 0.0, 0));
    }

    fn retry(&self, attempt: u32, delay_ms: u64, error: &str) {
        let _ = self.app.run_log(
            self.run,
            "retry",
            serde_json::json!({ "attempt": attempt, "delay_ms": delay_ms, "error": error }),
        );
    }
}

pub fn summary(u: &Usage, cost: f64, elapsed_ms: u64) -> UsageSummary {
    UsageSummary {
        tokens_in: u.total_input(),
        tokens_out: u.output,
        cache_read: u.cache_read,
        cache_write: u.cache_write,
        estimated_cost_usd: cost,
        elapsed_ms,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowlist_matching() {
        let l = vec![
            "cmd:cargo test".to_string(),
            "cmd:npm run *".to_string(),
            "crates.io".to_string(),
        ];
        assert!(allowlisted("cargo test", &l));
        assert!(allowlisted("  cargo test ", &l));
        assert!(!allowlisted("cargo test; rm -rf /", &l));
        assert!(allowlisted("npm run build", &l));
        assert!(!allowlisted("npm run build && curl evil", &l));
        assert!(!allowlisted("crates.io", &l));
    }
}
