//! IPC commands of the registry (ADR-61): roles, agents in the pool, custom teams, the skill library, MCP servers
//! and providers. Every rule lives in the orchestrator; these only translate.

use std::path::PathBuf;

use agenium_config::{McpServerConfig, ProviderKind};
use agenium_core::*;
use agenium_orchestrator::registry::{NewAgent, NewTeam, ProviderInput, RoleDef};
use serde::Deserialize;
use serde_json::{json, Value};
use tauri::State;

use crate::api::{pid, ModelArg};
use crate::state::{err, Desktop, R};

fn aid(s: &str) -> R<AgentId> {
    s.parse().map_err(|e: IdParseError| e.to_string())
}

fn tid(s: &str) -> R<TeamId> {
    s.parse().map_err(|e: IdParseError| e.to_string())
}

// ---------------------------------------------------------------- roles

#[tauri::command]
pub fn roles(d: State<'_, Desktop>) -> R<Value> {
    Ok(json!(d.opened()?.app.roles().map_err(err)?))
}

#[tauri::command]
pub fn put_role(d: State<'_, Desktop>, role: RoleDef) -> R<Value> {
    Ok(json!(d.opened()?.app.put_role(&role).map_err(err)?))
}

#[tauri::command]
pub fn remove_role(d: State<'_, Desktop>, id: String) -> R<()> {
    d.opened()?.app.remove_role(&id).map_err(err)
}

// ---------------------------------------------------------------- agents

#[derive(Debug, Deserialize)]
pub struct AgentForm {
    name: String,
    role: String,
    #[serde(default)]
    tags: Option<Vec<String>>,
    model: ModelArg,
    #[serde(default)]
    skills: Vec<String>,
    #[serde(default)]
    mcp: Vec<String>,
}

#[tauri::command]
pub fn create_agent(d: State<'_, Desktop>, form: AgentForm) -> R<Value> {
    let o = d.opened()?;
    let (backend, model) = form.model.binding()?;
    let a = o
        .app
        .create_agent(&NewAgent {
            name: form.name,
            role: form.role,
            tags: form.tags,
            model,
            backend,
            skills: form.skills,
            mcp: form
                .mcp
                .into_iter()
                .map(|server| agenium_mcp::Binding {
                    server,
                    credentials: Default::default(),
                    keys: Default::default(),
                    tools: None,
                })
                .collect(),
        })
        .map_err(err)?;
    Ok(json!(a))
}

#[tauri::command]
pub fn set_agent_role(
    d: State<'_, Desktop>,
    agent: String,
    role: String,
    tags: Option<Vec<String>>,
) -> R<()> {
    d.opened()?
        .app
        .set_agent_role(aid(&agent)?, &role, tags)
        .map_err(err)
}

#[tauri::command]
pub fn toggle_agent_skill(
    d: State<'_, Desktop>,
    agent: String,
    skill: String,
    on: bool,
) -> R<Value> {
    Ok(json!(d
        .opened()?
        .app
        .toggle_agent_skill(aid(&agent)?, &skill, on)
        .map_err(err)?))
}

#[tauri::command]
pub fn toggle_agent_mcp(d: State<'_, Desktop>, agent: String, server: String, on: bool) -> R<()> {
    d.opened()?
        .app
        .toggle_agent_mcp(aid(&agent)?, &server, on)
        .map_err(err)
}

/// The keys of the MCP servers an agent uses: the system value and the agent's own key (shown as they are).
#[tauri::command]
pub fn agent_mcp_keys(d: State<'_, Desktop>, agent: String) -> R<Value> {
    Ok(json!(d
        .opened()?
        .app
        .agent_mcp_keys(aid(&agent)?)
        .map_err(err)?))
}

/// Sets an agent's own key for one entry of a server (`value` null: back to the system key).
#[tauri::command]
pub fn set_agent_mcp_key(
    d: State<'_, Desktop>,
    agent: String,
    server: String,
    name: String,
    value: Option<String>,
) -> R<()> {
    d.opened()?
        .app
        .set_agent_mcp_key(aid(&agent)?, &server, &name, value.as_deref(), "user")
        .map_err(err)
}

/// Deletes an agent for good (the user typed its name again); its MCP connections are dropped.
#[tauri::command]
pub async fn delete_agent(d: State<'_, Desktop>, agent: String, confirm: String) -> R<Value> {
    let o = d.opened()?;
    let r = o.app.delete_agent(aid(&agent)?, &confirm).map_err(err)?;
    o.rt.mcp.disconnect_agent(&r.name).await;
    Ok(json!({ "name": r.name, "pending": r.pending }))
}

// ---------------------------------------------------------------- teams

/// Every team with its members, and the agents in the pool.
#[tauri::command]
pub fn teams(d: State<'_, Desktop>) -> R<Value> {
    let o = d.opened()?;
    let projects = o.app.projects().map_err(err)?;
    let name_of = |p: Option<ProjectId>| {
        p.and_then(|p| projects.iter().find(|x| x.id == p).map(|x| x.name.clone()))
    };
    // The projects each team worked on before its current one, newest first (a team works one project at a time).
    let moves = o
        .app
        .store()
        .read(|c| agenium_store::core::events::recent_of_kind(c, "TeamProjectChanged", 1000))
        .map_err(err)?;
    let past_of = |team: agenium_core::TeamId, now: Option<ProjectId>| -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for e in &moves {
            if let Some(agenium_core::DomainEvent::TeamProjectChanged {
                team: t,
                project: Some(p),
            }) = e.decode()
            {
                if t == team && Some(p) != now {
                    if let Some(n) = name_of(Some(p)).filter(|n| !out.contains(n)) {
                        out.push(n);
                    }
                }
            }
        }
        out
    };
    let teams: Vec<Value> = o
        .app
        .teams()
        .map_err(err)?
        .into_iter()
        .map(|(t, members)| {
            let planner = members
                .iter()
                .find(|m| !t.solo && o.app.is_planner_role(&m.role))
                .map(|m| m.id);
            let past = past_of(t.id, t.project);
            json!({ "team": t, "project_name": name_of(t.project), "past_projects": past, "members": members, "planner": planner })
        })
        .collect();
    let pool: Vec<_> = o
        .app
        .agents()
        .map_err(err)?
        .into_iter()
        .filter(|a| a.team.is_none())
        .collect();
    Ok(json!({ "teams": teams, "pool": pool }))
}

#[derive(Debug, Deserialize)]
pub struct TeamForm {
    name: String,
    #[serde(default)]
    project: Option<String>,
    lead: String,
    #[serde(default)]
    members: Vec<String>,
    #[serde(default)]
    solo: bool,
}

#[tauri::command]
pub fn compose_team(d: State<'_, Desktop>, form: TeamForm) -> R<Value> {
    let o = d.opened()?;
    let t = o
        .app
        .compose_team(&NewTeam {
            name: form.name,
            project: form.project.as_deref().map(pid).transpose()?,
            lead: aid(&form.lead)?,
            members: form.members.iter().map(|m| aid(m)).collect::<R<Vec<_>>>()?,
            solo: form.solo,
        })
        .map_err(err)?;
    Ok(json!(t))
}

#[tauri::command]
pub fn suggest_lead(d: State<'_, Desktop>, agents: Vec<String>) -> R<Value> {
    let ids = agents.iter().map(|a| aid(a)).collect::<R<Vec<_>>>()?;
    Ok(json!(d.opened()?.app.suggest_lead(&ids).map_err(err)?))
}

#[tauri::command]
pub fn team_member(d: State<'_, Desktop>, team: String, agent: String, add: bool) -> R<()> {
    let o = d.opened()?;
    let (t, a) = (tid(&team)?, aid(&agent)?);
    if add {
        o.app.add_team_member(t, a)
    } else {
        o.app.remove_team_member(t, a)
    }
    .map_err(err)
}

#[tauri::command]
pub fn set_lead_of_team(d: State<'_, Desktop>, team: String, agent: String) -> R<()> {
    d.opened()?
        .app
        .set_lead_of_team(tid(&team)?, aid(&agent)?)
        .map_err(err)
}

#[tauri::command]
pub fn set_team_project(d: State<'_, Desktop>, team: String, project: Option<String>) -> R<()> {
    d.opened()?
        .app
        .set_team_project(tid(&team)?, project.as_deref().map(pid).transpose()?)
        .map_err(err)
}

#[tauri::command]
pub fn dissolve_team(d: State<'_, Desktop>, team: String) -> R<()> {
    d.opened()?.app.dissolve_team(tid(&team)?).map_err(err)
}

// ---------------------------------------------------------------- skill library

/// The library with the agents that enable each skill, and folders that failed validation.
#[tauri::command]
pub fn skill_library(d: State<'_, Desktop>) -> R<Value> {
    let o = d.opened()?;
    let set = o.app.skill_registry().library();
    let agents = o.app.agents().map_err(err)?;
    let skills: Vec<Value> = set
        .skills
        .iter()
        .map(|s| {
            let users: Vec<&str> = agents
                .iter()
                .filter(|a| a.skills.contains(&s.meta.name))
                .map(|a| a.name.as_str())
                .collect();
            json!({ "name": s.meta.name, "description": s.meta.description, "version": s.meta.version,
                    "tags": s.meta.tags, "files": s.files.len(), "users": users, "dir": s.meta.dir })
        })
        .collect();
    let problems: Vec<Value> = set
        .problems
        .iter()
        .map(|p| json!({ "dir": p.dir, "error": p.error }))
        .collect();
    Ok(json!({ "skills": skills, "problems": problems }))
}

/// Imports a skill folder or `.zip` into the library.
#[tauri::command]
pub async fn import_skill(d: State<'_, Desktop>, path: String, replace: bool) -> R<Value> {
    let src = PathBuf::from(path.trim().trim_matches('"'));
    let o = d.opened()?;
    let meta = o.app.import_library_skill(&src, replace).map_err(err)?;
    // ADR-88: advisory skill-shape checks shown after import (one job, when-to-use, steps, tools, proof).
    let body = o.app.library_skill_body(&meta.name).unwrap_or_default();
    let mut notes = o.app.lint_skill(&meta.name, &body);
    // ADR-92: explicit opposite directives against the rest of the library (advice; the user decides).
    notes.extend(o.app.library_skill_conflicts(&meta.name));
    // ADR-90: when the judge is on, a calibrated read of the skill against the methodology principles.
    let quality = o.app.skill_quality(&body).await.unwrap_or_default();
    Ok(
        json!({ "name": meta.name, "description": meta.description, "shape_notes": notes, "quality": quality }),
    )
}

/// The empty SKILL.md template (the recommended sections) for authoring a new skill in the app.
#[tauri::command]
pub fn skill_skeleton(d: State<'_, Desktop>) -> R<Value> {
    Ok(json!({ "body": d.opened()?.app.skill_skeleton() }))
}

/// Evaluates SKILL.md text that is not saved yet (the editor's "Evaluate" button): the shape lint (ADR-88) and, when
/// the judge is on, the Jev quality read (ADR-90). Conflicts (ADR-92) need the saved skill, so they run on save.
#[tauri::command]
pub async fn evaluate_skill_text(d: State<'_, Desktop>, content: String) -> R<Value> {
    let o = d.opened()?;
    let (fm, body) = agenium_skills::parse(&content).map_err(|e| e.to_string())?;
    let name = fm.get("name").unwrap_or("").to_owned();
    let notes = o.app.lint_skill(&name, &body);
    let quality = o.app.skill_quality(&body).await;
    Ok(json!({ "shape_notes": notes, "quality": quality }))
}

/// A relative path inside a skill folder, restricted to `references/` and `scripts/` with safe segments.
fn skill_rel(rel: &str) -> R<PathBuf> {
    let rel = rel.trim().replace('\\', "/");
    let ok = (rel.starts_with("references/") || rel.starts_with("scripts/"))
        && !rel.contains("..")
        && rel.split('/').all(|s| {
            !s.is_empty()
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
        });
    if !ok {
        return Err(format!(
            "path must be under references/ or scripts/ (a-z, 0-9, . _ - only); got {rel:?}"
        ));
    }
    Ok(PathBuf::from(rel))
}

/// A skill's library folder on disk.
fn skill_dir(o: &crate::state::Opened, name: &str) -> R<PathBuf> {
    let set = o.app.skill_registry().library();
    Ok(set
        .get(name)
        .ok_or_else(|| format!("no library skill {name:?}"))?
        .meta
        .dir
        .clone())
}

/// One of a skill's files (reference or script) for editing in the app.
#[tauri::command]
pub fn skill_file(d: State<'_, Desktop>, name: String, path: String) -> R<Value> {
    let o = d.opened()?;
    let p = skill_dir(&o, &name)?.join(skill_rel(&path)?);
    let text = std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
    Ok(json!({ "path": path, "content": text }))
}

/// Writes one of a skill's reference/script files (create or edit), then re-validates the skill folder.
#[tauri::command]
pub fn save_skill_file(
    d: State<'_, Desktop>,
    name: String,
    path: String,
    content: String,
) -> R<Value> {
    let o = d.opened()?;
    let dir = skill_dir(&o, &name)?;
    let rel = skill_rel(&path)?;
    let full = dir.join(&rel);
    if let Some(parent) = full.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(&full, &content).map_err(|e| e.to_string())?;
    // A bad file (e.g. a reference the SKILL.md does not point at is fine; this just confirms the skill still loads).
    let body = o.app.library_skill_body(&name).unwrap_or_default();
    let notes = o.app.lint_skill(&name, &body);
    Ok(json!({ "path": path, "shape_notes": notes }))
}

/// Writes a library skill's SKILL.md from the editor (create or edit), validates it, then returns the three checks so
/// the author sees the advice right away: shape lint (ADR-88), library conflicts (ADR-92), Jev quality (ADR-90).
#[tauri::command]
pub async fn save_skill(
    d: State<'_, Desktop>,
    name: String,
    content: String,
    create: bool,
) -> R<Value> {
    let o = d.opened()?;
    let meta = o
        .app
        .save_library_skill(name.trim(), &content, create)
        .map_err(err)?;
    let body = o.app.library_skill_body(&meta.name).unwrap_or_default();
    let mut notes = o.app.lint_skill(&meta.name, &body);
    notes.extend(o.app.library_skill_conflicts(&meta.name));
    let quality = o.app.skill_quality(&body).await;
    Ok(json!({ "name": meta.name, "shape_notes": notes, "quality": quality }))
}

/// Re-runs the three skill checks on a library skill already in place (not only at import): the shape lint (ADR-88),
/// opposite-directive conflicts against the rest of the library (ADR-92), and, when the judge is on, its calibrated
/// quality read (ADR-90, a Jev call). On demand because the quality read costs a judge call.
#[tauri::command]
pub async fn evaluate_skill(d: State<'_, Desktop>, name: String) -> R<Value> {
    let o = d.opened()?;
    let body = o
        .app
        .library_skill_body(&name)
        .ok_or_else(|| format!("no library skill {name:?}"))?;
    let mut notes = o.app.lint_skill(&name, &body);
    notes.extend(o.app.library_skill_conflicts(&name));
    let quality = o.app.skill_quality(&body).await;
    Ok(json!({ "shape_notes": notes, "quality": quality }))
}

#[tauri::command]
pub fn remove_skill(d: State<'_, Desktop>, name: String, force: bool) -> R<()> {
    d.opened()?
        .app
        .remove_library_skill(&name, force)
        .map_err(err)
}

/// One skill's instructions (SKILL.md body) for reading, and the full SKILL.md (`raw`) for editing in the app.
#[tauri::command]
pub fn skill_text(d: State<'_, Desktop>, name: String) -> R<Value> {
    let o = d.opened()?;
    let set = o.app.skill_registry().library();
    let s = set
        .get(&name)
        .ok_or_else(|| format!("no library skill {name:?}"))?;
    let raw = std::fs::read_to_string(s.meta.dir.join("SKILL.md")).unwrap_or_default();
    Ok(json!({ "name": s.meta.name, "body": s.body, "raw": raw, "files": s.files }))
}

// ---------------------------------------------------------------- MCP servers

#[tauri::command]
pub fn mcp_servers(d: State<'_, Desktop>) -> R<Value> {
    let o = d.opened()?;
    let agents = o.app.agents().map_err(err)?;
    let hub = agenium_orchestrator::mcp::hub(o.app.clone());
    let servers: Vec<Value> = o
        .app
        .config()
        .mcp
        .iter()
        .map(|m| {
            let users: Vec<&str> = agents
                .iter()
                .filter(|a| {
                    agenium_mcp::parse_bindings(&a.mcp)
                        .map(|b| b.iter().any(|x| x.server == m.id))
                        .unwrap_or(false)
                })
                .map(|a| a.name.as_str())
                .collect();
            json!({ "config": m, "users": users, "signed_in": hub.signed_in(m) })
        })
        .collect();
    Ok(json!(servers))
}

#[tauri::command]
pub fn put_mcp(d: State<'_, Desktop>, server: McpServerConfig) -> R<()> {
    d.opened()?.app.put_mcp_server(server).map_err(err)
}

/// Pasted `{"mcpServers": {...}}` JSON; values are kept as written.
#[tauri::command]
pub fn import_mcp_json(d: State<'_, Desktop>, text: String) -> R<Value> {
    let r = d.opened()?.app.import_mcp_json(&text).map_err(err)?;
    Ok(json!({ "servers": r.servers }))
}

#[tauri::command]
pub fn remove_mcp(d: State<'_, Desktop>, id: String, force: bool) -> R<()> {
    let o = d.opened()?;
    if force {
        o.app.remove_mcp_server_force(&id)
    } else {
        o.app.remove_mcp_server(&id)
    }
    .map_err(err)
}

/// Signs the user in to an HTTP server that asks for OAuth: the browser opens, the tokens go to the vault (ADR-97).
#[tauri::command]
pub async fn login_mcp(d: State<'_, Desktop>, id: String) -> R<Value> {
    let o = d.opened()?;
    let cfg = o
        .app
        .config()
        .mcp_server(&id)
        .cloned()
        .ok_or_else(|| format!("no MCP server {id}"))?;
    let hub = agenium_orchestrator::mcp::hub(o.app.clone());
    let open = |url: &str| {
        if !agenium_mcp::oauth::open_browser(url) {
            tracing::warn!("could not open the browser for the MCP sign-in");
        }
    };
    let t = hub.login(&cfg, &[], &open).await.map_err(err)?;
    Ok(json!({ "client_id": t.client_id, "scope": t.scope }))
}

#[tauri::command]
pub async fn logout_mcp(d: State<'_, Desktop>, id: String) -> R<()> {
    let o = d.opened()?;
    agenium_orchestrator::mcp::hub(o.app.clone())
        .logout(&id)
        .await
        .map_err(err)
}

/// Connects, lists the tools, disconnects.
#[tauri::command]
pub async fn test_mcp(d: State<'_, Desktop>, id: String) -> R<Value> {
    let o = d.opened()?;
    let cfg = o
        .app
        .config()
        .mcp_server(&id)
        .cloned()
        .ok_or_else(|| format!("no MCP server {id}"))?;
    let hub = agenium_orchestrator::mcp::hub(o.app.clone());
    let binding = agenium_mcp::Binding {
        server: id,
        credentials: Default::default(),
        keys: Default::default(),
        tools: None,
    };
    let (info, protocol, tools) = hub.test(&cfg, &binding).await.map_err(err)?;
    let tools: Vec<Value> = tools
        .iter()
        .map(|t| json!({ "name": t.name, "description": t.description, "read_only": t.read_only }))
        .collect();
    Ok(json!({ "server": info, "protocol": protocol, "tools": tools }))
}

// ---------------------------------------------------------------- providers

#[derive(Debug, Deserialize)]
pub struct ProviderForm {
    id: String,
    kind: String,
    #[serde(default)]
    base_url: Option<String>,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    clear_key: bool,
    #[serde(default)]
    max_concurrency: Option<u32>,
    #[serde(default)]
    models: Vec<String>,
    #[serde(default)]
    headers: Option<Vec<agenium_orchestrator::registry::HeaderInput>>,
    #[serde(default)]
    model_caps: Option<Vec<agenium_config::ModelCaps>>,
    #[serde(default)]
    body: Option<Vec<agenium_config::BodyField>>,
    #[serde(default)]
    effort_field: Option<String>,
}

#[tauri::command]
pub fn put_provider(d: State<'_, Desktop>, form: ProviderForm) -> R<()> {
    let o = d.opened()?;
    let kind = match form.kind.as_str() {
        "anthropic" => ProviderKind::Anthropic,
        "gemini" => ProviderKind::Gemini,
        "openai_compat" => ProviderKind::OpenaiCompat,
        other => return Err(format!("unknown provider kind {other:?}")),
    };
    o.app
        .put_provider(&ProviderInput {
            headers: form.headers,
            body: form.body,
            effort_field: form.effort_field,
            model_caps: form.model_caps,
            id: form.id.trim().to_owned(),
            kind,
            base_url: form
                .base_url
                .map(|u| u.trim().to_owned())
                .filter(|u| !u.is_empty()),
            api_key: form.api_key,
            clear_key: form.clear_key,
            max_concurrency: form.max_concurrency.unwrap_or(4),
            models: form.models,
        })
        .map_err(err)?;
    o.rt.forget_provider(form.id.trim());
    Ok(())
}

#[tauri::command]
pub fn remove_provider(d: State<'_, Desktop>, id: String) -> R<()> {
    let o = d.opened()?;
    o.app.remove_provider(&id).map_err(err)?;
    o.rt.forget_provider(&id);
    Ok(())
}

/// Models the provider lists, with what Agenium knows about them (tool calling, vision, context window).
#[tauri::command]
pub async fn provider_models(d: State<'_, Desktop>, id: String) -> R<Value> {
    let o = d.opened()?;
    let p = o.rt.providers.provider(&id).map_err(err)?;
    let pc = o.app.config().provider(&id).cloned();
    // An endpoint that cannot list its models still has the ones the user named.
    let listed = p.list_models().await;
    let named = pc
        .iter()
        .flat_map(|c| c.models.iter().chain(c.model_caps.iter().map(|m| &m.model)));
    let mut models = match listed {
        Ok(m) => m,
        Err(_)
            if pc
                .as_ref()
                .is_some_and(|c| !c.models.is_empty() || !c.model_caps.is_empty()) =>
        {
            vec![]
        }
        Err(e) => return Err(e.to_string()),
    };
    for n in named.cloned().collect::<Vec<_>>() {
        if !models.iter().any(|m| m.id == n) {
            models.push(agenium_llm::ModelInfo {
                id: n,
                ..Default::default()
            });
        }
    }
    // Only what the API serves (OpenCode's free models need the opencode CLI backend, ADR-68).
    models.retain(|m| o.app.api_serves(&id, &m.id));
    if let Some(pc) = &pc {
        for m in &mut models {
            agenium_orchestrator::runtime::apply_caps(pc, m);
        }
    }
    Ok(json!(models))
}

/// Known vendors with their adapter, endpoint and usual key variable (ADR-107).
#[tauri::command]
pub fn provider_presets() -> R<Value> {
    let kind = |k: agenium_config::ProviderKind| match k {
        agenium_config::ProviderKind::Anthropic => "anthropic",
        agenium_config::ProviderKind::Gemini => "gemini",
        _ => "openai_compat",
    };
    Ok(json!(agenium_config::presets::PRESETS
        .iter()
        .map(|p| json!({
            "id": p.id,
            "name": p.name,
            "kind": kind(p.kind),
            "base_url": p.base_url,
            "key_env": p.key_env,
            "note": p.note,
        }))
        .collect::<Vec<_>>()))
}

/// Local model servers answering on this machine.
#[tauri::command]
pub async fn local_servers() -> R<Value> {
    let http = agenium_llm::http::Http::default_client().map_err(|e| e.to_string())?;
    Ok(json!(agenium_llm::local::discover(&http).await))
}
