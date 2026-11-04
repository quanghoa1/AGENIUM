//! IPC commands of plugins (ADR-157): import from a vendor's repository after a preview, update whole from the
//! vendor, remove, and turn a whole plugin on or off for an agent. The rules are the orchestrator's.

use agenium_core::*;
use agenium_orchestrator::plugins::{Plugin, PluginPreview};
use serde_json::{json, Value};
use tauri::State;

use crate::state::{err, Desktop, R};

fn aid(s: &str) -> R<AgentId> {
    s.parse().map_err(|e: IdParseError| e.to_string())
}

/// The installed plugins, each with the agents that have it on.
#[tauri::command]
pub fn plugins(d: State<'_, Desktop>) -> R<Value> {
    let o = d.opened()?;
    let agents = o.app.agents().map_err(err)?;
    let list: Vec<Value> = o
        .app
        .plugins()
        .map_err(err)?
        .into_iter()
        .map(|p| {
            let entry = format!("{}{}", agenium_skills::PLUGIN_PREFIX, p.name);
            let users: Vec<&str> = agents
                .iter()
                .filter(|a| a.skills.contains(&entry))
                .map(|a| a.name.as_str())
                .collect();
            json!({ "plugin": p, "users": users })
        })
        .collect();
    Ok(json!(list))
}

/// What installing `source` (`owner/repo[@ref]` or a folder) would bring; nothing is written.
#[tauri::command]
pub async fn preview_plugin(
    d: State<'_, Desktop>,
    source: String,
    entry: Option<String>,
) -> R<PluginPreview> {
    let o = d.opened()?;
    let entry = entry.filter(|e| !e.trim().is_empty());
    o.app
        .preview_plugin(&source, entry.as_deref())
        .await
        .map_err(err)
}

/// What updating `name` from its vendor would bring.
#[tauri::command]
pub async fn preview_plugin_update(d: State<'_, Desktop>, name: String) -> R<PluginPreview> {
    let o = d.opened()?;
    o.app.preview_plugin_update(&name).await.map_err(err)
}

/// Installs or replaces a plugin at the commit its preview showed.
#[tauri::command]
pub async fn install_plugin(
    d: State<'_, Desktop>,
    source: String,
    reference: String,
    entry: Option<String>,
    commit: String,
) -> R<Plugin> {
    let o = d.opened()?;
    o.app
        .install_plugin(&source, &reference, entry.as_deref(), &commit)
        .await
        .map_err(err)
}

#[tauri::command]
pub fn remove_plugin(d: State<'_, Desktop>, name: String) -> R<()> {
    d.opened()?.app.remove_plugin(&name).map_err(err)
}

/// Turns the whole plugin on or off for an agent (never one of its skills or servers).
#[tauri::command]
pub fn toggle_agent_plugin(
    d: State<'_, Desktop>,
    agent: String,
    name: String,
    on: bool,
) -> R<Value> {
    Ok(json!(d
        .opened()?
        .app
        .toggle_agent_plugin(aid(&agent)?, &name, on)
        .map_err(err)?))
}
