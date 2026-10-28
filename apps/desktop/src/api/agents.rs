//! Agents, teams, runs and events.

use super::*;

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

pub(super) fn memory_action_now(
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
            // A memory run belongs to no ticket: its log names what it learned from.
            if r.outcome.as_deref() == Some("Memory") {
                let log = o
                    .app
                    .store()
                    .read(|c| runs::log_entries(c, r.id))
                    .unwrap_or_default();
                if let Some(e) = log.iter().find(|e| e.kind == "run") {
                    v["source"] = e.data["source"].clone();
                }
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
                .map_err(err)?,
        ),
        None => None,
    };
    Ok(
        json!({ "run": run_json(&r, &names), "ticket_key": ticket.as_ref().map(|t| &t.key),
               "ticket_project": ticket.as_ref().map(|t| t.project), "log": log }),
    )
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
pub(super) fn signed_in_by_models(
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
pub(super) async fn cli_models(
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
        json!({ "configured": o.app.config().judge.enabled(), "disabled": h.disabled, "reason": h.reason, "keys_off": h.keys_off }),
    )
}

/// The judge's settings for the form (never the key: only whether one is stored).
#[tauri::command]
pub fn judge_config(d: State<'_, Desktop>) -> R<Value> {
    let o = d.opened()?;
    let j = o.app.config().judge;
    let h = o.app.judge_health();
    // The keys in the order they are used, each with whether its value is stored and whether it is set aside (never
    // the value itself).
    let mut keys = Vec::new();
    for k in j.key_list() {
        let has_value = o.app.secret(&k.secret_ref).map_err(err)?.is_some();
        keys.push(
            json!({ "name": k.name, "has_value": has_value, "off": h.keys_off.get(&k.name) }),
        );
    }
    let has_key = keys.iter().any(|k| k["has_value"] == true);
    Ok(json!({
        "base_url": j.base_url, "model": j.model, "has_key": has_key,
        "review_assist": j.review_assist, "skill_hint": j.skill_hint, "memory_gate": j.memory_gate,
        "skill_quality": j.skill_quality, "done_check": j.done_check, "trim_output": j.trim_output, "find_code": j.find_code, "shadow": j.shadow,
        "configured": j.enabled(), "disabled": h.disabled, "reason": h.reason, "keys": keys,
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

/// Brings a judge key (or every key) back into use after the user added credit or re-entered it, and checks it.
#[tauri::command]
pub async fn enable_judge(d: State<'_, Desktop>, name: Option<String>) -> R<()> {
    d.opened()?
        .app
        .enable_judge(name.as_deref())
        .await
        .map_err(err)
}

/// Adds a named judge key (its value goes to the vault), or replaces the value of one.
#[tauri::command]
pub fn judge_add_key(d: State<'_, Desktop>, name: String, key: String) -> R<()> {
    d.opened()?.app.add_judge_key(&name, &key).map_err(err)
}

/// Removes a judge key and its value.
#[tauri::command]
pub fn judge_remove_key(d: State<'_, Desktop>, name: String) -> R<()> {
    d.opened()?.app.remove_judge_key(&name).map_err(err)
}

/// Exports an agent (memory, skills and overlays, skill/MCP enablement; no secret values) to a file in `out`.
#[tauri::command]
pub async fn export_agent(d: State<'_, Desktop>, agent: String, out: String) -> R<String> {
    let o = d.opened()?;
    blocking(move || export_agent_now(&o, agent, out)).await
}

pub(super) fn export_agent_now(o: &Opened, agent: String, out: String) -> R<String> {
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

pub(super) fn import_agent_now(
    o: &Opened,
    file: String,
    project: String,
    name: Option<String>,
) -> R<Value> {
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
/// `clis: false` leaves the agent CLIs out (finding them runs each one: seconds the first time), so a page shows
/// the providers at once and asks again for the CLIs.
pub async fn capabilities(
    d: State<'_, Desktop>,
    refresh: Option<bool>,
    clis: Option<bool>,
) -> R<Value> {
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
    let pending = clis == Some(false);
    let (clis, models) = if pending {
        (vec![], Default::default())
    } else {
        let mut clis = detected_clis(&d, refresh == Some(true)).await?;
        let models = cli_models(&d, &o, &clis, refresh == Some(true)).await;
        signed_in_by_models(&mut clis, &models);
        (clis, models)
    };
    Ok(json!({
        "providers": providers, "clis": clis, "clis_pending": pending,
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
