//! An agent's own CLI as the model for Agenium's side calls (ADR-56 amended, 2026-11-03): the memory an agent keeps
//! is Agenium's, and the model that draws it is only the brain. A CLI agent's brain is the CLI it already works on, so
//! a user with no API key still gets lessons and facts — one read-only answer per call, no tools, an empty folder.

use std::path::PathBuf;
use std::sync::Arc;

use agenium_backend::{Effect, NoopObserver, Protocol, WorkOrder};
use agenium_cli_backend::{CliRun, Control, Driver, Limits};
use agenium_llm::{
    ChatEvent, ChatRequest, ChatResponse, LlmError, Message, ModelInfo, Provider, Role, StopReason,
};
use agenium_tools::{DenyAll, ToolCtx, ToolError};
use async_trait::async_trait;

/// A side call offers no protocol tool.
struct NoProtocol;

#[async_trait]
impl Protocol for NoProtocol {
    async fn call(
        &self,
        name: &str,
        _args: serde_json::Value,
        _call_id: &str,
    ) -> std::result::Result<(serde_json::Value, Effect), ToolError> {
        Err(ToolError::new(
            "no_tools",
            format!("{name} is not available here: answer in text"),
        ))
    }
}

/// A CLI's side turns at most (a memory answer is one short call), and how long it may last.
const STEPS: u32 = 4;
const TIMEOUT_MINS: u64 = 10;

/// A vendor CLI as an [`agenium_llm::Provider`], for the side calls of an agent that runs on it.
pub(crate) struct CliBrain {
    id: String,
    driver: &'static dyn Driver,
    program: PathBuf,
    cancel: agenium_backend::Cancel,
    uncontained: bool,
}

impl CliBrain {
    pub(crate) fn new(
        id: String,
        driver: &'static dyn Driver,
        program: PathBuf,
        cancel: agenium_backend::Cancel,
    ) -> Self {
        // As a side call (ADR-151): a CLI driven only inside a sandbox, with no sandbox backend here, runs uncontained;
        // `cli_ready_named` already refused it unless Settings allows that.
        let uncontained =
            driver.control() == Control::Sandbox && agenium_sandbox::detect().backend.is_none();
        Self {
            id,
            driver,
            program,
            cancel,
            uncontained,
        }
    }
}

#[async_trait]
impl Provider for CliBrain {
    fn id(&self) -> &str {
        &self.id
    }
    fn kind(&self) -> &'static str {
        "cli"
    }
    async fn list_models(&self) -> std::result::Result<Vec<ModelInfo>, LlmError> {
        Ok(Vec::new())
    }

    async fn chat(
        &self,
        req: &ChatRequest,
        _emit: &(dyn Fn(ChatEvent) + Send + Sync),
    ) -> std::result::Result<ChatResponse, LlmError> {
        let dir = std::env::temp_dir().join(format!("agenium-brain-{}", ulid::Ulid::new()));
        std::fs::create_dir_all(&dir).map_err(|e| LlmError::Config(e.to_string()))?;
        let ctx = ToolCtx::new(
            &dir,
            "brain",
            Arc::new(DenyAll("a memory call runs no command".into())),
        );
        let label = format!("{}:{}", self.id, req.model);
        let run = CliRun {
            driver: self.driver,
            program: self.program.clone(),
            model: req.model.clone(),
            effort: Some(req.effort.clone()),
            env: ctx.env.clone(),
            cwd: dir.clone(),
            label: label.clone(),
            protocol: &NoProtocol,
            protocol_specs: vec![],
            tools: vec![],
            ctx: &ctx,
            session: None,
            observer: &NoopObserver,
            cancel: self.cancel.child(),
            limits: Limits {
                max_steps: STEPS,
                timeout: std::time::Duration::from_secs(TIMEOUT_MINS * 60),
                ..Limits::default()
            },
            chat: true,
            nudge: None,
            subagents: false,
            read_only: true,
            uncontained: self.uncontained,
            on_limits: None,
            steer: None,
        };
        let order = WorkOrder {
            system: req.system.clone(),
            messages: req.messages.clone(),
        };
        let report = run.run(&order).await;
        let text = report
            .messages
            .iter()
            .rev()
            .find(|m| m.role == Role::Assistant && !m.text().trim().is_empty())
            .map(Message::text)
            .unwrap_or_default();
        agenium_sandbox::remove_run_temp_dir(&dir);
        let _ = std::fs::remove_dir_all(&dir);
        // A run that stopped before it finished (its step limit, its time, a cancel) left at most part of an answer:
        // memory drawn from it would be drawn from half a reply.
        if !matches!(report.outcome, agenium_backend::Outcome::Completed) {
            return Err(LlmError::Config(format!(
                "{label} did not finish its answer ({})",
                report.outcome.label()
            )));
        }
        if text.trim().is_empty() {
            return Err(LlmError::Config(format!(
                "{label} answered nothing ({})",
                report.outcome.label()
            )));
        }
        Ok(ChatResponse {
            message: Message::assistant_text(text),
            stop: StopReason::EndTurn,
            usage: report.usage,
            model: label,
            dropped_thinking: 0,
            refusal: None,
        })
    }
}
