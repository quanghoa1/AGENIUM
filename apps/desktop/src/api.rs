//! IPC commands of the desktop app. Read models come from the orchestrator (`views`, `inbox`, store queries); every
//! action goes through the same App/Runtime methods as the CLI, so the rules are the orchestrator's, never the UI's.

use std::collections::{BTreeMap, HashMap};

use agenium_core::forms::{self, PermissionDecision};
use agenium_core::*;
use agenium_orchestrator::app::{CreateAgent, CreateProject};
use agenium_orchestrator::runtime::Approval;
use agenium_orchestrator::CallCtx;
use agenium_store::core::{comments, events, runs, tickets, workspaces};
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
pub fn project_report(d: State<'_, Desktop>, project: String) -> R<String> {
    d.opened()?
        .app
        .project_report_md(pid(&project)?)
        .map_err(err)
}

#[tauri::command]
pub async fn support_bundle(d: State<'_, Desktop>, out: String) -> R<String> {
    let o = d.opened()?;
    let dir = std::path::PathBuf::from(if out.trim().is_empty() {
        std::env::temp_dir().display().to_string()
    } else {
        out
    });
    blocking(move || {
        Ok(o.app
            .support_bundle(&dir)
            .map_err(err)?
            .display()
            .to_string())
    })
    .await
}

#[tauri::command]
pub fn status(d: State<'_, Desktop>) -> Value {
    let opened = d.opened().ok();
    let cfg = opened.as_ref().map(|o| o.app.config());
    json!({
        "root": d.root.display().to_string(),
        "ready": opened.is_some(),
        "version": env!("CARGO_PKG_VERSION"),
        "ui_language": cfg.as_ref().map(|c| c.app.language.clone()),
        "model": d.model_prep(),
    })
}

/// How agent shell commands are approved (ADR-64) and which sandbox backend works on this machine.
#[tauri::command]
pub fn command_policy(d: State<'_, Desktop>) -> R<Value> {
    let o = d.opened()?;
    let sb = agenium_sandbox::detect();
    Ok(
        json!({ "approval": o.app.config().sandbox.approval, "unsandboxed": o.app.config().sandbox.unsandboxed,
                "backend": sb.backend, "reason": sb.reason }),
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

/// Whether agents may run their commands without a sandbox on a machine that has none (ADR-117).
#[tauri::command]
pub fn set_unsandboxed(d: State<'_, Desktop>, on: bool) -> R<Value> {
    d.opened()?
        .app
        .update_config(|c| c.sandbox.unsandboxed = on)
        .map_err(err)?;
    command_policy(d)
}

#[tauri::command]
pub fn setup(d: State<'_, Desktop>, handle: AppHandle, ui_language: String) -> R<Value> {
    // A second click (or a second window) while the root is open: one App and one runtime per root.
    if d.opened().is_ok() {
        return Ok(status(d));
    }
    if d.model_prep().phase != "ready" {
        return Err("the embedding model is not in place yet".into());
    }
    let input = agenium_config::setup::SetupInput {
        root: Some(d.root.clone()),
        ui_language,
    };
    let home = dirs::home_dir().unwrap_or_default().join(".agenium");
    // A root given by AGENIUM_HOME is this process's override: nothing is remembered machine-wide.
    let by_env = std::env::var_os(agenium_config::root::ENV_HOME).is_some_and(|v| !v.is_empty());
    let bootstrap = agenium_config::Bootstrap::default_path().filter(|_| !by_env);
    let fresh = agenium_config::setup::needs_setup(&d.root);
    agenium_config::setup::complete_setup(&input, &home, bootstrap.as_deref()).map_err(err)?;
    // A root set up for a UI test (a throwaway one, under AGENIUM_HOME) is marked as such: it alone opens with the
    // in-memory vault key in a release build.
    if fresh && by_env && crate::state::memory_keyring_asked() {
        std::fs::write(d.root.join(crate::state::TEST_ROOT_MARK), "").map_err(err)?;
    }
    d.open(&handle)?;
    Ok(status(d))
}

// ---------------------------------------------------------------- overview

#[tauri::command]
pub fn projects(d: State<'_, Desktop>) -> R<Vec<Value>> {
    let o = d.opened()?;
    let mut out = vec![];
    for p in o.app.projects().map_err(err)? {
        // One project that cannot be read is listed with its error; the others stay usable.
        match project_summary(&o, &p) {
            Ok(v) => out.push(v),
            Err(e) => out.push(json!({
                "id": p.id, "name": p.name, "path": p.path, "key_prefix": p.key_prefix, "status": p.status,
                "main_branch": p.main_branch, "team": null, "tickets": 0, "by_status": {}, "plan_items": 0,
                "plan_done": 0, "needs_you": 0, "run_state": "Paused", "inbox": 0, "error": e,
            })),
        }
    }
    Ok(out)
}

fn project_summary(o: &Opened, p: &agenium_store::core::projects::Project) -> R<Value> {
    {
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
        Ok(json!({
            "id": p.id, "name": p.name, "path": p.path, "key_prefix": p.key_prefix, "status": p.status,
            "main_branch": p.main_branch, "team": team, "tickets": tickets.len(), "by_status": by_status,
            "plan_items": pv.items.len(), "plan_done": pv.items.iter().filter(|i| i.done).count(),
            "plan_version": pv.plan.as_ref().map(|x| x.version), "spec_version": pv.spec.as_ref().map(|x| x.version),
            "needs_you": o.app.needs_you(p.id).map_err(err)?.len(),
            "run_state": o.app.run_state(p.id).map_err(err)?,
            "inbox": o.app.lead_inbox(p.id).map_err(err)?.len(),
        }))
    }
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
    let today = runs_today(&o)?;
    let mut usage = UsageSummary::default();
    for r in &today {
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
        "runs_today": today.len(),
        "command_approvals": d.approvals.pending(None),
    }))
}

/// Whether a run went through a provider's API (its key), not through a vendor CLI: only then is the prompt cache
/// Agenium's to show (a CLI caches its own prompt; ADR-113).
fn api_run(r: &runs::Run) -> bool {
    !agenium_cli_backend::CliKind::ALL
        .iter()
        .any(|k| k.program() == r.provider)
}

/// Share of an API run's input tokens served from the provider's prompt cache (`None` for CLI runs or no input yet).
fn cache_hit(r: &runs::Run) -> Option<f64> {
    (api_run(r) && r.usage.tokens_in > 0)
        .then(|| r.usage.cache_read as f64 / r.usage.tokens_in as f64)
}

/// Today's runs: started since midnight on the user's clock (UTC's day turns at 07:00 in Vietnam), all of them (a
/// busy day has more than the few hundred newest runs the lists read).
pub(crate) fn runs_today(o: &Opened) -> R<Vec<runs::Run>> {
    let midnight = chrono::Local::now()
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .and_then(|t| t.and_local_timezone(chrono::Local).earliest())
        .map(|t| t.with_timezone(&chrono::Utc))
        .unwrap_or_else(chrono::Utc::now)
        .format("%Y-%m-%dT%H:%M:%S")
        .to_string();
    let mut n = 500;
    loop {
        let rs = o.app.store().read(|c| runs::recent(c, n)).map_err(err)?;
        let reaches_yesterday = rs.last().is_some_and(|r| r.started_at < midnight);
        if rs.len() < n || reaches_yesterday || n >= 50_000 {
            return Ok(rs
                .into_iter()
                .filter(|r| r.started_at >= midnight)
                .collect());
        }
        n *= 4;
    }
}

pub(crate) fn run_json(r: &runs::Run, names: &HashMap<AgentId, String>) -> Value {
    json!({ "id": r.id, "ticket": r.ticket, "agent": r.agent, "agent_name": names.get(&r.agent),
            "provider": r.provider, "model": r.model, "started_at": r.started_at, "ended_at": r.ended_at,
            "outcome": r.outcome, "steps": r.steps, "usage": r.usage,
            "api": api_run(r), "cache_hit": cache_hit(r) })
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
    // Links to other tickets (related, aggregates...), each with the other ticket's key and title and its direction.
    let all = o.app.tickets(p).map_err(err)?;
    // The tickets that wait for this one (their depends_on names it): the other side of "depends on".
    let dependents: Vec<&TicketKey> = all
        .iter()
        .filter(|x| x.depends_on.contains(&t.key))
        .map(|x| &x.key)
        .collect();
    let dependents = json!(dependents);
    let others: HashMap<_, _> = all.into_iter().map(|x| (x.id, (x.key, x.title))).collect();
    let links: Vec<Value> = o
        .app
        .store()
        .read(|c| tickets::links_of(c, t.id))
        .map_err(err)?
        .into_iter()
        .filter_map(|l| {
            let out = l.src == t.id;
            let (key, title) = others.get(if out { &l.dst } else { &l.src })?;
            Some(json!({ "kind": l.kind, "out": out, "key": key, "title": title,
                         "proposed_by": l.proposed_by, "confirmed": l.confirmed_by.is_some(), "note": l.note }))
        })
        .collect();
    Ok(json!({
        "ticket": t, "assignee_name": t.assignee.and_then(|a| names.get(&a).cloned()),
        "comments": comments, "runs": rs, "workspace": ws, "links": links, "dependents": dependents,
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

/// One changed file of a ticket, before and after (for a side-by-side diff): the content at the ticket's base, and
/// the worktree's file (or, once merged, the merge's). `null` = the file does not exist on that side.
#[tauri::command]
pub fn ticket_file(d: State<'_, Desktop>, project: String, key: String, path: String) -> R<Value> {
    const MAX: usize = 2_000_000;
    let o = d.opened()?;
    let p = o.app.project(pid(&project)?).map_err(err)?;
    let t = o.app.ticket(p.id, &self::key(&key)?).map_err(err)?;
    let rel = std::path::Path::new(&path);
    if path.is_empty()
        || rel.is_absolute()
        || rel
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
    {
        return Err(format!("{path:?} is not a path inside the project"));
    }
    let ws = o
        .app
        .store()
        .read(|c| workspaces::get(c, t.id))
        .map_err(err)?
        .ok_or("no worktree yet")?;
    let repo = o.app.repo(&p);
    let root = std::path::Path::new(&p.path);
    let at = |rev: &str| -> Option<String> {
        repo.git()
            .check(
                root,
                ["show", &format!("{rev}:{}", path.replace('\\', "/"))],
            )
            .ok()
    };
    let original = at(&ws.base_sha);
    let modified = if !ws.removed {
        std::fs::read(std::path::Path::new(&ws.worktree).join(rel))
            .ok()
            .map(|b| String::from_utf8_lossy(&b).into_owned())
    } else {
        ws.merged_sha.as_deref().and_then(at)
    };
    let binary = [&original, &modified]
        .iter()
        .any(|s| s.as_deref().is_some_and(|s| s.contains('\0')));
    let big = [&original, &modified]
        .iter()
        .any(|s| s.as_ref().is_some_and(|s| s.len() > MAX));
    if binary || big {
        return Ok(json!({ "original": null, "modified": null, "binary": binary, "too_big": big }));
    }
    Ok(json!({ "original": original, "modified": modified, "binary": false, "too_big": false }))
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
        // What the agent works with, counted for its card (names are on its page): every skill it uses (library,
        // role, its own), those it learned itself (its own skills and its own versions of shared ones), MCP servers.
        let set = o.rt.skills(&a);
        let learned = set
            .skills
            .iter()
            .filter(|s| {
                matches!(
                    s.meta.origin,
                    agenium_skills::Origin::Agent | agenium_skills::Origin::Overlay
                )
            })
            .count();
        // The fields the agent's skills cover (their tags, deduplicated): pentest, coding, research, finance…
        let mut domains: Vec<String> = Vec::new();
        for s in &set.skills {
            for t in &s.meta.tags {
                if !domains.contains(t) {
                    domains.push(t.clone());
                }
            }
        }
        out.push(json!({
            "skill_count": set.skills.len(), "learned_skills": learned, "mcp_count": a.mcp.as_array().map_or(0, Vec::len),
            "skill_domains": domains,
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
    let recent = o
        .app
        .store()
        .read(|c| runs::for_agent(c, id, 50))
        .map_err(err)?;
    // The prompt cache over the agent's recent API runs (its CLI runs are not counted).
    let (cached, input, api_runs) = recent
        .iter()
        .filter(|r| api_run(r))
        .fold((0u64, 0u64, 0usize), |(c, i, n), r| {
            (c + r.usage.cache_read, i + r.usage.tokens_in, n + 1)
        });
    let cache = (input > 0).then(|| {
        json!({ "hit": cached as f64 / input as f64, "runs": api_runs, "input": input, "cached": cached })
    });
    let runs: Vec<Value> = recent.iter().map(|r| run_json(r, &names)).collect();
    let mem = o
        .app
        .agent_store(id, o.rt.embedder.dim(), o.rt.embedder.model_id())
        .map_err(err)?;
    let facts = mem.live(500).map_err(err)?;
    let set = o.rt.skills(&a);
    let skills: Vec<Value> = set
        .skills
        .iter()
        .map(|s| json!({ "name": s.meta.name, "description": s.meta.description, "version": s.meta.version, "tags": s.meta.tags, "origin": s.meta.origin }))
        .collect();
    Ok(json!({
        "agent": a, "planner": o.app.is_planner_role(&a.role), "runs": runs, "facts": facts,
        "facts_total": mem.count_live().map_err(err)?, "skills": skills, "mcp": a.mcp,
        "cache": cache,
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
        // ADR-91: skills the agent created itself, with the lesson behind each and whether it was flagged.
        "created": o.app.created_skills(id).map_err(err)?,
        "hide_below": agenium_store::agent::HIDE_BELOW_WEIGHT,
    }))
}

/// Memory actions of the user: `retire` a fact, `remove_skill` (a skill the agent created), `revert` or `promote` a
/// skill overlay, `rebase`, `tidy` (decay and consolidation now).
#[tauri::command]
pub async fn memory_action(
    d: State<'_, Desktop>,
    agent: String,
    action: String,
    fact: Option<i64>,
    skill: Option<String>,
) -> R<Value> {
    let o = d.opened()?;
    blocking(move || memory_action_now(&o, agent, action, fact, skill)).await
}

fn memory_action_now(
    o: &Opened,
    agent: String,
    action: String,
    fact: Option<i64>,
    skill: Option<String>,
) -> R<Value> {
    let id: AgentId = agent.parse().map_err(|e: IdParseError| e.to_string())?;
    let mem = o
        .app
        .agent_store(id, o.rt.embedder.dim(), o.rt.embedder.model_id())
        .map_err(err)?;
    let skill = || skill.clone().ok_or_else(|| "skill missing".to_owned());
    match action.as_str() {
        "retire" => {
            mem.retire(fact.ok_or("fact missing")?).map_err(err)?;
            // ADR-91: a skill built on a retired lesson is flagged for review right away.
            o.app.audit_created_skills(id).map_err(err)?;
        }
        "remove_skill" => o.app.remove_created_skill(id, &skill()?).map_err(err)?,
        "revert" => o.app.revert_skill_overlay(id, &skill()?).map_err(err)?,
        "promote" => o.app.promote_skill_overlay(id, &skill()?).map_err(err)?,
        "rebase" => {
            let outcomes = o.app.rebase_skill_overlays(id).map_err(err)?;
            return Ok(json!({ "rebased": outcomes }));
        }
        "tidy" => {
            let (decayed, folded) = mem.tidy().map_err(err)?;
            return Ok(json!({ "decayed": decayed, "consolidated": folded }));
        }
        other => return Err(format!("unknown memory action {other:?}")),
    }
    Ok(json!({ "ok": true }))
}

#[tauri::command]
pub fn runs(d: State<'_, Desktop>, limit: Option<usize>) -> R<Vec<Value>> {
    let o = d.opened()?;
    let names = names(&o)?;
    // Each run's ticket key and project, so the list says what a run was about (one ticket list per project).
    let mut keys: HashMap<TicketId, (TicketKey, String)> = HashMap::new();
    for p in o.app.projects().map_err(err)? {
        for t in o.app.tickets(p.id).map_err(err)? {
            keys.insert(t.id, (t.key, p.name.clone()));
        }
    }
    Ok(o.app
        .store()
        .read(|c| runs::recent(c, limit.unwrap_or(100)))
        .map_err(err)?
        .iter()
        .map(|r| {
            let mut v = run_json(r, &names);
            if let Some((k, p)) = r.ticket.and_then(|t| keys.get(&t)) {
                v["ticket_key"] = json!(k);
                v["project_name"] = json!(p);
            }
            v
        })
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

/// A CLI with no login-status command (agy, grok) that listed its models from its vendor's server is signed in.
fn signed_in_by_models(
    clis: &mut [agenium_cli_backend::CliInfo],
    models: &crate::state::CliModels,
) {
    use agenium_cli_backend::detect::{CliKind, Login};
    for c in clis.iter_mut() {
        let listed = models.get(c.kind.program()).is_some_and(|m| !m.is_empty());
        if c.login == Login::Unknown && listed && matches!(c.kind, CliKind::Agy | CliKind::Grok) {
            c.login = Login::LoggedIn;
        }
    }
}

/// Finds the agent CLIs and their models in the background once the root is open, so the first look at the
/// providers page or the agent form does not wait for them (about 8 s cold, every CLI started twice).
pub(crate) async fn warm_cli_caches(handle: tauri::AppHandle) {
    use tauri::Manager;
    for _ in 0..600 {
        let d = handle.state::<Desktop>();
        if let Ok(o) = d.opened() {
            match detected_clis(&d, false).await {
                Ok(clis) => {
                    cli_models(&d, &o, &clis, false).await;
                }
                Err(e) => tracing::warn!(error = %e, "the agent CLIs were not detected in advance"),
            }
            return;
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
}

/// Models each driven CLI can run (`model/list`, `agy models`, ...), listed at most every 5 minutes.
async fn cli_models(
    d: &Desktop,
    o: &crate::state::Opened,
    clis: &[agenium_cli_backend::CliInfo],
    refresh: bool,
) -> crate::state::CliModels {
    let cached = d
        .cli_models
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .as_ref()
        .filter(|(at, _)| at.elapsed() < std::time::Duration::from_secs(300))
        .map(|(_, v)| v.clone());
    if let Some(v) = cached.filter(|_| !refresh) {
        return v;
    }
    let found = agenium_orchestrator::model_lists::cli_models(&o.app, clis).await;
    *d.cli_models.lock().unwrap_or_else(|p| p.into_inner()) =
        Some((std::time::Instant::now(), found.clone()));
    found
}

/// The judge's state: configured, enabled (not disabled by a failure), and why if disabled (ADR-81).
#[tauri::command]
pub fn judge_status(d: State<'_, Desktop>) -> R<Value> {
    let o = d.opened()?;
    let h = o.app.judge_health();
    Ok(
        json!({ "configured": o.app.config().judge.enabled(), "disabled": h.disabled, "reason": h.reason }),
    )
}

/// The judge's settings for the form (never the key: only whether one is stored).
#[tauri::command]
pub fn judge_config(d: State<'_, Desktop>) -> R<Value> {
    let o = d.opened()?;
    let j = o.app.config().judge;
    let has_key = !j.api_key_ref.is_empty() && o.app.secret(&j.api_key_ref).map_err(err)?.is_some();
    let h = o.app.judge_health();
    Ok(json!({
        "base_url": j.base_url, "model": j.model, "has_key": has_key,
        "review_assist": j.review_assist, "skill_hint": j.skill_hint, "memory_gate": j.memory_gate,
        "skill_quality": j.skill_quality, "done_check": j.done_check,
        "configured": j.enabled(), "disabled": h.disabled, "reason": h.reason,
    }))
}

/// The interface language chosen in Settings, kept in the config too (desktop notifications use it).
#[tauri::command]
pub fn set_ui_language(d: State<'_, Desktop>, language: String) -> R<()> {
    if !matches!(language.as_str(), "en" | "vi") {
        return Err(format!("unknown language {language:?}"));
    }
    d.opened()?
        .app
        .update_config(|c| c.app.language = language)
        .map(|_| ())
        .map_err(err)
}

/// What the judge was asked lately, newest first (the Settings page shows it).
#[tauri::command]
pub fn judge_activity(d: State<'_, Desktop>) -> R<Value> {
    Ok(json!(d.opened()?.app.judge_activity(40)))
}

/// Saves the judge's settings (the key to the vault).
#[tauri::command]
pub fn set_judge(d: State<'_, Desktop>, form: agenium_orchestrator::judge::JudgeInput) -> R<()> {
    d.opened()?.app.set_judge(&form).map(|_| ()).map_err(err)
}

/// Re-enables the judge and checks it (after the user re-entered the key).
#[tauri::command]
pub async fn enable_judge(d: State<'_, Desktop>) -> R<()> {
    d.opened()?.app.enable_judge().await.map_err(err)
}

/// Exports an agent (memory, skills and overlays, skill/MCP enablement; no secret values) to a file in `out`.
#[tauri::command]
pub async fn export_agent(d: State<'_, Desktop>, agent: String, out: String) -> R<String> {
    let o = d.opened()?;
    blocking(move || export_agent_now(&o, agent, out)).await
}

fn export_agent_now(o: &Opened, agent: String, out: String) -> R<String> {
    let id: AgentId = agent.parse().map_err(|e: IdParseError| e.to_string())?;
    let name = o.app.agent(id).map_err(err)?.name;
    let dir = std::path::PathBuf::from(if out.trim().is_empty() {
        std::env::temp_dir().display().to_string()
    } else {
        out
    });
    let file = dir.join(format!("{name}.agenium.jsonl"));
    o.app
        .export_agent(id, &file, o.rt.embedder.as_ref(), &Default::default())
        .map_err(err)?;
    Ok(file.display().to_string())
}

/// Imports an agent from a file into `project` as a Solo agent (renamed if `name` is given).
#[tauri::command]
pub async fn import_agent(
    d: State<'_, Desktop>,
    file: String,
    project: String,
    name: Option<String>,
) -> R<Value> {
    let o = d.opened()?;
    blocking(move || import_agent_now(&o, file, project, name)).await
}

fn import_agent_now(o: &Opened, file: String, project: String, name: Option<String>) -> R<Value> {
    let a = o
        .app
        .import_agent(
            std::path::Path::new(&file),
            &agenium_orchestrator::agent_io::ImportRequest {
                project: pid(&project)?,
                name,
            },
            o.rt.embedder.as_ref(),
        )
        .map_err(err)?;
    Ok(json!({ "id": a.id, "name": a.name }))
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
        .map(|p| {
            // Headers for the form: a secret one by name only (its value stays in the vault).
            let headers: Vec<Value> = p
                .headers
                .iter()
                .map(|(k, v)| {
                    let secret = v.starts_with("${secret:");
                    json!({ "name": k, "secret": secret, "value": if secret { "" } else { v.as_str() } })
                })
                .collect();
            json!({ "id": p.id, "kind": p.kind, "base_url": p.base_url, "has_key": !p.api_key_ref.is_empty(),
                    "models": p.models.iter().filter(|m| o.app.api_serves(&p.id, m)).collect::<Vec<_>>(),
                    "max_concurrency": p.max_concurrency, "headers": headers, "model_caps": p.model_caps,
                    "body": p.body, "effort_field": p.effort_field })
        })
        .collect();
    let mut clis = detected_clis(&d, refresh == Some(true)).await?;
    let models = cli_models(&d, &o, &clis, refresh == Some(true)).await;
    signed_in_by_models(&mut clis, &models);
    Ok(json!({
        "providers": providers, "clis": clis,
        "cli_models": models,
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
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> R<T> + Send + 'static) -> R<T> {
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
        // In a team the answer is a message to the Lead (ADR-122); its turn runs in the background.
        UserAnswer::ToLead(message) => {
            let rt = o.rt.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = rt.chat(p, &message).await {
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
    let (model_backend, model_b) = model.binding()?;
    let lead = lead_model.as_ref().map(ModelArg::binding).transpose()?;
    let planner = planner_model.as_ref().map(ModelArg::binding).transpose()?;
    let out = o
        .app
        .create_team_from_template(
            p,
            &template,
            &model_b,
            lead.as_ref().map(|(_, b)| b),
            planner.as_ref().map(|(_, b)| b),
            prefix.as_deref().unwrap_or(""),
        )
        .map_err(err)?;
    // A model picked from the agent CLIs runs on that CLI: each member gets the backend of the model it was given
    // (the Lead's, the planner's, or the team's).
    let members = o
        .app
        .store()
        .read(|c| agenium_store::core::agents::agents_of_team(c, out.team))
        .map_err(err)?;
    for a in members {
        let backend = match (&lead, &planner) {
            (Some((k, b)), _) if a.role_kind == RoleKind::Lead && a.model == *b => *k,
            (_, Some((k, b))) if a.role_kind == RoleKind::Worker && a.model == *b => *k,
            _ => model_backend,
        };
        if backend == BackendKind::Cli {
            o.app
                .set_agent_backend(a.id, BackendKind::Cli, &a.model)
                .map_err(err)?;
        }
    }
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
