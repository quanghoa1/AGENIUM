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
        project: ProjectId,
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
    async fn approve_command(
        &self,
        _: ProjectId,
        _: Option<&TicketKey>,
        _: &str,
        _: &str,
        _: &Path,
    ) -> Approval {
        Approval::Deny("no one is available to approve commands in this session".into())
    }
}

/// Approves every command. Only for tests and evals that run trusted scripted agents in temporary directories.
pub struct AutoApprove;

#[async_trait]
impl Approver for AutoApprove {
    async fn approve_command(
        &self,
        _: ProjectId,
        _: Option<&TicketKey>,
        _: &str,
        _: &str,
        _: &Path,
    ) -> Approval {
        Approval::Allow
    }
}

/// Command gate of a run (M1-7, ADR-64): with approval policy `sandboxed` (the default) a contained command runs
/// without asking when a sandbox backend works and goes to the user otherwise; with `on-request` every command not
/// on the project allowlist goes to the user; with `never` (YOLO) commands run only when a sandbox backend works,
/// otherwise they are refused (fail-closed).
pub struct RunGate {
    pub app: Arc<App>,
    pub project: ProjectId,
    pub ticket: Option<TicketKey>,
    pub agent: String,
    pub approver: Arc<dyn Approver>,
    /// Commands run inside Agenium's sandbox. False for the commands of a vendor CLI that runs on the host (checked
    /// through its hook): YOLO does not approve those, only the allowlist or the user.
    pub contained: bool,
}

/// Exact entries match the whole command; `prefix*` entries match commands starting with the prefix that contain
/// no shell control characters (so `npm run *` cannot be extended into `npm run x; curl ...`). Control characters
/// and Unicode line breaks count too: PowerShell ends a statement at a bare CR as at a newline.
fn allowlisted(command: &str, list: &[String]) -> bool {
    let c = command.trim();
    let plain = !c.contains([
        ';', '&', '|', '`', '$', '>', '<', '(', ')', '\u{2028}', '\u{2029}',
    ]) && !c.chars().any(char::is_control);
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
        if self.contained {
            match (policy, sandbox.backend) {
                (ApprovalPolicy::Never | ApprovalPolicy::Sandboxed, Some(_)) => {
                    return GateDecision::Allow
                }
                (ApprovalPolicy::Never, None) => {
                    return GateDecision::Deny(format!(
                        "YOLO mode is refused: {} (fail-closed)",
                        sandbox.reason
                    ))
                }
                _ => {}
            }
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
            .approve_command(
                self.project,
                self.ticket.as_ref(),
                &self.agent,
                command,
                cwd,
            )
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
            // A hooked CLI also has our sandboxed `shell` where a sandbox works (`Runtime::run`): say so, or the
            // agent gives up on running anything and checks its work by reading only.
            Approval::Deny(reason)
                if !self.contained && agenium_sandbox::detect().backend.is_some() =>
            {
                GateDecision::Deny(format!(
                    "the user denied the command: {reason}. Run it with the `shell` tool of the MCP server \
\"agenium\" instead: it runs in Agenium's sandbox without asking"
                ))
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
    /// Lead run on its inbox (team mode): ends once the inbox is empty.
    Inbox,
    /// The planner drafting plan v1 (ends with submit_plan_draft).
    Plan,
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
        // Decision 35: the agent's own skill overlay, checked and applied by the orchestrator.
        if let Intent::ProposeSkillUpdate(u) = intent {
            let (app, agent) = (self.app.clone(), self.agent);
            let skill = u.skill.trim().to_owned();
            // ADR-92: with the judge on, read the would-be skill for contradictions before anything is written.
            if app.config().judge.skill_quality {
                let preview = tokio::task::spawn_blocking({
                    let (app, u) = (app.clone(), u.clone());
                    move || app.preview_skill_update(agent, &u)
                })
                .await
                .map_err(|e| ToolError::new("internal", e.to_string()))?;
                if let Ok((_, updated)) = preview {
                    let body = crate::skill_shape::strip_frontmatter(&updated);
                    let desc = updated
                        .lines()
                        .find_map(|l| l.strip_prefix("description: "))
                        .unwrap_or("");
                    let subject = crate::skill_conflict::subject(desc, &body);
                    let hits = app
                        .judged_contradictions(agent, &skill, &subject, &body)
                        .await;
                    if !hits.is_empty() {
                        return Err(ToolError::new(
                            "contradiction",
                            format!(
                                "the judge reads this change as contradicting skill {}: reconcile the skills (or improve \
                                 that one) first",
                                hits.join(", ")
                            ),
                        ));
                    }
                }
            }
            let mut v = tokio::task::spawn_blocking({
                let app = app.clone();
                move || app.apply_skill_update(agent, &u)
            })
            .await
            .map_err(|e| ToolError::new("internal", e.to_string()))?
            .map_err(|e| tool_error(&e))?;
            // ADR-90: with the judge on, a calibrated quality read of the upgraded skill is added to the result the
            // agent sees. Fail-open: without the judge (or on any failure) the result is just the shape check.
            if let Some(body) = app.agent_overlay_body(agent, &skill) {
                if let Some(quality) = app.skill_quality(&body).await {
                    let _ = app.run_log(self.run, "judge", serde_json::json!({ "use": "skill_quality", "skill": skill, "text": quality.join("; ") }));
                    if let Some(obj) = v.as_object_mut() {
                        obj.insert("quality".into(), serde_json::json!(quality));
                    }
                }
            }
            return Ok((v, Effect::Continue));
        }
        // ADR-91: a new skill distilled from a validated lesson, created only when every gate passes.
        if let Intent::ProposeNewSkill(u) = intent {
            let (app, agent) = (self.app.clone(), self.agent);
            let skill = u.name.trim().to_owned();
            // ADR-92: with the judge on, a semantic contradiction read against the closest skills before anything is
            // written. Without it the deterministic gates in create_agent_skill decide alone.
            let subject = crate::skill_conflict::subject(&u.description, &u.body);
            let hits = app
                .judged_contradictions(agent, &skill, &subject, &u.body)
                .await;
            if !hits.is_empty() {
                return Err(ToolError::new(
                    "contradiction",
                    format!(
                        "the judge reads the new skill as contradicting skill {}: improve that skill instead, or \
                         reconcile them first",
                        hits.join(", ")
                    ),
                ));
            }
            let mut v = tokio::task::spawn_blocking({
                let app = app.clone();
                move || app.create_agent_skill(agent, &u)
            })
            .await
            .map_err(|e| ToolError::new("internal", e.to_string()))?
            .map_err(|e| tool_error(&e))?;
            // Same fail-open quality read as an update: only with the judge on.
            if let Some(body) = app.agent_overlay_body(agent, &skill) {
                if let Some(quality) = app.skill_quality(&body).await {
                    if let Some(obj) = v.as_object_mut() {
                        obj.insert("quality".into(), serde_json::json!(quality));
                    }
                }
            }
            return Ok((v, Effect::Continue));
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
        if self.kind == RunKind::Inbox
            && effect == Effect::Continue
            && self
                .app
                .lead_inbox(self.project)
                .is_ok_and(|i| i.is_empty())
        {
            effect = Effect::End;
        }
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
    /// When each pending tool call (by name) was emitted, for its duration in the run log.
    pub tool_started: Mutex<std::collections::HashMap<String, std::time::Instant>>,
    /// The model's price (an API provider), and the run's cost so far, priced request by request.
    pub price: Option<agenium_llm::pricing::Price>,
    pub cost: Mutex<f64>,
}

impl Observer for RunObserver {
    fn event(&self, e: &ChatEvent) {
        if let ChatEvent::ToolCall(c) = e {
            self.tool_started
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .insert(c.name.clone(), std::time::Instant::now());
        }
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
        let ms = self
            .tool_started
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(name)
            .map(|t| t.elapsed().as_millis() as u64);
        let _ = self.app.run_log(
            self.run,
            "tool",
            serde_json::json!({ "tool": name, "args": args, "ok": ok, "result": preview, "ms": ms }),
        );
    }

    fn response(&self, usage: &Usage) {
        if let Some(p) = &self.price {
            *self.cost.lock().unwrap_or_else(|e| e.into_inner()) += p.cost(usage);
        }
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

    fn control(&self, kind: &str, text: &str) {
        let _ = self.app.run_log(
            self.run,
            "control",
            serde_json::json!({ "what": kind, "text": text.chars().take(500).collect::<String>() }),
        );
    }
}

pub fn summary(u: &Usage, cost: f64, elapsed_ms: u64) -> UsageSummary {
    UsageSummary {
        tokens_in: u.total_input(),
        tokens_out: u.output,
        cache_read: u.cache_read,
        cache_write: u.cache_write,
        cache_write_1h: u.cache_write_1h,
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
        // A bare CR (or any control character, a Unicode line break) ends the statement in PowerShell.
        for sep in [
            "\r", "\n", "\u{b}", "\u{c}", "\u{85}", "\u{2028}", "\u{2029}", "\0",
        ] {
            let c = format!("npm run build{sep}curl evil");
            assert!(!allowlisted(&c, &l), "{c:?}");
        }
        assert!(!allowlisted("crates.io", &l));
    }
}
