//! The project's map page (ADR-166): the user looks at what the project's agents mapped, searches it, finds a chain
//! between two parts, reads the evidence of a link. Nothing here changes the map: only the agents add to it.

use agenium_orchestrator::map::{MapGraph, MapStats, NodeView};
use tauri::State;

use super::pid;
use crate::state::{err, Desktop, R};

#[tauri::command]
pub fn map_stats(d: State<'_, Desktop>, project: String) -> R<MapStats> {
    d.opened()?.app.map_stats(pid(&project)?).map_err(err)
}

#[tauri::command]
pub fn map_search(d: State<'_, Desktop>, project: String, q: String) -> R<Vec<NodeView>> {
    d.opened()?.app.map_search(pid(&project)?, &q).map_err(err)
}

#[tauri::command]
pub fn map_around(
    d: State<'_, Desktop>,
    project: String,
    key: Option<String>,
    depth: Option<usize>,
) -> R<MapGraph> {
    d.opened()?
        .app
        .map_around(pid(&project)?, key.as_deref(), depth.unwrap_or(1))
        .map_err(err)
}

#[tauri::command]
pub fn map_path(
    d: State<'_, Desktop>,
    project: String,
    from: String,
    to: String,
) -> R<Option<MapGraph>> {
    d.opened()?
        .app
        .map_path(pid(&project)?, &from, &to)
        .map_err(err)
}

#[tauri::command]
pub fn map_code(d: State<'_, Desktop>, project: String, evidence: String) -> R<String> {
    d.opened()?
        .app
        .map_code(pid(&project)?, &evidence)
        .map_err(err)
}
