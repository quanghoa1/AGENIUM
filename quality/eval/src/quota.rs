//! Quota used up in the middle of a ticket (decision 3j, ADR-132): the worker's provider answers a real model until the
//! worker has changed a file, then "insufficient_quota". The work so far must stay (worktree, conversation) and the user
//! must be told; then the user gives that same agent another model (the API model again, or a vendor CLI with
//! `--worker-cli`) and it goes on from its conversation: the agent stays, only its model changes. `same`: a Solo agent;
//! `team`: a Lead and two workers, where the ticket stays with its worker though the other is free. `back` (ADR-135): a
//! Solo agent whose quota comes back; nobody changes anything, the scheduler's round half an hour later sets it back to
//! work and it finishes with the same model. Graded on the golden task's own check after the merge.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use agenium_core::plan::*;
use agenium_core::*;
use agenium_llm::{ChatEvent, ChatRequest, ChatResponse, LlmError, ModelInfo, Provider};
use agenium_orchestrator::app::{App, AppOptions, CreateAgent, CreateProject, NoCheckFactory};
use agenium_orchestrator::runtime::{
    AutoApprove, DenyApprover, RunSummary, Runtime, StaticProviders,
};
use agenium_orchestrator::CallCtx;
use agenium_store::core::{events, runs};
use async_trait::async_trait;
use serde_json::json;

use crate::{Mode, TaskResult};

pub const ID: &str = "quota-mid-work";

/// The golden task the worker is doing when its quota runs out (several steps: read, change three files, report).
const TASK: &str = "rename-function";

/// Model calls the worker's provider answers at most before its quota is gone (it is gone sooner: right after the
/// worker's first change of a file).
const CALLS_BEFORE_QUOTA: u32 = 6;

/// Tools that change a file.
const WRITES: [&str; 4] = ["write_file", "edit_file", "multi_edit", "apply_patch"];

/// A provider whose quota runs out in the middle of the work: the real model until the worker has changed a file (or
/// made its last allowed call), so a part of the work is done and a part is left.
struct RunsOut {
    inner: Arc<dyn Provider>,
    /// Calls left; 0 once the quota is gone.
    left: Mutex<u32>,
    /// The quota is back (`back`): every call goes through.
    back: std::sync::atomic::AtomicBool,
}

/// Whether the conversation holds a change of a file that went through.
fn changed_a_file(req: &ChatRequest) -> bool {
    req.messages.iter().flat_map(|m| &m.parts).any(|p| {
        matches!(p, agenium_llm::Part::ToolResult { name, is_error: false, .. }
            if WRITES.contains(&name.as_str()))
    })
}

#[async_trait]
impl Provider for RunsOut {
    fn id(&self) -> &str {
        self.inner.id()
    }
    fn kind(&self) -> &'static str {
        self.inner.kind()
    }
    async fn list_models(&self) -> Result<Vec<ModelInfo>, LlmError> {
        self.inner.list_models().await
    }
    async fn chat(
        &self,
        req: &ChatRequest,
        emit: &(dyn Fn(ChatEvent) + Send + Sync),
    ) -> Result<ChatResponse, LlmError> {
        let gone = !self.back.load(std::sync::atomic::Ordering::SeqCst) && {
            let mut left = self.left.lock().unwrap_or_else(|p| p.into_inner());
            if *left == 0 || changed_a_file(req) {
                *left = 0;
                true
            } else {
                *left -= 1;
                false
            }
        };
        if gone {
            return Err(LlmError::Http {
                status: 429,
                kind: "insufficient_quota".into(),
                message: "You exceeded your current quota (eval)".into(),
                quota: true,
                retry_after: None,
            });
        }
        self.inner.chat(req, emit).await
    }
}

pub async fn run(
    mode: &Mode,
    how: &str,
    taker_cli: Option<(String, String)>,
) -> anyhow::Result<TaskResult> {
    let (team, back) = (how == "team", how == "back");
    let start = Instant::now();
    let Mode::Model {
        provider,
        model,
        effort,
        approve_commands,
    } = mode
    else {
        anyhow::bail!("the quota scenario needs a real model (--provider/--model)");
    };
    let task = crate::tasks()
        .into_iter()
        .find(|t| t.id == TASK)
        .ok_or_else(|| anyhow::anyhow!("no task {TASK}"))?;
    let tmp = tempfile::tempdir()?;
    let (app, _) = App::open(
        &tmp.path().join("root"),
        AppOptions {
            keyring: Arc::new(agenium_secrets::MemoryKeyring::default()),
            checks: Arc::new(NoCheckFactory),
        },
    )?;
    let app = Arc::new(app);
    let project = app.create_project(&CreateProject {
        name: ID.into(),
        path: None,
        key_prefix: None,
        template: None,
        folder: None,
        repo: None,
    })?;
    let root = Path::new(&project.path).to_path_buf();
    for (rel, content) in task.fixture {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap())?;
        std::fs::write(p, content)?;
    }
    app.repo(&project)
        .commit_all(&root, "Eval fixture", ("eval", "eval@agenium.local"), &[])?;
    let binding = |provider: &str| ModelBinding {
        provider: provider.into(),
        model: model.clone(),
        effort: effort.clone(),
        max_tokens: None,
    };
    let agent = |name: &str, role: &str, role_kind: RoleKind, provider: &str| CreateAgent {
        name: name.into(),
        role: role.into(),
        role_kind,
        tags: vec![],
        model: binding(provider),
    };
    // The worker whose quota runs out (in a team, beside a free one that must not take its ticket).
    let first = if team {
        let (_, m) = app.create_team(
            project.id,
            "team",
            &agent("lead", "lead", RoleKind::Lead, "eval"),
            &[
                agent("worker-a", "backend", RoleKind::Worker, "eval-limited"),
                agent("worker-b", "backend", RoleKind::Worker, "eval"),
            ],
        )?;
        m[1].clone()
    } else {
        let (_, a) = app.create_solo(
            project.id,
            &agent("solo", "solo", RoleKind::Lead, "eval-limited"),
        )?;
        a
    };

    let user = CallCtx::user(project.id);
    let ac: Vec<AcceptanceCriterion> = task
        .acceptance
        .iter()
        .enumerate()
        .map(|(i, a)| AcceptanceCriterion {
            id: format!("AC-{}", i + 1),
            text: (*a).into(),
        })
        .collect();
    app.apply(
        &user,
        Intent::ProposeSpec(RequirementSpec {
            goals: vec![task.title.into()],
            scope: vec![],
            out_of_scope: vec![],
            requirements: vec![Requirement {
                id: "REQ-1".into(),
                text: task.description.into(),
                acceptance: ac.clone(),
            }],
            constraints: vec![],
        }),
    )?;
    app.apply(
        &user,
        Intent::CreatePlan(TicketPlan {
            spec_version: 1,
            milestones: vec![Milestone {
                id: "M1".into(),
                title: task.title.into(),
            }],
            items: vec![PlanItem {
                id: "PI-1".into(),
                milestone: "M1".into(),
                title: task.title.into(),
                trace_to: vec!["REQ-1".into()],
                acceptance: ac.clone(),
                role: first.role.clone(),
                depends_on: vec![],
                estimate: Estimate::S,
                scope_paths: vec![],
            }],
        }),
    )?;
    let out = app.apply(
        &user,
        Intent::CreateTicket(forms::NewTicket {
            kind: TicketKind::Task,
            plan_item: "PI-1".into(),
            parent: None,
            title: task.title.into(),
            description: task.description.into(),
            acceptance: ac,
            priority: 2,
            depends_on: vec![],
            required_role: None,
            required_tags: vec![],
            scope_paths: vec![],
            report_template: None,
            follows: None,
        }),
    )?;
    let key: TicketKey = serde_json::from_value(out.result["ticket_id"].clone())?;
    app.apply(
        &user,
        Intent::AssignTicket(forms::AssignTicket {
            ticket_id: key.clone(),
            worker: first.id,
            note: None,
        }),
    )?;

    let approver: Arc<dyn agenium_orchestrator::runtime::Approver> = if *approve_commands {
        Arc::new(AutoApprove)
    } else {
        Arc::new(DenyApprover)
    };
    crate::price_as_eval(&app, provider.as_ref())?;
    let runs_out = Arc::new(RunsOut {
        inner: provider.clone(),
        left: Mutex::new(CALLS_BEFORE_QUOTA),
        back: Default::default(),
    });
    let limited: Arc<dyn Provider> = runs_out.clone();
    let rt = Runtime::new(
        app.clone(),
        Arc::new(StaticProviders(vec![
            ("eval".into(), provider.clone()),
            ("eval-limited".into(), limited),
        ])),
        approver,
    );

    // Until the quota is gone.
    let mut problems: Vec<String> = vec![];
    let mut summaries: Vec<RunSummary> = rt.drive(project.id, 4).await?;
    let stopped = summaries
        .iter()
        .any(|s| s.agent == first.name && s.outcome == "QuotaExhausted");
    if !stopped {
        problems.push(format!("{} never ran out of quota", first.name));
    }
    let t = app.ticket(project.id, &key)?;
    let told = app
        .store()
        .read(|c| events::recent_of_kind(c, "NeedsUser", 20))?
        .iter()
        .any(|e| {
            e.payload["reason"]
                .as_str()
                .unwrap_or_default()
                .contains("exhausted its quota")
        });
    if !told {
        problems.push("the user was not told of the used-up quota".into());
    }
    let ws = app
        .store()
        .read(|c| agenium_store::core::workspaces::get(c, t.id))?
        .filter(|w| !w.removed);
    let changed = ws.as_ref().is_some_and(|w| {
        app.repo(&project)
            .status(Path::new(&w.worktree))
            .is_ok_and(|s| !s.is_empty())
    });

    // The ticket waits with its agent (in a team too: no one else takes it).
    if app.ticket(project.id, &key)?.assignee != Some(first.id) {
        problems.push(format!("{key} left {} while it was set aside", first.name));
    }
    // The work goes on: the quota comes back and the scheduler's round half an hour later sets the agent back to work
    // (`back`), or the user gives the agent another model, which frees it at once.
    let model_before = app.agent(first.id)?.model;
    match &taker_cli {
        _ if back => {
            runs_out
                .back
                .store(true, std::sync::atomic::Ordering::SeqCst);
            let later = std::time::SystemTime::now() + std::time::Duration::from_secs(31 * 60);
            if rt.quota_back_round(later)? != vec![first.id] {
                problems.push("the agent was not set back to work when its quota was back".into());
            }
            if app.agent(first.id)?.model != model_before {
                problems.push("the agent's model changed".into());
            }
        }
        Some((cli, cli_model)) => app.set_agent_backend(
            first.id,
            BackendKind::Cli,
            &ModelBinding {
                provider: cli.clone(),
                model: cli_model.clone(),
                effort: effort.clone(),
                max_tokens: None,
            },
        )?,
        None => app.set_agent_model(first.id, &binding("eval"))?,
    }
    if app.agent(first.id)?.status != AgentStatus::Idle {
        problems.push("the new model did not free the agent".into());
    }
    summaries.extend(rt.drive(project.id, 12).await?);

    // Grading.
    let t = app.ticket(project.id, &key)?;
    let taker = &first;
    if t.assignee != Some(taker.id) {
        problems.push(format!(
            "{key} ended with {:?}, not {}",
            t.assignee, taker.name
        ));
    }
    let finished_by_taker = summaries
        .iter()
        .any(|s| s.agent == taker.name && s.kind == "work" && s.outcome == "Completed");
    if !finished_by_taker {
        problems.push(format!("{} did not finish the work", taker.name));
    }
    if t.status != TicketStatus::Closed {
        problems.push(format!("{key} is {}", t.status));
    }
    if let Err(e) = (task.check)(&root) {
        problems.push(e);
    }

    let (mut tool_errors, mut error_codes, mut errors) =
        (0u32, std::collections::BTreeMap::new(), vec![]);
    let mut tools = std::collections::BTreeMap::new();
    for s in &summaries {
        for e in app.store().read(|c| runs::log_entries(c, s.run))? {
            if e.kind != "tool" {
                continue;
            }
            let tool = e.data["tool"].as_str().unwrap_or_default();
            *tools
                .entry(format!("{}:{}/{tool}", s.agent, s.kind))
                .or_insert(0) += 1;
            if e.data["ok"] == false {
                tool_errors += 1;
                let res = e.data["result"].as_str().unwrap_or_default();
                *error_codes
                    .entry(res.split(':').next().unwrap_or_default().to_owned())
                    .or_insert(0) += 1;
                errors.push(
                    format!("{}/{tool}: {res}", s.agent)
                        .chars()
                        .take(300)
                        .collect(),
                );
            }
        }
    }
    let usage = summaries.iter().fold(UsageSummary::default(), |mut u, s| {
        u.add(&s.usage);
        u
    });
    Ok(TaskResult {
        id: format!("{ID}-{how}"),
        passed: problems.is_empty(),
        closed: t.status == TicketStatus::Closed,
        detail: if problems.is_empty() {
            "ok".into()
        } else {
            problems.join("; ")
        },
        runs: summaries.len(),
        steps: summaries.iter().map(|s| s.steps).sum(),
        tool_errors,
        error_codes,
        errors,
        tools,
        outcomes: summaries
            .iter()
            .map(|s| {
                format!(
                    "{}:{}:{}:{}",
                    s.kind,
                    s.agent,
                    s.ticket.as_ref().map(|k| k.to_string()).unwrap_or_default(),
                    s.outcome
                )
            })
            .collect(),
        reopened: t.reopen_count,
        tokens_in: usage.tokens_in,
        tokens_cached: usage.cache_read,
        tokens_out: usage.tokens_out,
        cost_usd: usage.estimated_cost_usd,
        elapsed_ms: start.elapsed().as_millis() as u64,
        record: None,
        learning: None,
        discipline: Some(json!({
            "quota_stopped": stopped,
            "user_told": told,
            "work_left_in_worktree": changed,
            "taker": taker.name,
            "taker_on": if back { "the same model".to_owned() } else { taker_cli.as_ref().map_or_else(|| "api".to_owned(), |(c, m)| format!("{c} {m}")) },
        })),
    })
}
