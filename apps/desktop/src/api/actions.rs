//! The user's actions: chat, answers, approvals, intents.

use super::*;

pub(super) fn user(o: &Opened, project: ProjectId, intent: Intent) -> R<Value> {
    intent
        .validate()
        .map_err(|e| format!("invalid: {}", serde_json::to_string(&e).unwrap_or_default()))?;
    Ok(o.app
        .apply(&CallCtx::user(project), intent)
        .map_err(err)?
        .result)
}

#[tauri::command]
pub async fn chat(d: State<'_, Desktop>, project: String, message: String) -> R<Value> {
    let o = d.opened()?;
    let s = o.rt.chat(pid(&project)?, &message).await.map_err(err)?;
    Ok(
        json!({ "run": s.run, "kind": s.kind, "outcome": s.outcome, "text": s.text, "steps": s.steps, "usage": s.usage }),
    )
}

/// The messages waiting for the busy Lead (ADR-149), oldest first; the user can still change or take them back.
#[tauri::command]
pub fn lead_queue(d: State<'_, Desktop>, project: String) -> R<Value> {
    let o = d.opened()?;
    let q = o
        .app
        .undelivered_lead_messages(pid(&project)?)
        .map_err(err)?;
    Ok(json!(q))
}

#[tauri::command]
pub fn edit_lead_message(d: State<'_, Desktop>, project: String, id: i64, text: String) -> R<()> {
    let o = d.opened()?;
    o.app
        .edit_lead_message(pid(&project)?, id, &text)
        .map_err(err)
}

#[tauri::command]
pub fn drop_lead_message(d: State<'_, Desktop>, project: String, id: i64) -> R<()> {
    let o = d.opened()?;
    o.app.drop_lead_message(pid(&project)?, id).map_err(err)
}

/// The waiting messages answered now (ADR-149). After turns that failed they wait for the user's next message, or
/// for this.
#[tauri::command]
pub async fn deliver_lead_queue(d: State<'_, Desktop>, project: String) -> R<Value> {
    let o = d.opened()?;
    let s = o.rt.deliver_queued(pid(&project)?).await.map_err(err)?;
    Ok(
        json!({ "run": s.run, "kind": s.kind, "outcome": s.outcome, "text": s.text, "steps": s.steps, "usage": s.usage }),
    )
}

#[tauri::command]
pub fn chat_history(d: State<'_, Desktop>, project: String) -> R<Value> {
    let o = d.opened()?;
    let p = pid(&project)?;
    // A closed project, or one waiting for its team, shows the conversation its last team's Lead had (ADR-130).
    let team = match o.app.team_of_project(p).map_err(err)? {
        Some((t, _)) => Some(t),
        None => o
            .app
            .previous_team(p)
            .map_err(err)?
            .or(o.app.waits_for(p).map_err(err)?)
            .and_then(|t| o.app.team(t).ok())
            .map(|(t, _)| t),
    };
    let Some(team) = team else {
        return Ok(json!([]));
    };
    let Some(lead) = team.lead else {
        return Ok(json!([]));
    };
    let mem = o
        .app
        .agent_store(lead, o.rt.embedder.dim(), o.rt.embedder.model_id())
        .map_err(err)?;
    let msgs = mem.load_messages(&format!("chat:{p}")).map_err(err)?;
    // User turns carry the run context before the request and the project state after it: show what the user wrote.
    let out: Vec<Value> = msgs
        .into_iter()
        .filter_map(|m| serde_json::from_value::<agenium_llm::Message>(m).ok())
        .filter_map(|m| {
            let text = m.text();
            let text = match m.role {
                agenium_llm::Role::User => agenium_orchestrator::runtime::learn::user_words(&text),
                _ => &text,
            }
            .trim()
            .to_owned();
            (!text.is_empty()).then(|| json!({ "role": m.role, "text": text }))
        })
        .collect();
    Ok(json!(out))
}

#[tauri::command]
pub async fn approve(
    d: State<'_, Desktop>,
    project: String,
    what: String,
    target: String,
) -> R<()> {
    let o = d.opened()?;
    let p = pid(&project)?;
    // A merge rebases and runs the pre-merge check (minutes for a test suite): off the window's thread.
    blocking(move || match what.as_str() {
        "spec" => o
            .app
            .approve_spec(p, target.parse().map_err(err)?)
            .map_err(err),
        "plan" => o
            .app
            .approve_plan(p, target.parse().map_err(err)?)
            .map_err(err),
        "merge" => o
            .app
            .user_approve_merge(p, &key(&target)?)
            .map(|_| ())
            .map_err(err),
        other => Err(format!("unknown approval {other}")),
    })
    .await
}

/// Runs a command's slow work (git, the pre-merge check, embeddings, files) on the blocking pool. A command without
/// `async` runs on the window's thread in Tauri 2: the whole window would freeze until it returns.
pub(super) async fn blocking<T: Send + 'static>(f: impl FnOnce() -> R<T> + Send + 'static) -> R<T> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn answer(
    d: State<'_, Desktop>,
    project: String,
    ticket: String,
    question: String,
    answer: String,
    option: Option<usize>,
) -> R<Value> {
    use agenium_orchestrator::engine::UserAnswer;
    let o = d.opened()?;
    let p = pid(&project)?;
    let q: CommentId = question.parse().map_err(|e: IdParseError| e.to_string())?;
    match o
        .app
        .route_user_answer(p, &key(&ticket)?, q, &answer, option)
        .map_err(err)?
    {
        UserAnswer::Answered(out) => Ok(out.result),
        // In a team the answer is a message to the Lead (ADR-122); its turn runs in the background. It is stored first
        // (ADR-147): acknowledged, it reaches the Lead even if the app stops before the turn; while the Lead is busy it
        // waits in the queue (ADR-149).
        UserAnswer::ToLead(message) => {
            o.app.post_lead_message(p, &message).map_err(err)?;
            let rt = o.rt.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = rt.deliver_queued(p).await {
                    tracing::warn!(error = %e, "the answer did not reach the Lead");
                }
            });
            Ok(json!({ "routed_to": "lead" }))
        }
    }
}

#[tauri::command]
pub fn decide_permission(
    d: State<'_, Desktop>,
    project: String,
    ticket: String,
    request: String,
    approve: bool,
    always: bool,
    note: Option<String>,
) -> R<Value> {
    let o = d.opened()?;
    let r: CommentId = request.parse().map_err(|e: IdParseError| e.to_string())?;
    let decision = if approve {
        PermissionDecision::Approve
    } else {
        PermissionDecision::Deny
    };
    Ok(o.app
        .decide_permission(pid(&project)?, &key(&ticket)?, r, decision, always, note)
        .map_err(err)?
        .result)
}

#[tauri::command]
pub fn decide_command(
    d: State<'_, Desktop>,
    id: u64,
    decision: String,
    reason: Option<String>,
) -> R<bool> {
    let a = match decision.as_str() {
        "allow" => Approval::Allow,
        "always" => Approval::AllowAlways,
        _ => Approval::Deny(reason.unwrap_or_else(|| "declined in the app".into())),
    };
    Ok(d.approvals.decide(id, a))
}

#[tauri::command]
pub fn comment(d: State<'_, Desktop>, project: String, ticket: String, text: String) -> R<()> {
    let o = d.opened()?;
    o.app
        .user_comment(pid(&project)?, &key(&ticket)?, &text)
        .map(|_| ())
        .map_err(err)
}

#[derive(Deserialize)]
pub struct Feedback {
    ac_id: String,
    feedback: String,
}

// The arguments are the IPC call's named fields.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn ticket_action(
    d: State<'_, Desktop>,
    project: String,
    ticket: String,
    action: String,
    reason: Option<String>,
    agent: Option<String>,
    priority: Option<u8>,
    feedback: Option<Vec<Feedback>>,
) -> R<Value> {
    let o = d.opened()?;
    let p = pid(&project)?;
    let k = key(&ticket)?;
    let reason = reason.unwrap_or_default();
    // An approval runs the merge right here (the rebase and the pre-merge check: minutes for a test suite); on the
    // window's thread it froze the whole window (audit H4, 2026-10-31).
    blocking(move || match action.as_str() {
        "priority" => {
            let t = o
                .app
                .user_set_priority(p, &k, priority.unwrap_or(2))
                .map_err(err)?;
            Ok(json!({ "priority": t.priority }))
        }
        "assign" => {
            let id: AgentId = agent
                .unwrap_or_default()
                .parse()
                .map_err(|e: IdParseError| e.to_string())?;
            user(
                &o,
                p,
                Intent::AssignTicket(forms::AssignTicket {
                    ticket_id: k,
                    worker: id,
                    note: None,
                }),
            )
        }
        "approve" | "reopen" => user(
            &o,
            p,
            Intent::ReviewTicket(forms::ReviewDecision {
                ticket_id: k,
                decision: if action == "approve" {
                    forms::ReviewVerdict::Approve
                } else {
                    forms::ReviewVerdict::Reopen
                },
                ac_feedback: feedback
                    .unwrap_or_default()
                    .into_iter()
                    .map(|f| forms::AcFeedback {
                        ac_id: f.ac_id,
                        feedback: f.feedback,
                    })
                    .collect(),
                required_changes: vec![],
            }),
        ),
        "close" => user(
            &o,
            p,
            Intent::CloseTicket(forms::CloseTicket {
                ticket_id: k,
                reason,
            }),
        ),
        "cancel" => user(
            &o,
            p,
            Intent::CancelTicket(forms::CancelTicket {
                ticket_id: k,
                reason,
                replaced_by: None,
            }),
        ),
        "waive" => {
            let note = (!reason.trim().is_empty()).then_some(reason.trim());
            o.app.waive_cancelled(p, &k, note).map_err(err)?;
            Ok(json!({ "waived": true }))
        }
        other => Err(format!("unknown ticket action {other}")),
    })
    .await
}

/// A ticket prefix made from a project name, free among the projects (the form shows it until the user gives one).
#[tauri::command]
pub fn suggest_key_prefix(d: State<'_, Desktop>, name: String) -> R<String> {
    d.opened()?.app.suggest_key_prefix(&name).map_err(err)
}

/// `source`: a folder name under the app's projects folder, or a git repository to clone there. `key_prefix`: the
/// tickets' prefix the user gave (checked), else one made from the name.
#[tauri::command]
pub async fn create_project(
    d: State<'_, Desktop>,
    name: String,
    source: Option<String>,
    key_prefix: Option<String>,
) -> R<Value> {
    let o = d.opened()?;
    let key_prefix = match key_prefix
        .map(|p| p.trim().to_uppercase())
        .filter(|p| !p.is_empty())
    {
        Some(p) => {
            o.app.check_key_prefix(&p).map_err(err)?;
            p
        }
        None => o.app.suggest_key_prefix(&name).map_err(err)?,
    };
    let source = source
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty());
    let (folder, repo) = match source {
        Some(s) if agenium_orchestrator::app::is_repo_url(&s) => (None, Some(s)),
        other => (other, None),
    };
    let req = CreateProject {
        name,
        path: None,
        key_prefix: Some(key_prefix),
        template: None,
        folder,
        repo,
    };
    // A clone may take a while: off the UI thread.
    let p = tokio::task::spawn_blocking(move || o.app.create_project(&req))
        .await
        .map_err(|e| e.to_string())?
        .map_err(err)?;
    Ok(json!({ "id": p.id, "name": p.name }))
}

pub(super) fn binding(provider: &str, model: &str, effort: &str) -> R<ModelBinding> {
    Ok(ModelBinding {
        provider: provider.to_owned(),
        model: model.to_owned(),
        effort: effort.parse()?,
        max_tokens: None,
    })
}

#[derive(Debug, Deserialize)]
pub struct ModelArg {
    provider: String,
    model: String,
    effort: String,
    /// `api` or `cli` (the CLI program is `provider`).
    #[serde(default)]
    backend: Option<String>,
}

impl ModelArg {
    /// The backend and model binding the form describes.
    pub(crate) fn binding(&self) -> R<(BackendKind, ModelBinding)> {
        let b = binding(&self.provider, &self.model, &self.effort)?;
        let backend = if self.backend.as_deref() == Some("cli") {
            BackendKind::Cli
        } else {
            BackendKind::Api
        };
        Ok((backend, b))
    }
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn create_team(
    d: State<'_, Desktop>,
    project: String,
    template: String,
    model: ModelArg,
    lead_model: Option<ModelArg>,
    planner_model: Option<ModelArg>,
    prefix: Option<String>,
    role_models: Option<bool>,
) -> R<Value> {
    let o = d.opened()?;
    let p = pid(&project)?;
    // A model picked from the agent CLIs runs on that CLI: each member gets the backend of the model it was given
    // (the Lead's, the planner's, its role's own, or the team's).
    let out = o
        .app
        .create_team_with_models(
            p,
            &template,
            &agenium_orchestrator::team_templates::TeamModels {
                model: model.binding()?,
                lead: lead_model.as_ref().map(ModelArg::binding).transpose()?,
                planner: planner_model.as_ref().map(ModelArg::binding).transpose()?,
                role_models: role_models.unwrap_or(false),
            },
            prefix.as_deref().unwrap_or(""),
        )
        .map_err(err)?;
    Ok(json!(out))
}

#[tauri::command]
pub fn create_solo(
    d: State<'_, Desktop>,
    project: String,
    name: String,
    model: ModelArg,
) -> R<Value> {
    let o = d.opened()?;
    let b = binding(&model.provider, &model.model, &model.effort)?;
    let (_, a) = o
        .app
        .create_solo(
            pid(&project)?,
            &CreateAgent {
                name,
                role: "solo".into(),
                role_kind: RoleKind::Lead,
                tags: vec![],
                model: b.clone(),
            },
        )
        .map_err(err)?;
    if model.backend.as_deref() == Some("cli") {
        o.app
            .set_agent_backend(a.id, BackendKind::Cli, &b)
            .map_err(err)?;
        // The agent as it is now (on its CLI), not as it was created.
        return Ok(json!(o.app.agent(a.id).map_err(err)?));
    }
    Ok(json!(a))
}

#[tauri::command]
pub fn set_agent_model(d: State<'_, Desktop>, agent: String, model: ModelArg) -> R<()> {
    let o = d.opened()?;
    let id: AgentId = agent.parse().map_err(|e: IdParseError| e.to_string())?;
    let (backend, b) = model.binding()?;
    o.app.set_agent_backend(id, backend, &b).map_err(err)
}

/// The agent's context against the window of the model it has now (ADR-155): after a change to a smaller model the
/// context of its last run is shown against the new window, over it until the next run folds the conversation.
#[tauri::command]
pub async fn agent_context(d: State<'_, Desktop>, agent: String) -> R<Value> {
    let o = d.opened()?;
    let id: AgentId = agent.parse().map_err(|e: IdParseError| e.to_string())?;
    let a = o.app.agent(id).map_err(err)?;
    let (used, was) = o.app.context_now(id).map_err(err)?.unwrap_or((0, 0));
    let window = o.rt.window_of(&a).await;
    Ok(json!({
        "used": used,
        "was": was,
        "window": window,
        "over": window > 0 && used > window * 85 / 100,
    }))
}

/// What a model change does to the agent's context (ADR-155): the context it is at, and the new model's window. Over
/// 85 % of it, the next run clears and folds the conversation to fit first; the UI says so before the change.
#[tauri::command]
pub async fn context_check(d: State<'_, Desktop>, agent: String, model: ModelArg) -> R<Value> {
    let o = d.opened()?;
    let id: AgentId = agent.parse().map_err(|e: IdParseError| e.to_string())?;
    let (backend, b) = model.binding()?;
    let now = o.app.context_now(id).map_err(err)?;
    let window = o.rt.window_for(backend, &b.provider, &b.model).await;
    let (used, was) = now.unwrap_or((0, 0));
    Ok(json!({
        "used": used,
        "was": was,
        "window": window,
        "over": window > 0 && used > window * 85 / 100,
    }))
}

/// The user approves (an agent with the role joins the Lead's team, with the Lead's model) or declines a role the
/// Lead proposed (decision 3l).
#[tauri::command]
pub fn decide_role_proposal(
    d: State<'_, Desktop>,
    project: String,
    proposal: String,
    approve: bool,
    note: Option<String>,
) -> R<Value> {
    let o = d.opened()?;
    let p = o
        .app
        .decide_role_proposal(pid(&project)?, &proposal, approve, None, note.as_deref())
        .map_err(err)?;
    Ok(json!(p))
}

/// Model prices per configured provider and where they came from.
#[tauri::command]
pub fn prices(d: State<'_, Desktop>) -> R<Value> {
    Ok(json!(d.opened()?.app.price_view()))
}

/// The embedding model memory recalls with: the local model (meaning-level recall) when installed, else keyword-level
/// hashing. `active` is what this session uses (a model installed during it is used after a restart).
#[tauri::command]
pub fn embedding_model(d: State<'_, Desktop>) -> R<Value> {
    use agenium_memory::local::{installed, BGE_SMALL_EN as SPEC};
    let o = d.opened()?;
    let models = o.app.layout().models();
    Ok(json!({
        "id": SPEC.id,
        "mb": SPEC.total_bytes() >> 20,
        "installed": installed(&SPEC, &models),
        "active": o.rt.embedder.model_id(),
    }))
}

/// Prepares the embedding model again after a failure (the window shows the preparation until it is ready).
#[tauri::command]
pub fn retry_embedding_model(d: State<'_, Desktop>, handle: AppHandle) {
    if d.model_prep().phase == "error" {
        tauri::async_runtime::spawn(crate::model::ensure(handle));
    }
}

/// Downloads the local embedding model (pinned revision, sha256-checked).
#[tauri::command]
pub async fn install_embedding_model(d: State<'_, Desktop>) -> R<()> {
    use agenium_memory::local::{install, BGE_SMALL_EN as SPEC};
    let o = d.opened()?;
    let http = agenium_llm::http::Http::default_client().map_err(err)?;
    install(&SPEC, &o.app.layout().models(), &http, &|m| {
        tracing::info!(step = m, "embedding model")
    })
    .await
}

/// Downloads the price catalogue now; returns how many earlier runs got a price.
#[tauri::command]
pub async fn refresh_prices(d: State<'_, Desktop>) -> R<Value> {
    let o = d.opened()?;
    let n = agenium_orchestrator::prices::refresh_price_catalogue(&o.app)
        .await
        .map_err(err)?;
    Ok(json!({ "priced": n, "view": o.app.price_view() }))
}

#[tauri::command]
pub fn rename_agent(d: State<'_, Desktop>, agent: String, name: String) -> R<()> {
    let o = d.opened()?;
    let id: AgentId = agent.parse().map_err(|e: IdParseError| e.to_string())?;
    o.app.rename_agent(id, &name).map_err(err)
}

#[tauri::command]
pub fn set_agent_status(d: State<'_, Desktop>, agent: String, status: String) -> R<()> {
    let o = d.opened()?;
    let id: AgentId = agent.parse().map_err(|e: IdParseError| e.to_string())?;
    // The user can only bring an agent back (after a model change or new credit): Idle.
    if status != "Idle" {
        return Err("only Idle can be set by hand".into());
    }
    o.app.set_agent_status(id, AgentStatus::Idle).map_err(err)
}

/// The user stops a project's team (its runs stop after the current step).
/// The user reopens a closed project (ADR-130): with a free team on it, waiting for a busy one (`wait`), or none.
#[tauri::command]
pub fn reopen_project(
    d: State<'_, Desktop>,
    project: String,
    team: Option<String>,
    wait: Option<bool>,
) -> R<()> {
    use agenium_orchestrator::project_close::Taker;
    let team = team
        .as_deref()
        .map(|t| t.parse())
        .transpose()
        .map_err(|e: IdParseError| e.to_string())?;
    let taker = match (team, wait == Some(true)) {
        (Some(t), true) => Taker::WaitFor(t),
        (Some(t), false) => Taker::Team(t),
        (None, _) => Taker::Nobody,
    };
    d.opened()?
        .app
        .reopen_project(pid(&project)?, taker)
        .map_err(err)
}

/// The user closes a done project (ADR-130; no agent can): its team is set free to take another project. Says
/// the freed team's name and the project waiting for it that took it at once.
#[tauri::command]
pub fn close_project(d: State<'_, Desktop>, project: String) -> R<Value> {
    let o = d.opened()?;
    let (team, taken) = o.app.close_project(pid(&project)?).map_err(err)?;
    let team = match team {
        Some(t) => Some(o.app.team(t).map_err(err)?.0.name),
        None => None,
    };
    let taken = match taken {
        Some(p) => Some(o.app.project(p).map_err(err)?.name),
        None => None,
    };
    Ok(json!({ "team": team, "taken_by": taken }))
}

#[tauri::command]
pub fn pause_project(d: State<'_, Desktop>, project: String) -> R<()> {
    d.opened()?.rt.pause_project(pid(&project)?).map_err(err)
}

/// The user lets a stopped project's team go on.
#[tauri::command]
pub fn resume_project(d: State<'_, Desktop>, project: String) -> R<()> {
    d.opened()?.rt.resume_project(pid(&project)?).map_err(err)
}

/// Kill switch: every live run stops and every working project is paused (resume them one by one).
#[tauri::command]
pub fn stop_all(d: State<'_, Desktop>) -> R<()> {
    let o = d.opened()?;
    for p in o.app.projects().map_err(err)? {
        if o.app.run_state(p.id).map_err(err)? == agenium_orchestrator::views::RunState::Running {
            o.rt.pause_project(p.id).map_err(err)?;
        }
    }
    let (rt, app) = (o.rt.clone(), o.app.clone());
    rt.cancel.cancel();
    // The stop holds until the runs going at the click have ended: a run checks it between steps, and a model call
    // or a tool can take minutes (a fixed 5 s reset let those runs go on). Lifted after 5 minutes at the latest,
    // so a run row left open by a crash cannot block the next chat.
    let going: std::collections::HashSet<RunId> = app
        .store()
        .read(runs::unfinished)
        .map(|rs| rs.into_iter().map(|r| r.id).collect())
        .unwrap_or_default();
    tauri::async_runtime::spawn(async move {
        for _ in 0..300 {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            let still = app
                .store()
                .read(runs::unfinished)
                .map(|rs| rs.iter().any(|r| going.contains(&r.id)))
                .unwrap_or(false);
            if !still {
                break;
            }
        }
        rt.cancel.reset();
    });
    Ok(())
}
