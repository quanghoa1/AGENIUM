//! markitdown in Settings (ADR-152): what reads documents, and Agenium's own copy installed, self-tested, updated and
//! removed on the user's request. Each step of an install reaches the window as `agenium://markitdown`.

use agenium_orchestrator::markitdown_setup::{MarkitdownStatus, Method};
use tauri::Emitter;

use super::*;

fn progress(handle: AppHandle) -> impl Fn(&str) + Send + Sync {
    move |step: &str| {
        let _ = handle.emit("agenium://markitdown", json!({ "step": step }));
    }
}

#[tauri::command]
pub async fn markitdown_status(d: State<'_, Desktop>) -> R<MarkitdownStatus> {
    let o = d.opened()?;
    Ok(o.app.markitdown_status().await)
}

/// `method`: how to install (left out: uv, else the Python found, else a downloaded uv).
#[tauri::command]
pub async fn install_markitdown(
    handle: AppHandle,
    d: State<'_, Desktop>,
    method: Option<Method>,
) -> R<MarkitdownStatus> {
    let o = d.opened()?;
    o.app
        .install_markitdown(None, method, &progress(handle))
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn update_markitdown(handle: AppHandle, d: State<'_, Desktop>) -> R<MarkitdownStatus> {
    let o = d.opened()?;
    o.app
        .update_markitdown(&progress(handle))
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn check_markitdown_update(d: State<'_, Desktop>) -> R<MarkitdownStatus> {
    let o = d.opened()?;
    o.app.check_markitdown_update().await.map_err(err)
}

#[tauri::command]
pub async fn test_markitdown(d: State<'_, Desktop>) -> R<Vec<(String, String)>> {
    let o = d.opened()?;
    o.app.test_markitdown().await.map_err(err)
}

#[tauri::command]
pub async fn remove_markitdown(d: State<'_, Desktop>) -> R<MarkitdownStatus> {
    let o = d.opened()?;
    o.app.remove_markitdown().await.map_err(err)
}
