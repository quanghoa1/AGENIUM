//! The Live tab: what every agent is doing now (team, current run, last tool call, open tickets), a worker's
//! internal message bus (assignments from the Lead, questions, answers, reports, reviews) and the trace of an agent's
//! current or last run. Read-only: talking to a team goes through its Lead (workers are watched, not chatted with).

use std::collections::HashMap;

use agenium_core::*;
use agenium_store::core::{comments, runs, tickets};
use serde_json::{json, Value};
use tauri::State;

use crate::api::{names, run_json};
use crate::state::{err, Desktop, Opened, R};

fn aid(s: &str) -> R<AgentId> {
    s.parse().map_err(|e: IdParseError| e.to_string())
}

/// The tool calls the shell view shows as terminal blocks (API `shell`, and the CLIs' own shells).
const SHELL: [&str; 5] = [
    "shell",
    "cli:Bash",
    "cli:PowerShell",
    "cli:run_command",
    "cli:shell",
];

fn ticket_key(o: &Opened, t: Option<TicketId>) -> Option<TicketKey> {
    let t = t?;
    o.app
        .store()
        .read(|c| tickets::find(c, t))
        .ok()
        .flatten()
        .map(|t| t.key)
}

/// What kind of run this is for the agent: its own ticket (work), a ticket it reviews as Lead (review), or Lead
/// work without a ticket (chat, inbox, plan).
fn run_kind(o: &Opened, r: &runs::Run) -> &'static str {
    match r.ticket {
        Some(t) => match o.app.store().read(|c| tickets::find(c, t)).ok().flatten() {
            Some(t) if t.assignee == Some(r.agent) => "work",
            Some(_) => "review",
            None => "work",
        },
        None => "lead",
    }
}

/// Every agent now: team and role in it, live run (with its last tool call), open tickets; teams; totals.
#[tauri::command]
pub fn live(d: State<'_, Desktop>) -> R<Value> {
    let o = d.opened()?;
    let projects = o.app.projects().map_err(err)?;
    let teams = o.app.teams().map_err(err)?;
    let unfinished = o.app.store().read(runs::unfinished).map_err(err)?;
    let names = names(&o)?;
    let today: String = now_iso().chars().take(10).collect();
    let recent = o.app.store().read(|c| runs::recent(c, 500)).map_err(err)?;
    let tokens_today: u64 = recent
        .iter()
        .filter(|r| r.started_at.starts_with(&today))
        .map(|r| r.usage.tokens_in + r.usage.tokens_out)
        .sum();
    let mut by_agent: HashMap<AgentId, &runs::Run> = HashMap::new();
    for r in &unfinished {
        by_agent.insert(r.agent, r);
    }
    let mut agents = Vec::new();
    for a in o.app.agents().map_err(err)? {
        let team = a
            .team
            .and_then(|t| teams.iter().find(|(x, _)| x.id == t).map(|(x, _)| x));
        let project = team.and_then(|t| t.project);
        let run = by_agent.get(&a.id).map(|r| {
            let log = o
                .app
                .store()
                .read(|c| runs::log_entries(c, r.id))
                .unwrap_or_default();
            let last_tool = log.iter().rev().find(|e| e.kind == "tool").map(|e| {
                json!({ "tool": e.data["tool"], "args": e.data["args"], "ok": e.data["ok"], "at": e.at,
                        "ms": e.data["ms"] })
            });
            json!({ "run": run_json(r, &names), "kind": run_kind(&o, r), "ticket_key": ticket_key(&o, r.ticket),
                    "last_tool": last_tool, "tools": log.iter().filter(|e| e.kind == "tool").count() })
        });
        let open: Vec<Value> = o
            .app
            .store()
            .read(|c| tickets::open_for_agent(c, a.id))
            .map_err(err)?
            .into_iter()
            .map(|t| json!({ "key": t.key, "title": t.title, "status": t.status, "project": t.project }))
            .collect();
        agents.push(json!({
            "agent": a,
            "team": team.map(|t| t.id),
            "team_name": team.map(|t| t.name.clone()),
            "solo": team.is_some_and(|t| t.solo),
            "is_lead": team.is_some_and(|t| t.lead == Some(a.id)),
            "is_planner": team.is_some_and(|t| !t.solo) && o.app.is_planner_role(&a.role),
            "project": project,
            "project_name": project.and_then(|p| projects.iter().find(|x| x.id == p)).map(|p| p.name.clone()),
            "run": run,
            "open_tickets": open,
        }));
    }
    let teams: Vec<Value> = teams
        .iter()
        .map(|(t, members)| {
            json!({ "id": t.id, "name": t.name, "solo": t.solo, "lead": t.lead, "project": t.project,
                    "project_name": t.project.and_then(|p| projects.iter().find(|x| x.id == p)).map(|p| p.name.clone()),
                    "members": members.iter().map(|m| m.id).collect::<Vec<_>>() })
        })
        .collect();
    Ok(json!({
        "agents": agents,
        "teams": teams,
        "totals": { "live_runs": unfinished.len(), "tokens_today": tokens_today,
                    "working": by_agent.len() },
    }))
}

/// A worker's internal message bus: for each ticket it holds or held, the assignment (what the Lead handed over)
/// and every comment on it (questions, answers, progress, reports, reviews, permissions, System notes), oldest first.
#[tauri::command]
pub fn agent_bus(d: State<'_, Desktop>, agent: String, limit: Option<usize>) -> R<Value> {
    let o = d.opened()?;
    let id = aid(&agent)?;
    let names = names(&o)?;
    let author = |a: &Author| match a {
        Author::Agent(id) => names.get(id).cloned().unwrap_or_else(|| id.to_string()),
        other => other.to_string(),
    };
    let mut items: Vec<(String, Value)> = Vec::new();
    for p in o.app.projects().map_err(err)? {
        for t in o.app.tickets(p.id).map_err(err)? {
            let held = t.assignee == Some(id);
            let cs = o
                .app
                .store()
                .read(|c| comments::for_ticket(c, t.id))
                .map_err(err)?;
            let involved = held || cs.iter().any(|c| c.author == Author::Agent(id));
            if !involved {
                continue;
            }
            items.push((
                t.created_at.clone(),
                json!({ "type": "assignment", "ticket": t.key, "project": p.id, "title": t.title,
                        "description": t.description, "acceptance": t.acceptance, "status": t.status,
                        "scope_paths": t.scope_paths, "at": t.created_at }),
            ));
            for c in cs {
                items.push((
                    c.created_at.clone(),
                    json!({ "type": "comment", "ticket": t.key, "project": p.id, "id": c.id, "kind": c.kind,
                            "author": author(&c.author), "mine": c.author == Author::Agent(id),
                            "body": c.body, "resolved": c.resolved, "at": c.created_at }),
                ));
            }
        }
    }
    items.sort_by(|a, b| a.0.cmp(&b.0));
    let n = limit.unwrap_or(300);
    let skip = items.len().saturating_sub(n);
    Ok(json!(items
        .into_iter()
        .skip(skip)
        .map(|(_, v)| v)
        .collect::<Vec<_>>()))
}

/// The agent's current run (or its last one, or `run` when given): steps as the trace shows them (tool calls with
/// arguments, result preview, duration; retries; errors) and the shell commands for the terminal view.
#[tauri::command]
pub fn agent_trace(d: State<'_, Desktop>, agent: String, run: Option<String>) -> R<Value> {
    let o = d.opened()?;
    let id = aid(&agent)?;
    let names = names(&o)?;
    let r = match run {
        Some(r) => {
            let rid: RunId = r.parse().map_err(|e: IdParseError| e.to_string())?;
            Some(o.app.store().read(|c| runs::get(c, rid)).map_err(err)?)
        }
        None => o
            .app
            .store()
            .read(|c| runs::for_agent(c, id, 1))
            .map_err(err)?
            .into_iter()
            .next(),
    };
    let Some(r) = r else {
        return Ok(json!({ "run": null, "steps": [], "terminal": [] }));
    };
    let log = o
        .app
        .store()
        .read(|c| runs::log_entries(c, r.id))
        .map_err(err)?;
    let terminal: Vec<Value> = log
        .iter()
        .filter(|e| e.kind == "tool" && e.data["tool"].as_str().is_some_and(|t| SHELL.contains(&t)))
        .map(|e| {
            let cmd = e.data["args"]["command"]
                .as_str()
                .or_else(|| e.data["args"]["cmd"].as_str())
                .map(str::to_owned)
                .unwrap_or_else(|| e.data["args"].to_string());
            json!({ "n": e.n, "command": cmd, "ok": e.data["ok"], "output": e.data["result"], "ms": e.data["ms"],
                    "at": e.at })
        })
        .collect();
    Ok(json!({
        "run": run_json(&r, &names),
        "kind": run_kind(&o, &r),
        "ticket_key": ticket_key(&o, r.ticket),
        "live": r.ended_at.is_none(),
        "steps": log,
        "terminal": terminal,
    }))
}
