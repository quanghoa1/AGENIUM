//! One model answer beside the agents' runs, for the app's own helpers (ADR-151: the team designer, the skill writer):
//! an API model's single request, or a vendor CLI's chat turn with no tools in an empty scratch folder. The user picks
//! the model, backend and effort; nothing is stored, the caller keeps the conversation.

use std::path::PathBuf;
use std::sync::Arc;

use agenium_backend::{Effect, NoopObserver, Outcome, Protocol, WorkOrder};
use agenium_core::{BackendKind, ModelBinding};
use agenium_llm::{ChatRequest, Message, Role, SystemBlock, Usage};
use agenium_tools::{DenyAll, ToolCtx, ToolError};
use async_trait::async_trait;

use super::Runtime;
use crate::{OrchError, Result};

/// A side call's turns at most, and how long a CLI's may last.
const CLI_STEPS: u32 = 12;
const CLI_TIMEOUT_MINS: u64 = 10;

/// What a side call answered.
#[derive(Debug, Clone)]
pub struct SideAnswer {
    pub text: String,
    /// The model that answered (`codex:gpt-6-luna` for a CLI).
    pub model: String,
    pub usage: Usage,
}

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

/// An empty folder for a CLI's side turn, removed when dropped with the temporary folder its commands would use (a CLI
/// that runs in the sandbox writes its git config there).
struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        agenium_sandbox::remove_run_temp_dir(&self.0);
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

impl Runtime {
    /// One answer to `messages` from `model` on `backend`, under `system`. A CLI must be installed, logged in and
    /// usable here (as for an agent's run); it reads and writes nothing (no tools, an empty folder, read-only).
    pub async fn side_chat(
        &self,
        backend: BackendKind,
        model: &ModelBinding,
        system: &str,
        messages: Vec<Message>,
        max_tokens: u32,
    ) -> Result<SideAnswer> {
        if messages.is_empty() {
            return Err(OrchError::Invalid("nothing to answer".into()));
        }
        match backend {
            BackendKind::Api => self.side_api(model, system, messages, max_tokens).await,
            BackendKind::Cli => self.side_cli(model, system, messages).await,
        }
    }

    async fn side_api(
        &self,
        model: &ModelBinding,
        system: &str,
        messages: Vec<Message>,
        max_tokens: u32,
    ) -> Result<SideAnswer> {
        let provider = self.providers.provider(&model.provider)?;
        let req = ChatRequest {
            model: model.model.clone(),
            system: vec![SystemBlock {
                text: system.to_owned(),
                cache: false,
            }],
            messages,
            tools: vec![],
            effort: model.effort.clone(),
            max_tokens: model.max_tokens.unwrap_or(max_tokens),
            cache_key: None,
            long_cache: false,
            tools_off: false,
        };
        let resp = agenium_llm::chat_with_retry(
            provider.as_ref(),
            &req,
            &self.retry,
            &|_| {},
            |_, _, _| {},
        )
        .await
        .map_err(|e| OrchError::Invalid(format!("{}: {e}", model.provider)))?;
        Ok(SideAnswer {
            text: resp.message.text(),
            model: resp.model,
            usage: resp.usage,
        })
    }

    async fn side_cli(
        &self,
        model: &ModelBinding,
        system: &str,
        messages: Vec<Message>,
    ) -> Result<SideAnswer> {
        use agenium_cli_backend::{CliRun, Limits};
        let (driver, program) = self
            .cli_ready_named(&model.provider)
            .await
            .map_err(OrchError::Invalid)?;
        let dir = Scratch(std::env::temp_dir().join(format!("agenium-side-{}", ulid::Ulid::new())));
        std::fs::create_dir_all(&dir.0)?;
        let ctx = ToolCtx::new(
            &dir.0,
            "side",
            Arc::new(DenyAll("a side answer runs no command".into())),
        );
        let label = format!("{}:{}", model.provider, model.model);
        let run = CliRun {
            driver,
            program,
            model: model.model.clone(),
            effort: Some(model.effort.clone()),
            env: ctx.env.clone(),
            cwd: dir.0.clone(),
            label: label.clone(),
            protocol: &NoProtocol,
            protocol_specs: vec![],
            tools: vec![],
            ctx: &ctx,
            session: None,
            observer: &NoopObserver,
            cancel: self.cancel.child(),
            limits: Limits {
                max_steps: CLI_STEPS,
                timeout: std::time::Duration::from_secs(CLI_TIMEOUT_MINS * 60),
                ..Limits::default()
            },
            chat: true,
            nudge: None,
            subagents: false,
            read_only: true,
            uncontained: driver.control() == agenium_cli_backend::Control::Sandbox
                && agenium_sandbox::detect().backend.is_none(),
            on_limits: None,
            steer: None,
        };
        let order = WorkOrder {
            system: vec![SystemBlock {
                text: system.to_owned(),
                cache: false,
            }],
            messages,
        };
        let report = run.run(&order).await;
        let text = report
            .messages
            .iter()
            .rev()
            .find(|m| m.role == Role::Assistant && !m.text().trim().is_empty())
            .map(Message::text)
            .unwrap_or_default();
        match report.outcome {
            Outcome::Completed => {}
            other if text.trim().is_empty() => {
                return Err(OrchError::Invalid(format!(
                    "{label} gave no answer ({})",
                    other.label()
                )))
            }
            _ => {}
        }
        Ok(SideAnswer {
            text,
            model: label,
            usage: report.usage,
        })
    }
}
