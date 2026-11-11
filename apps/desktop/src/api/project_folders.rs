//! The folders of a project (ADR-159): the one it works in, picked anywhere, and the reference folders its agents
//! read but never change.

use std::path::PathBuf;

use agenium_orchestrator::project_create::FolderInfo;
use agenium_store::core::projects::Reference;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use super::*;

/// A folder picked in the system's dialog, as it is, or `None` when the user closed it.
#[tauri::command]
pub async fn pick_folder(handle: AppHandle) -> R<Option<String>> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    handle.dialog().file().pick_folder(move |p| {
        let _ = tx.send(p);
    });
    let picked = rx.await.map_err(err)?;
    Ok(picked
        .and_then(|p| p.into_path().ok())
        .map(|p| p.display().to_string()))
}

/// What making a project in `path` does (a repository taken as it is, files committed, an empty folder).
#[tauri::command]
pub async fn inspect_folder(d: State<'_, Desktop>, path: String) -> R<FolderInfo> {
    let o = d.opened()?;
    // Counting a large folder's files takes a moment: off the UI thread.
    tokio::task::spawn_blocking(move || o.app.inspect_folder(&PathBuf::from(path.trim())))
        .await
        .map_err(err)?
        .map_err(err)
}

/// Replaces the project's reference folders; returns them as kept (full paths, no duplicates).
#[tauri::command]
pub fn set_references(
    d: State<'_, Desktop>,
    project: String,
    references: Vec<Reference>,
) -> R<Vec<Reference>> {
    let o = d.opened()?;
    o.app
        .set_references(project.parse().map_err(err)?, references)
        .map_err(err)
}
