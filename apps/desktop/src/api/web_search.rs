//! Web search in Settings (ADR-163): the order searches go in and which keys are set, set a key (its API told by
//! the key), remove one API's key. The keys go to the secret store and never come back to the UI.

use agenium_orchestrator::web_search::WebSearchStatus;
use tauri::State;

use crate::state::{err, Desktop, R};

#[tauri::command]
pub fn web_search_status(d: State<'_, Desktop>) -> R<WebSearchStatus> {
    Ok(d.opened()?.app.web_search_status())
}

#[tauri::command]
pub fn set_web_search_key(d: State<'_, Desktop>, key: String) -> R<WebSearchStatus> {
    d.opened()?.app.set_web_search_key(&key).map_err(err)
}

#[tauri::command]
pub fn remove_web_search_key(d: State<'_, Desktop>, api: String) -> R<WebSearchStatus> {
    d.opened()?.app.remove_web_search_key(&api).map_err(err)
}
