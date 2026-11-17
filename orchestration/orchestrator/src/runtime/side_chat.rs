//! One model answer beside the agents' runs, for the app's own helpers (ADR-151: the team designer, the skill writer):
//! an API model's single request, or a vendor CLI's chat turn with no tools in an empty scratch folder. The user picks
//! the model, backend and effort; the caller keeps the conversation. Each call's tokens and cost are kept
//! (`helper_call`, ADR-152) for the usage views.

use std::path::PathBuf;
use std::sync::Arc;

use agenium_backend::{Effect, Observer, Outcome, Protocol, WorkOrder};
use agenium_core::{BackendKind, ModelBinding};
use agenium_llm::{ChatEvent, ChatRequest, Message, Role, SystemBlock, Usage};
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

/// What a side call is told as the answer is written: its text and reasoning pieces (`TextDelta`, `ThinkingDelta`),
/// and `Restart` when a dropped stream starts again.
pub type Live<'a> = &'a (dyn Fn(&ChatEvent) + Send + Sync);

/// Hands a CLI's live events to the side call's `Live`.
struct Forward<'a>(Live<'a>);

impl Observer for Forward<'_> {
    fn event(&self, e: &ChatEvent) {
        (self.0)(e)
    }
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
    #[allow(clippy::too_many_arguments)]
    pub async fn side_chat(
        &self,
        kind: &str,
        session: Option<&str>,
        backend: BackendKind,
        model: &ModelBinding,
        system: &str,
        messages: Vec<Message>,
        max_tokens: u32,
    ) -> Result<SideAnswer> {
        self.side_chat_live(
            kind,
            session,
            backend,
            model,
            system,
            messages,
            max_tokens,
            &|_| {},
        )
        .await
    }

    /// `side_chat`, telling `live` the answer's pieces as they are written (the helpers show it live).
    #[allow(clippy::too_many_arguments)]
    pub async fn side_chat_live(
        &self,
        kind: &str,
        session: Option<&str>,
        backend: BackendKind,
        model: &ModelBinding,
        system: &str,
        messages: Vec<Message>,
        max_tokens: u32,
        live: Live<'_>,
    ) -> Result<SideAnswer> {
        if messages.is_empty() {
            return Err(OrchError::Invalid("nothing to answer".into()));
        }
        let started = agenium_core::now_iso();
        let a = match backend {
            BackendKind::Api => {
                self.side_api(model, system, messages, max_tokens, live)
                    .await
            }
            BackendKind::Cli => self.side_cli(session, model, system, messages, live).await,
        }?;
        self.record_helper(kind, session, backend, model, &started, &a);
        Ok(a)
    }

    /// Keeps a helper call's tokens and cost (an API model's priced; a CLI's runs on the user's subscription).
    fn record_helper(
        &self,
        kind: &str,
        session: Option<&str>,
        backend: BackendKind,
        model: &ModelBinding,
        started: &str,
        a: &SideAnswer,
    ) {
        let api = backend == BackendKind::Api;
        let usage = agenium_core::UsageSummary {
            tokens_in: a.usage.total_input(),
            tokens_out: a.usage.output,
            cache_read: a.usage.cache_read,
            cache_write: a.usage.cache_write,
            cache_write_1h: a.usage.cache_write_1h,
            estimated_cost_usd: if api {
                self.cost(&model.provider, &model.model, &a.usage)
            } else {
                0.0
            },
            ..Default::default()
        };
        let call = agenium_store::core::helper_calls::HelperCall {
            id: ulid::Ulid::new().to_string(),
            kind: kind.to_owned(),
            provider: model.provider.clone(),
            model: model.model.clone(),
            api,
            session: session.map(str::to_owned),
            started_at: started.to_owned(),
            ended_at: agenium_core::now_iso(),
            usage,
        };
        let r = self.app.store().commit(None, "system", |w| {
            agenium_store::core::helper_calls::insert(w.tx, &call).map_err(crate::OrchError::from)
        });
        if let Err(e) = r {
            tracing::warn!(error = %e, "a helper call's usage was not kept");
        }
    }

    async fn side_api(
        &self,
        model: &ModelBinding,
        system: &str,
        messages: Vec<Message>,
        max_tokens: u32,
        live: Live<'_>,
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
            &|e| live(&e),
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

    /// A helper chat's turn (`session`) resumes the CLI's own session of the chat's earlier turns when the same CLI and
    /// model wrote it and it holds them all: only the new message is sent. Another CLI or model starts a new session.
    async fn side_cli(
        &self,
        session: Option<&str>,
        model: &ModelBinding,
        system: &str,
        messages: Vec<Message>,
        live: Live<'_>,
    ) -> Result<SideAnswer> {
        use agenium_cli_backend::{run::SessionStore, CliRun, Limits};
        let observer = Forward(live);
        let (driver, program) = self
            .cli_ready_named(&model.provider)
            .await
            .map_err(OrchError::Invalid)?;
        let kept = session.and_then(|s| self.app.helper_cli_session(s));
        // A chat's turns run in the same (empty) folder: a CLI finds the session it resumes by its folder (Claude).
        let name = kept.as_ref().and_then(|k| k.file_name()).map_or_else(
            || ulid::Ulid::new().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        let dir = Scratch(std::env::temp_dir().join(format!("agenium-side-{name}")));
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
            session: kept.map(|dir| {
                if let Some(parent) = dir.parent() {
                    SessionStore::sweep(parent);
                }
                SessionStore {
                    dir,
                    held: messages.len().saturating_sub(1),
                    // Only the model that wrote the session resumes it.
                    window: 0,
                }
            }),
            observer: &observer,
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
        // Only a finished turn is an answer: one cut short (its step limit, its time, a cancel) left part of one.
        if !matches!(report.outcome, Outcome::Completed) {
            return Err(OrchError::Invalid(format!(
                "{label} did not finish its answer ({})",
                report.outcome.label()
            )));
        }
        Ok(SideAnswer {
            text,
            model: label,
            usage: report.usage,
        })
    }
}
