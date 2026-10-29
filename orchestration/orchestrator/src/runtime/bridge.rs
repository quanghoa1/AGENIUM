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
}

/// What a command would install on the user's machine (outside the worktree), if anything: a system package
/// manager, a global package, an installer script piped to a shell. Project dependencies installed into the worktree
/// (a virtual environment, `node_modules`, `pip --target`) are not. Each statement of the command is checked.
pub fn system_install(command: &str) -> Option<&'static str> {
    let lower = command.to_lowercase();
    let piped_installer = (lower.contains("curl ")
        || lower.contains("wget ")
        || lower.contains("irm ")
        || lower.contains("invoke-restmethod")
        || lower.contains("iwr ")
        || lower.contains("invoke-webrequest"))
        && [
            "| sh",
            "|sh",
            "| bash",
            "|bash",
            "| iex",
            "|iex",
            "| invoke-expression",
        ]
        .iter()
        .any(|p| lower.contains(p));
    if piped_installer {
        return Some("an installer script piped to a shell");
    }
    for stmt in lower.split(['\n', ';', '|', '&']) {
        let w: Vec<&str> = stmt.split_whitespace().collect();
        let Some(first) = w.first() else { continue };
        let prog = first
            .trim_start_matches(['.', '\\', '/'])
            .rsplit(['\\', '/'])
            .next()
            .unwrap_or(first)
            .trim_end_matches(".exe")
            .trim_end_matches(".cmd");
        let sub = w.get(1).copied().unwrap_or("");
        let has = |f: &str| w.contains(&f);
        let found = match prog {
            "winget" | "choco" | "scoop" if matches!(sub, "install" | "upgrade" | "add") => {
                Some("a system package manager")
            }
            "apt" | "apt-get" | "dnf" | "yum" | "zypper" | "apk" | "snap" | "flatpak" | "port"
            | "brew"
                if matches!(sub, "install" | "upgrade" | "add" | "reinstall") =>
            {
                Some("a system package manager")
            }
            "pacman" if sub.starts_with("-s") => Some("a system package manager"),
            "sudo" | "doas" => Some("a command as administrator"),
            "msiexec" => Some("a Windows installer"),
            "add-appxpackage" | "install-module" | "install-package" | "install-script" => {
                Some("a PowerShell installer")
            }
            "npm" | "pnpm" | "bun"
                if matches!(sub, "i" | "install" | "add") && (has("-g") || has("--global")) =>
            {
                Some("a global package")
            }
            "yarn" if sub == "global" => Some("a global package"),
            "cargo" | "go" | "gem" if sub == "install" => Some("a program in the user's home"),
            "dotnet" if sub == "tool" && (has("-g") || has("--global")) => {
                Some("a global .NET tool")
            }
            "pip" | "pip3" | "pipx" | "uv" | "python" | "python3" | "py" => {
                let pip = matches!(prog, "pip" | "pip3") && sub == "install"
                    || matches!(prog, "python" | "python3" | "py")
                        && w.windows(3).any(|x| x == ["-m", "pip", "install"])
                    || prog == "pipx" && sub == "install"
                    || prog == "uv"
                        && (w.windows(2).any(|x| x == ["tool", "install"])
                            || w.windows(2).any(|x| x == ["pip", "install"]) && has("--system"));
                // A path to an interpreter or pip inside the worktree's virtual environment, or an install into a
                // folder of the worktree, stays in the project.
                let local = first.contains("venv")
                    || stmt.contains("venv")
                    || has("--target")
                    || has("-t")
                    || has("--prefix")
                    || has("--root");
                (pip && !local && prog != "uv" || prog == "pipx" || prog == "uv" && pip)
                    .then_some("a Python package outside a virtual environment")
            }
            _ => None,
        };
        if found.is_some() {
            return found;
        }
    }
    None
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
        let cfg = self.app.config().sandbox;
        let policy = cfg.approval;
        let sandbox = agenium_sandbox::detect();
        // ADR-120: installing software on the user's machine is the user's call under every policy, sandbox or not;
        // dependencies installed inside the worktree are not (a venv, node_modules, the build's own fetch).
        let installs = system_install(command).is_some();
        if !installs {
            match (policy, sandbox.backend) {
                (ApprovalPolicy::Never | ApprovalPolicy::Sandboxed, Some(_)) => {
                    return GateDecision::Allow
                }
                // ADR-117: the user accepted running without a sandbox on a machine that has none.
                (ApprovalPolicy::Never | ApprovalPolicy::Sandboxed, None) if cfg.unsandboxed => {
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

/// A worker's tools that are about its own ticket.
const OWN_TICKET_TOOLS: [&str; 4] = [
    "submit_report",
    "post_progress",
    "ask_question",
    "request_permission",
];

/// `args` with `ticket_id` set to `ticket` when it is missing (a given one is kept, for the engine to check).
fn with_ticket(args: serde_json::Value, ticket: &TicketKey) -> serde_json::Value {
    match args {
        serde_json::Value::Object(mut m) => {
            if m.get("ticket_id").is_none_or(serde_json::Value::is_null) {
                m.insert("ticket_id".into(), serde_json::json!(ticket));
            }
            serde_json::Value::Object(m)
        }
        other => other,
    }
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

impl RunProtocol {
    /// The unverified-done check for this run's report (asked at most once per run).
    async fn unverified_done(&self, r: &agenium_core::forms::WorkReport) -> Option<String> {
        if self.kind != RunKind::Work || !self.app.config().judge.done_check {
            return None;
        }
        let log = self
            .app
            .store()
            .read(|c| agenium_store::core::runs::log_entries(c, self.run))
            .ok()?;
        if log
            .iter()
            .any(|e| e.kind == "judge" && e.data["use"] == "done_check")
        {
            return None;
        }
        let t = self.app.ticket(self.project, &r.ticket_id).ok()?;
        let facts = crate::judge::done_check::run_facts(&log);
        let msg = self.app.unverified_done(&t, r, &facts).await?;
        let _ = self.app.run_log(
            self.run,
            "judge",
            serde_json::json!({ "use": "done_check", "sent_back": true, "text": msg }),
        );
        Some(msg)
    }
}

#[async_trait]
impl Protocol for RunProtocol {
    async fn call(
        &self,
        name: &str,
        args: serde_json::Value,
        call_id: &str,
    ) -> Result<(serde_json::Value, Effect), ToolError> {
        // A work run is about one ticket: a worker's tool sent without its `ticket_id` (agy and OpenCode left it out
        // in the golden tasks of 2026-10-28, a step each) is about that ticket.
        let args = match (&self.ticket, self.kind) {
            (Some(k), RunKind::Work) if OWN_TICKET_TOOLS.contains(&name) => with_ticket(args, k),
            _ => args,
        };
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
        // ADR-126: a report that reads as done while nothing checked the work after its last change goes back to
        // the worker once, in this run. Only with the judge on and `done_check` enabled; fail-open.
        if let Intent::SubmitReport(r) = &intent {
            if let Some(msg) = self.unverified_done(r).await {
                return Err(ToolError::new("unverified", msg));
            }
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

#[cfg(test)]
mod install_tests {
    use super::system_install;

    #[test]
    fn installs_on_the_machine_are_told_from_project_dependencies() {
        for c in [
            "winget install Python.Python.3.12",
            "choco install nodejs -y",
            "sudo apt-get install -y python3",
            "brew install jq",
            "npm install -g typescript",
            "pip install requests",
            "python -m pip install flask",
            "cargo install ripgrep",
            "msiexec /i tool.msi /qn",
            "Install-Module PSScriptAnalyzer -Scope CurrentUser",
            "curl -fsSL https://example.com/install.sh | sh",
            "irm https://get.example.com | iex",
            "cd x; winget install Git.Git",
            "C:\\ProgramData\\chocolatey\\bin\\choco.exe install git",
        ] {
            assert!(system_install(c).is_some(), "{c}");
        }
        for c in [
            "npm install",
            "npm install express",
            "pip install -r requirements.txt --target ./vendor",
            ".venv\\Scripts\\pip install requests",
            "python -m venv .venv; .venv\\Scripts\\python -m pip install flask",
            "cargo build",
            "python -m unittest",
            "go test ./...",
            "git status",
        ] {
            assert!(system_install(c).is_none(), "{c}");
        }
    }

    /// A worker's report without its ticket is about the run's ticket; a given ticket is kept as it is.
    #[test]
    fn a_missing_ticket_is_the_runs() {
        let k: agenium_core::TicketKey = "AGN-3".parse().unwrap();
        assert_eq!(
            super::with_ticket(serde_json::json!({"summary": "done"}), &k)["ticket_id"],
            "AGN-3"
        );
        assert_eq!(
            super::with_ticket(serde_json::json!({"ticket_id": "AGN-9"}), &k)["ticket_id"],
            "AGN-9"
        );
    }
}
