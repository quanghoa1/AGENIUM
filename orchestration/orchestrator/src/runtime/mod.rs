//! Agent runtime (M1): turns scheduled tickets into agent runs and handles their outcomes.
//!
//! * **Work run**: the assignee works on a ready ticket in its worktree (work tools + its protocol tools).
//! * **Review run**: the Lead (the Solo agent itself in Solo mode) reviews a Resolved ticket from its report and
//!   diff, approves (merge queue) or reopens, then closes it once merged.
//! * **Chat run**: the Lead/Solo talks with the user (discovery, spec, plan, tickets).
//!
//! Each run rebuilds its work order from stored data in a cache-stable order (decision 36), so a crash or a model
//! switch loses nothing; continuation runs resume the stored conversation (or the checkpoint summary).

pub mod bridge;
mod chat_history;
mod cli_brain;
mod cli_fit;
mod context;
mod execute;
mod lead_runs;
pub mod learn;
mod mcp_gate;
mod models;
mod pool;
mod quota_back;
mod recall;
mod reflect;
mod scheduling;
pub mod side_chat;
mod stalls;
mod steering;
mod ticket_runs;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use agenium_backend::prompt::{self, Manifest};
use agenium_backend::{
    sorted_specs, work_specs, Cancel, LoopConfig, Outcome, Run, RunReport, WorkOrder,
};
use agenium_core::*;
use agenium_llm::{ChatEvent, Message, Provider, RetryPolicy, SystemBlock, ToolSpec};
use agenium_memory::{Embedder, MemoryRetriever};
use agenium_skills::SkillSet;
use agenium_store::core::{comments, runs, tickets, workspaces};
use agenium_tools::{Tool, ToolCtx};
use serde::Serialize;

use crate::app::App;
use crate::{OrchError, Result};
pub use bridge::{Approval, Approver, AutoApprove, DenyApprover, RunKind};
use bridge::{RunGate, RunObserver, RunProtocol};
use execute::OpenRun;
mod providers;
pub use providers::{ConfiguredProviders, ProviderSource, StaticProviders};

pub use models::apply_caps;
use steering::SteerQueue;

/// Text-only answers in a review run: the review ends with a decision, then the close.
const REVIEW_NUDGE: &str = "Text alone does not finish the review. Call review_ticket (Approve, or Reopen with feedback on every unmet acceptance criterion at once). Once an approved ticket is merged, or has no code, call close_ticket with a reason. Stop only when a tool result tells you to.";

/// Text-only answers in a Lead inbox run.
const INBOX_NUDGE: &str = "Text alone does not handle the inbox. Use answer_question (or escalate it), approve_permission, create_ticket and assign_ticket until every item is handled; the run ends when the inbox is empty. Only when the user's latest messages ask you to wait or hold, leave those items and say so in one sentence: the user is told.";

/// Text-only answers in a planner run.
const PLAN_NUDGE: &str =
    "Text alone does not hand in the plan. Call submit_plan_draft with milestones, items and \
your rationale; that ends your run.";

fn nudge_for(kind: RunKind) -> Option<String> {
    match kind {
        RunKind::Review => Some(REVIEW_NUDGE.to_owned()),
        RunKind::Inbox => Some(INBOX_NUDGE.to_owned()),
        RunKind::Plan => Some(PLAN_NUDGE.to_owned()),
        RunKind::Work | RunKind::Chat => None,
    }
}

/// How often a work run's ticket is checked for having ended under it.
const TICKET_WATCH: std::time::Duration = std::time::Duration::from_secs(2);

/// Stops a background task when dropped.
struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RunSummary {
    pub run: RunId,
    pub kind: String,
    pub ticket: Option<TicketKey>,
    pub agent: String,
    pub outcome: String,
    pub steps: u32,
    pub usage: UsageSummary,
    /// Last assistant text (chat answer, checkpoint note).
    pub text: String,
}

/// Live event callback (agent view).
pub type LiveSink = Arc<dyn Fn(&RunId, &ChatEvent) + Send + Sync>;

pub struct Runtime {
    pub app: Arc<App>,
    pub providers: Arc<dyn ProviderSource>,
    pub approver: Arc<dyn Approver>,
    pub embedder: Arc<dyn Embedder>,
    pub retry: RetryPolicy,
    pub live: Option<LiveSink>,
    pub cancel: Cancel,
    /// Facts injected per work order.
    pub memory_k: usize,
    /// MCP connections of the agents (one per agent and server).
    pub mcp: Arc<agenium_mcp::McpHub>,
    /// What each (provider, model) can do, looked up once (ADR-31).
    model_infos: Mutex<std::collections::HashMap<(String, String), Option<agenium_llm::ModelInfo>>>,
    /// Each agent CLI's detection (version and login probes) and when it was made; reused for [`CLI_INFO_TTL`].
    cli_infos: Mutex<
        std::collections::HashMap<
            agenium_cli_backend::CliKind,
            (std::time::Instant, agenium_cli_backend::detect::CliInfo),
        >,
    >,
    /// Per project: the stop flag of its work (child of `cancel`); pausing a project stops its runs, not others.
    project_cancels: Mutex<std::collections::HashMap<ProjectId, Cancel>>,
    /// Per project: the inbox the Lead last failed on and how many times in a row (stops re-running a Lead that
    /// cannot handle the same inbox).
    inbox_failures: Mutex<std::collections::HashMap<ProjectId, (String, usize)>>,
    /// Failed planner runs per (project, spec version).
    planner_failures: Mutex<std::collections::HashMap<(ProjectId, u32), usize>>,
    /// The message queues of each Lead or Solo agent's runs while they last (ADR-104), in start order: a chat turn and
    /// a review or inbox run of the same Lead can overlap. Workers are never here: they get work through tickets and
    /// the Lead's plan only.
    steering: Mutex<std::collections::HashMap<AgentId, Vec<SteerQueue>>>,
    /// Turns in a row that failed to answer a project's queued messages: from MAX_FAILED_RUNS on, the messages wait
    /// for the user's next message instead of a failing turn every round.
    left_failures: Mutex<std::collections::HashMap<ProjectId, usize>>,
    /// Agents set aside for their quota: when each is tried again (ADR-135).
    quota_waits: Mutex<std::collections::HashMap<AgentId, quota_back::QuotaWait>>,
}

/// Two stopped/failed runs in a row on one ticket: stop auto-restarting it and ask the user (avoids burning tokens on
/// a deterministic failure; the stall-nudge of M4 builds on this).
pub(crate) const MAX_FAILED_RUNS: usize = 2;

/// A run that stopped or failed. A Cancelled run is the user's pause or stop (ADR-63), not a failure: it never stops
/// the ticket's restarts nor counts against a planner or an inbox.
fn failed(outcome: &str) -> bool {
    outcome.starts_with("Stopped") || outcome.starts_with("Error")
}

impl Runtime {
    pub fn new(
        app: Arc<App>,
        providers: Arc<dyn ProviderSource>,
        approver: Arc<dyn Approver>,
    ) -> Self {
        let r = app.config().retry;
        let retry = RetryPolicy {
            base: std::time::Duration::from_millis(r.base_ms),
            cap: std::time::Duration::from_millis(r.max_backoff_ms),
            max_attempts: r.max_attempts,
            max_total: std::time::Duration::from_secs(r.max_total_secs),
        };
        let k = app.config().memory.top_k as usize;
        let embedder = app.embedder();
        let mcp = Arc::new(crate::mcp::hub(app.clone()));
        Self {
            project_cancels: Mutex::default(),
            model_infos: Mutex::default(),
            cli_infos: Mutex::default(),
            inbox_failures: Mutex::default(),
            planner_failures: Mutex::default(),
            steering: Mutex::default(),
            left_failures: Mutex::default(),
            quota_waits: Mutex::default(),
            mcp,
            app,
            providers,
            approver,
            embedder,
            retry,
            live: None,
            cancel: Cancel::default(),
            memory_k: k,
        }
    }
}

pub(crate) async fn futures_join_all<F: std::future::Future>(
    futs: impl Iterator<Item = F>,
) -> Vec<F::Output> {
    let v: Vec<F> = futs.collect();
    let mut out = Vec::with_capacity(v.len());
    // Runs of different agents proceed concurrently.
    let pinned: Vec<std::pin::Pin<Box<F>>> = v.into_iter().map(Box::pin).collect();
    let results = join_all_boxed(pinned).await;
    out.extend(results);
    out
}

async fn join_all_boxed<F: std::future::Future>(
    mut futs: Vec<std::pin::Pin<Box<F>>>,
) -> Vec<F::Output> {
    use std::task::Poll;
    let mut results: Vec<Option<F::Output>> = futs.iter().map(|_| None).collect();
    std::future::poll_fn(|cx| {
        let mut pending = false;
        for (i, f) in futs.iter_mut().enumerate() {
            if results[i].is_none() {
                match f.as_mut().poll(cx) {
                    Poll::Ready(v) => results[i] = Some(v),
                    Poll::Pending => pending = true,
                }
            }
        }
        if pending {
            Poll::Pending
        } else {
            Poll::Ready(())
        }
    })
    .await;
    results
        .into_iter()
        .map(|r| r.expect("every future completed"))
        .collect()
}
