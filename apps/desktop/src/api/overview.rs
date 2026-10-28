//! The overview: projects, boards, what needs the user.

use super::*;

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

pub(super) fn project_summary(o: &Opened, p: &agenium_store::core::projects::Project) -> R<Value> {
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
pub(super) fn api_run(r: &runs::Run) -> bool {
    !agenium_cli_backend::CliKind::ALL
        .iter()
        .any(|k| k.program() == r.provider)
}

/// Share of an API run's input tokens served from the provider's prompt cache (`None` for CLI runs or no input yet).
pub(super) fn cache_hit(r: &runs::Run) -> Option<f64> {
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
