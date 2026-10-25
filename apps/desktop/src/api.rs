//! IPC commands of the desktop app. Read models come from the orchestrator (`views`, `inbox`, store queries); every
//! action goes through the same App/Runtime methods as the CLI, so the rules are the orchestrator's, never the UI's.

use std::collections::{BTreeMap, HashMap};

use agenium_core::forms::{self, PermissionDecision};
use agenium_core::*;
use agenium_orchestrator::app::{CreateAgent, CreateProject};
use agenium_orchestrator::runtime::Approval;
use agenium_orchestrator::CallCtx;
use agenium_store::core::{comments, events, runs, workspaces};
use serde::Deserialize;
use serde_json::{json, Value};
use tauri::{AppHandle, State};

use crate::state::{err, Desktop, Opened, R};

pub(crate) fn key(s: &str) -> R<TicketKey> {
    s.parse().map_err(|e: IdParseError| e.to_string())
}

pub(crate) fn pid(s: &str) -> R<ProjectId> {
    s.parse().map_err(|e: IdParseError| e.to_string())
}

pub(crate) fn names(o: &Opened) -> R<HashMap<AgentId, String>> {
    Ok(o.app
        .agents()
        .map_err(err)?
        .into_iter()
        .map(|a| (a.id, a.name))
        .collect())
}

// ---------------------------------------------------------------- setup / status

#[tauri::command]
pub fn status(d: State<'_, Desktop>) -> Value {
    let opened = d.opened().ok();
    let cfg = opened.as_ref().map(|o| o.app.config());
    json!({
        "root": d.root.display().to_string(),
        "ready": opened.is_some(),
        "version": env!("CARGO_PKG_VERSION"),
        "ui_language": cfg.as_ref().map(|c| c.app.language.clone()),
    })
}

/// How agent shell commands are approved (ADR-64) and which sandbox backend works on this machine.
#[tauri::command]
pub fn command_policy(d: State<'_, Desktop>) -> R<Value> {
    let o = d.opened()?;
    let sb = agenium_sandbox::detect();
    Ok(
        json!({ "approval": o.app.config().sandbox.approval, "backend": sb.backend, "reason": sb.reason }),
    )
}

#[tauri::command]
pub fn set_command_policy(
    d: State<'_, Desktop>,
    approval: agenium_config::ApprovalPolicy,
) -> R<Value> {
    d.opened()?
        .app
        .update_config(|c| c.sandbox.approval = approval)
        .map_err(err)?;
    command_policy(d)
}

#[tauri::command]
pub fn setup(d: State<'_, Desktop>, handle: AppHandle, ui_language: String) -> R<Value> {
    let input = agenium_config::setup::SetupInput {
        root: Some(d.root.clone()),
        ui_language,
    };
    let home = dirs::home_dir().unwrap_or_default().join(".agenium");
    // A root given by AGENIUM_HOME is this process's override: nothing is remembered machine-wide.
    let by_env = std::env::var_os(agenium_config::root::ENV_HOME).is_some_and(|v| !v.is_empty());
    let bootstrap = agenium_config::Bootstrap::default_path().filter(|_| !by_env);
    agenium_config::setup::complete_setup(&input, &home, bootstrap.as_deref()).map_err(err)?;
    d.open(&handle)?;
    Ok(status(d))
}

// ---------------------------------------------------------------- overview

#[tauri::command]
pub fn projects(d: State<'_, Desktop>) -> R<Vec<Value>> {
    let o = d.opened()?;
    let mut out = vec![];
    for p in o.app.projects().map_err(err)? {
        let tickets = o.app.tickets(p.id).map_err(err)?;
        let mut by_status: BTreeMap<String, usize> = BTreeMap::new();
        for t in &tickets {
            *by_status.entry(t.status.to_string()).or_default() += 1;
        }
        let team = o.app.team_of_project(p.id).map_err(err)?.map(|(t, m)| {
            json!({ "id": t.id, "name": t.name, "solo": t.solo, "template": t.template, "lead": t.lead,
                    "members": m.len() })
        });
        let pv = o.app.plan_view(p.id).map_err(err)?;
        out.push(json!({
            "id": p.id, "name": p.name, "path": p.path, "key_prefix": p.key_prefix, "status": p.status,
            "main_branch": p.main_branch, "team": team, "tickets": tickets.len(), "by_status": by_status,
            "plan_items": pv.items.len(), "plan_done": pv.items.iter().filter(|i| i.done).count(),
            "plan_version": pv.plan.as_ref().map(|x| x.version), "spec_version": pv.spec.as_ref().map(|x| x.version),
            "needs_you": o.app.needs_you(p.id).map_err(err)?.len(),
            "run_state": o.app.run_state(p.id).map_err(err)?,
            "inbox": o.app.lead_inbox(p.id).map_err(err)?.len(),
        }));
    }
    Ok(out)
}

#[tauri::command]
pub fn overview(d: State<'_, Desktop>) -> R<Value> {
    let o = d.opened()?;
    let names = names(&o)?;
    let mut needs = vec![];
    let mut current: HashMap<AgentId, (String, TicketKey, String)> = HashMap::new();
    let mut project_names = HashMap::new();
    for p in o.app.projects().map_err(err)? {
        project_names.insert(p.id, p.name.clone());
        for n in o.app.needs_you(p.id).map_err(err)? {
            needs.push(json!({ "project_name": p.name, "item": n }));
        }
        for t in o.app.tickets(p.id).map_err(err)? {
            if let (Some(a), TicketStatus::InProgress | TicketStatus::Pending) =
                (t.assignee, t.status)
            {
                current.insert(a, (p.name.clone(), t.key.clone(), t.title.clone()));
            }
        }
    }
    let agents: Vec<Value> = o
        .app
        .agents()
        .map_err(err)?
        .into_iter()
        .map(|a| {
            let cur = current.get(&a.id);
            json!({ "id": a.id, "name": a.name, "role": a.role, "role_kind": a.role_kind, "status": a.status,
                    "backend": a.backend, "model": a.model,
                    "current": cur.map(|(p, k, t)| json!({ "project": p, "ticket": k, "title": t })) })
        })
        .collect();
    let recent = o.app.store().read(|c| runs::recent(c, 400)).map_err(err)?;
    let today = &now_iso()[..10];
    let mut usage = UsageSummary::default();
    for r in recent.iter().filter(|r| r.started_at.starts_with(today)) {
        usage.add(&r.usage);
    }
    let live: Vec<Value> = recent
        .iter()
        .filter(|r| r.ended_at.is_none())
        .map(|r| run_json(r, &names))
        .collect();
    let last: Vec<Value> = recent
        .iter()
        .take(12)
        .map(|r| run_json(r, &names))
        .collect();
    Ok(json!({
        "needs": needs, "agents": agents, "live_runs": live, "recent_runs": last, "usage_today": usage,
        "runs_today": recent.iter().filter(|r| r.started_at.starts_with(today)).count(),
        "command_approvals": d.approvals.pending(None),
    }))
}

pub(crate) fn run_json(r: &runs::Run, names: &HashMap<AgentId, String>) -> Value {
    json!({ "id": r.id, "ticket": r.ticket, "agent": r.agent, "agent_name": names.get(&r.agent),
            "provider": r.provider, "model": r.model, "started_at": r.started_at, "ended_at": r.ended_at,
            "outcome": r.outcome, "steps": r.steps, "usage": r.usage })
}

#[tauri::command]
pub fn needs_you(d: State<'_, Desktop>, project: String) -> R<Value> {
    let o = d.opened()?;
    let p = pid(&project)?;
    Ok(json!({
        "needs": o.app.needs_you(p).map_err(err)?,
        "command_approvals": d.approvals.pending(Some(p)),
    }))
}

#[tauri::command]
pub fn inbox(d: State<'_, Desktop>, project: String) -> R<Value> {
    let o = d.opened()?;
    Ok(json!(o.app.lead_inbox(pid(&project)?).map_err(err)?))
}

// ---------------------------------------------------------------- tickets

#[tauri::command]
pub fn board(d: State<'_, Desktop>, project: String) -> R<Value> {
    Ok(json!(d.opened()?.app.board(pid(&project)?).map_err(err)?))
}

#[tauri::command]
pub fn plan(d: State<'_, Desktop>, project: String) -> R<Value> {
    Ok(json!(d
        .opened()?
        .app
        .plan_view(pid(&project)?)
        .map_err(err)?))
}

#[tauri::command]
pub fn ticket(d: State<'_, Desktop>, project: String, key: String) -> R<Value> {
    let o = d.opened()?;
    let p = pid(&project)?;
    let k = self::key(&key)?;
    let t = o.app.ticket(p, &k).map_err(err)?;
    let names = names(&o)?;
    let cs = o
        .app
        .store()
        .read(|c| comments::for_ticket(c, t.id))
        .map_err(err)?;
    let rs: Vec<Value> = o
        .app
        .store()
        .read(|c| runs::for_ticket(c, t.id))
        .map_err(err)?
        .iter()
        .map(|r| run_json(r, &names))
        .collect();
    let ws = o
        .app
        .store()
        .read(|c| workspaces::get(c, t.id))
        .map_err(err)?;
    let author = |a: &Author| match a {
        Author::Agent(id) => names.get(id).cloned().unwrap_or_else(|| id.to_string()),
        other => other.to_string(),
    };
    let comments: Vec<Value> = cs
        .iter()
        .map(|c| {
            json!({ "id": c.id, "kind": c.kind, "author": author(&c.author), "reply_to": c.reply_to,
                          "body": c.body, "resolved": c.resolved, "created_at": c.created_at })
        })
        .collect();
    Ok(json!({
        "ticket": t, "assignee_name": t.assignee.and_then(|a| names.get(&a).cloned()),
        "comments": comments, "runs": rs, "workspace": ws,
        "audit": o.app.audit(p, Some(&k)).map_err(err)?,
    }))
}

/// The ticket's changes: the live branch against its base, or what was merged (base..merged commit).
#[tauri::command]
pub fn ticket_diff(d: State<'_, Desktop>, project: String, key: String) -> R<Value> {
    let o = d.opened()?;
    let p = o.app.project(pid(&project)?).map_err(err)?;
    let t = o.app.ticket(p.id, &self::key(&key)?).map_err(err)?;
    let Some(ws) = o
        .app
        .store()
        .read(|c| workspaces::get(c, t.id))
        .map_err(err)?
    else {
        return Ok(json!({ "diff": "", "note": "no worktree yet" }));
    };
    let repo = o.app.repo(&p);
    let diff = if !ws.removed {
        repo.diff(std::path::Path::new(&ws.worktree), &ws.base_sha)
            .map_err(err)?
    } else if let Some(m) = &ws.merged_sha {
        repo.git()
            .check(
                std::path::Path::new(&p.path),
                [
                    "diff",
                    "--no-color",
                    "--no-ext-diff",
                    &format!("{}..{m}", ws.base_sha),
                ],
            )
            .map_err(err)?
    } else {
        String::new()
    };
    const MAX: usize = 400_000;
    let cut = diff.len() > MAX;
    let diff: String = if cut {
        diff.chars().take(MAX).collect()
    } else {
        diff
    };
    Ok(json!({ "diff": diff, "truncated": cut, "branch": ws.branch, "merged": ws.merged_sha }))
}

#[tauri::command]
pub fn audit(
    d: State<'_, Desktop>,
    project: String,
    key: Option<String>,
    format: Option<String>,
) -> R<Value> {
    let o = d.opened()?;
    let k = key.as_deref().map(self::key).transpose()?;
    let entries = o.app.audit(pid(&project)?, k.as_ref()).map_err(err)?;
    Ok(match format.as_deref() {
        Some("md") => json!(agenium_orchestrator::views::audit_markdown(
            &k.map(|k| k.to_string()).unwrap_or_else(|| project.clone()),
            &entries
        )),
        _ => json!(entries),
    })
}

// ---------------------------------------------------------------- agents, teams, runs, events

#[tauri::command]
pub fn agents(d: State<'_, Desktop>) -> R<Vec<Value>> {
    let o = d.opened()?;
    let projects = o.app.projects().map_err(err)?;
    let teams = o.app.teams().map_err(err)?;
    let mut out = vec![];
    for a in o.app.agents().map_err(err)? {
        let planner = o.app.is_planner_role(&a.role);
        let team = a.team.and_then(|t| {
            teams
                .iter()
                .find(|(x, _)| x.id == t)
                .map(|(x, _)| x.clone())
        });
        let project = team.as_ref().and_then(|t| t.project);
        let project_name = project
            .and_then(|p| projects.iter().find(|x| x.id == p))
            .map(|p| p.name.clone());
        out.push(json!({
            "agent": a, "project": project, "project_name": project_name,
            "team": team.as_ref().map(|t| t.name.clone()), "team_id": team.as_ref().map(|t| t.id),
            "solo": team.as_ref().is_some_and(|t| t.solo),
            "is_lead": team.as_ref().is_some_and(|t| t.lead == Some(a.id)),
            "is_planner": planner && team.as_ref().is_some_and(|t| !t.solo),
            "pool": team.is_none(),
        }));
    }
    Ok(out)
}

/// One agent: profile, team role, runs, memory (live facts), skills and MCP bindings.
#[tauri::command]
pub fn agent_detail(d: State<'_, Desktop>, agent: String) -> R<Value> {
    let o = d.opened()?;
    let id: AgentId = agent.parse().map_err(|e: IdParseError| e.to_string())?;
    let a = o.app.agent(id).map_err(err)?;
    let names = names(&o)?;
    let runs: Vec<Value> = o
        .app
        .store()
        .read(|c| runs::for_agent(c, id, 50))
        .map_err(err)?
        .iter()
        .map(|r| run_json(r, &names))
        .collect();
    let mem = o
        .app
        .agent_store(id, o.rt.embedder.dim(), o.rt.embedder.model_id())
        .map_err(err)?;
    let facts = mem.live(500).map_err(err)?;
    let set = o.rt.skills(&a);
    let skills: Vec<Value> = set
        .skills
        .iter()
        .map(|s| json!({ "name": s.meta.name, "description": s.meta.description, "version": s.meta.version, "origin": s.meta.origin }))
        .collect();
    Ok(json!({
        "agent": a, "planner": o.app.is_planner_role(&a.role), "runs": runs, "facts": facts,
        "facts_total": mem.count_live().map_err(err)?, "skills": skills, "mcp": a.mcp,
    }))
}

/// The Memory page: an agent's live facts and lessons (faded ones included) and its skill overlays.
#[tauri::command]
pub fn memory(d: State<'_, Desktop>, agent: String) -> R<Value> {
    let o = d.opened()?;
    let id: AgentId = agent.parse().map_err(|e: IdParseError| e.to_string())?;
    let mem = o
        .app
        .agent_store(id, o.rt.embedder.dim(), o.rt.embedder.model_id())
        .map_err(err)?;
    Ok(json!({
        "facts": mem.live(2000).map_err(err)?,
        "overlays": o.app.skill_overlays(id).map_err(err)?,
        "hide_below": agenium_store::agent::HIDE_BELOW_WEIGHT,
    }))
}

/// Memory actions of the user: `retire` a fact, `revert` or `promote` a skill overlay, `tidy` (decay and
/// consolidation now).
#[tauri::command]
pub fn memory_action(
    d: State<'_, Desktop>,
    agent: String,
    action: String,
    fact: Option<i64>,
    skill: Option<String>,
) -> R<Value> {
    let o = d.opened()?;
    let id: AgentId = agent.parse().map_err(|e: IdParseError| e.to_string())?;
    let mem = o
        .app
        .agent_store(id, o.rt.embedder.dim(), o.rt.embedder.model_id())
        .map_err(err)?;
    let skill = || skill.clone().ok_or_else(|| "skill missing".to_owned());
    match action.as_str() {
        "retire" => mem.retire(fact.ok_or("fact missing")?).map_err(err)?,
        "revert" => o.app.revert_skill_overlay(id, &skill()?).map_err(err)?,
        "promote" => o.app.promote_skill_overlay(id, &skill()?).map_err(err)?,
        "tidy" => {
            let (decayed, folded) = mem.tidy().map_err(err)?;
            return Ok(json!({ "decayed": decayed, "consolidated": folded }));
        }
        other => return Err(format!("unknown memory action {other:?}")),
    }
    Ok(json!({ "ok": true }))
}

#[tauri::command]
pub fn set_team_lead(d: State<'_, Desktop>, project: String, agent: String) -> R<()> {
    let o = d.opened()?;
    let id: AgentId = agent.parse().map_err(|e: IdParseError| e.to_string())?;
    o.app.set_team_lead(pid(&project)?, id).map_err(err)
}

#[tauri::command]
pub fn runs(d: State<'_, Desktop>, limit: Option<usize>) -> R<Vec<Value>> {
    let o = d.opened()?;
    let names = names(&o)?;
    Ok(o.app
        .store()
        .read(|c| runs::recent(c, limit.unwrap_or(100)))
        .map_err(err)?
        .iter()
        .map(|r| run_json(r, &names))
        .collect())
}

#[tauri::command]
pub fn run_detail(d: State<'_, Desktop>, run: String) -> R<Value> {
    let o = d.opened()?;
    let id: RunId = run.parse().map_err(|e: IdParseError| e.to_string())?;
    let names = names(&o)?;
    let (r, log) = o
        .app
        .store()
        .read(|c| Ok((runs::get(c, id)?, runs::log_entries(c, id)?)))
        .map_err(err)?;
    let ticket = match r.ticket {
        Some(t) => Some(
            o.app
                .store()
                .read(|c| agenium_store::core::tickets::get(c, t))
                .map_err(err)?
                .key,
        ),
        None => None,
    };
    Ok(json!({ "run": run_json(&r, &names), "ticket_key": ticket, "log": log }))
}

#[tauri::command]
pub fn events(d: State<'_, Desktop>, since: Option<u64>, limit: Option<usize>) -> R<Value> {
    let o = d.opened()?;
    let limit = limit.unwrap_or(200);
    let since = match since {
        Some(s) => s,
        None => o
            .app
            .store()
            .read(events::last_seq)
            .map_err(err)?
            .saturating_sub(limit as u64),
    };
    Ok(json!(o
        .app
        .store()
        .read(|c| events::since(c, since, limit))
        .map_err(err)?))
}

/// The agent CLIs on this machine, detected at most every 5 minutes (or now with `refresh`).
pub async fn detected_clis(d: &Desktop, refresh: bool) -> R<Vec<agenium_cli_backend::CliInfo>> {
    let cached = d
        .clis
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .as_ref()
        .filter(|(at, _)| at.elapsed() < std::time::Duration::from_secs(300))
        .map(|(_, v)| v.clone());
    if let Some(v) = cached.filter(|_| !refresh) {
        return Ok(v);
    }
    let found = tokio::task::spawn_blocking(agenium_cli_backend::detect::detect_all)
        .await
        .map_err(|e| e.to_string())?;
    *d.clis.lock().unwrap_or_else(|p| p.into_inner()) =
        Some((std::time::Instant::now(), found.clone()));
    Ok(found)
}

/// The vendors' subscription usage for the footer (last readings; see `agenium_orchestrator::vendor_usage`).
#[tauri::command]
pub async fn vendor_usage(d: State<'_, Desktop>) -> R<Value> {
    let o = d.opened()?;
    let clis = detected_clis(&d, false).await?;
    Ok(json!(o.app.vendor_usage(&clis)))
}

/// Reads the vendors now (the footer's refresh).
#[tauri::command]
pub async fn refresh_vendor_usage(d: State<'_, Desktop>) -> R<Value> {
    let o = d.opened()?;
    let clis = detected_clis(&d, false).await?;
    agenium_orchestrator::vendor_usage::refresh_vendor_usage(&o.app, &clis)
        .await
        .map_err(err)?;
    Ok(json!(o.app.vendor_usage(&clis)))
}

/// Reads Claude's usage now (one tiny Haiku turn, on the user's click).
#[tauri::command]
pub async fn read_claude_usage(d: State<'_, Desktop>) -> R<Value> {
    let o = d.opened()?;
    let clis = detected_clis(&d, false).await?;
    agenium_orchestrator::vendor_usage::read_claude_usage(&o.app, &clis)
        .await
        .map_err(err)?;
    Ok(json!(o.app.vendor_usage(&clis)))
}

/// Providers, agent CLIs (detected at most every 5 minutes, or now with `refresh`), templates, CLI fallback.
#[tauri::command]
pub async fn capabilities(d: State<'_, Desktop>, refresh: Option<bool>) -> R<Value> {
    let o = d.opened()?;
    let cfg = o.app.config();
    let providers: Vec<Value> = cfg
        .providers
        .iter()
        .map(|p| json!({ "id": p.id, "kind": p.kind, "base_url": p.base_url, "has_key": !p.api_key_ref.is_empty(),
                          "models": p.models, "max_concurrency": p.max_concurrency }))
        .collect();
    let clis = detected_clis(&d, refresh == Some(true)).await?;
    Ok(json!({
        "providers": providers, "clis": clis,
        "cli_models": { "opencode": o.app.opencode_free_models() },
        "templates": o.app.team_templates().map_err(err)?,
        "cli_fallback": { "provider": cfg.cli.fallback_provider, "model": cfg.cli.fallback_model },
    }))
}

#[tauri::command]
pub fn templates(d: State<'_, Desktop>) -> R<Value> {
    let app = &d.opened()?.app;
    let ts = app.team_templates().map_err(err)?;
    // Each template with where it comes from: built in, saved by the user, or a user copy of a built-in.
    Ok(json!(ts
        .into_iter()
        .map(|t| {
            let (builtin, custom) = app.template_origin(&t.id);
            let mut v = json!(t);
            v["builtin"] = json!(builtin);
            v["custom"] = json!(custom);
            v
        })
        .collect::<Vec<_>>()))
}

/// The template wizard: saves a team template (validated) under the root.
#[tauri::command]
pub fn save_template(d: State<'_, Desktop>, template: Value) -> R<Value> {
    let mut template = template;
    if let Some(o) = template.as_object_mut() {
        o.remove("builtin");
        o.remove("custom");
    }
    let t: agenium_orchestrator::team_templates::TeamTemplate =
        serde_json::from_value(template).map_err(|e| e.to_string())?;
    Ok(json!(d
        .opened()?
        .app
        .save_team_template(&t)
        .map_err(err)?))
}

/// Removes a user template (a built-in it overrode comes back).
#[tauri::command]
pub fn remove_template(d: State<'_, Desktop>, id: String) -> R<()> {
    d.opened()?.app.remove_team_template(&id).map_err(err)
}

/// A template drafted from a team as it is (not saved yet).
#[tauri::command]
pub fn template_from_team(
    d: State<'_, Desktop>,
    team: String,
    id: String,
    name: String,
) -> R<Value> {
    let team: TeamId = team.parse().map_err(|e: IdParseError| e.to_string())?;
    Ok(json!(d
        .opened()?
        .app
        .template_from_team(team, &id, &name)
        .map_err(err)?))
}

/// A template as TOML (export), or a TOML text checked and saved (import).
#[tauri::command]
pub fn template_toml(d: State<'_, Desktop>, id: Option<String>, text: Option<String>) -> R<Value> {
    let app = &d.opened()?.app;
    match (id, text) {
        (_, Some(text)) => {
            let t =
                agenium_orchestrator::team_templates::TeamTemplate::parse(&text).map_err(err)?;
            Ok(json!(app.save_team_template(&t).map_err(err)?))
        }
        (Some(id), None) => Ok(json!(app.team_template(&id).map_err(err)?.to_toml())),
        (None, None) => Err("give a template id or a TOML text".into()),
    }
}

// ---------------------------------------------------------------- actions

fn user(o: &Opened, project: ProjectId, intent: Intent) -> R<Value> {
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
        json!({ "run": s.run, "outcome": s.outcome, "text": s.text, "steps": s.steps, "usage": s.usage }),
    )
}

#[tauri::command]
pub fn chat_history(d: State<'_, Desktop>, project: String) -> R<Value> {
    let o = d.opened()?;
    let p = pid(&project)?;
    let Some((team, _)) = o.app.team_of_project(p).map_err(err)? else {
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
    // User turns carry the project state after the request: show only what the user wrote.
    let out: Vec<Value> = msgs
        .into_iter()
        .filter_map(|m| serde_json::from_value::<agenium_llm::Message>(m).ok())
        .filter_map(|m| {
            let text = m.text();
            let text = text
                .split("\n\n<project_state>")
                .next()
                .unwrap_or_default()
                .trim()
                .to_owned();
            (!text.is_empty()).then(|| json!({ "role": m.role, "text": text }))
        })
        .collect();
    Ok(json!(out))
}

#[tauri::command]
pub fn approve(d: State<'_, Desktop>, project: String, what: String, target: String) -> R<()> {
    let o = d.opened()?;
    let p = pid(&project)?;
    match what.as_str() {
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
    }
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
    let o = d.opened()?;
    let q: CommentId = question.parse().map_err(|e: IdParseError| e.to_string())?;
    Ok(o.app
        .user_answer(pid(&project)?, &key(&ticket)?, q, &answer, option)
        .map_err(err)?
        .result)
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
pub fn ticket_action(
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
    match action.as_str() {
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
            }),
        ),
        other => Err(format!("unknown ticket action {other}")),
    }
}

/// `source`: a folder name under the app's projects folder, or a git repository to clone there.
#[tauri::command]
pub async fn create_project(
    d: State<'_, Desktop>,
    name: String,
    source: Option<String>,
) -> R<Value> {
    let o = d.opened()?;
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
        key_prefix: None,
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

fn binding(provider: &str, model: &str, effort: &str) -> R<ModelBinding> {
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
pub fn create_team(
    d: State<'_, Desktop>,
    project: String,
    template: String,
    model: ModelArg,
    lead_model: Option<ModelArg>,
    planner_model: Option<ModelArg>,
    prefix: Option<String>,
) -> R<Value> {
    let o = d.opened()?;
    let p = pid(&project)?;
    let b = |m: &ModelArg| binding(&m.provider, &m.model, &m.effort);
    let out = o
        .app
        .create_team_from_template(
            p,
            &template,
            &b(&model)?,
            lead_model.as_ref().map(b).transpose()?.as_ref(),
            planner_model.as_ref().map(b).transpose()?.as_ref(),
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
    let rt = o.rt.clone();
    rt.cancel.cancel();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        rt.cancel.reset();
    });
    Ok(())
}
