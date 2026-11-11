//! Many skills at once: the skills a chosen folder holds are listed, the user ticks the ones to bring in.

use std::path::PathBuf;

use tauri::State;

use super::*;

/// The skills a folder holds (itself, or its subfolders a few levels down), each with whether it can be imported and
/// whether the library already has one of its name.
#[tauri::command]
pub async fn scan_skills(d: State<'_, Desktop>, path: String) -> R<Vec<Value>> {
    let o = d.opened()?;
    let dir = PathBuf::from(path.trim().trim_matches('"'));
    // A large folder takes a moment to read: off the UI thread.
    let found = tokio::task::spawn_blocking(move || o.app.scan_skill_folder(&dir))
        .await
        .map_err(err)?
        .map_err(err)?;
    Ok(found
        .into_iter()
        .map(|(f, exists)| {
            json!({ "path": f.path, "name": f.name, "description": f.description, "error": f.error, "exists": exists })
        })
        .collect())
}

/// Imports the chosen skill folders; each one's result (`name`, or `error`).
#[tauri::command]
pub async fn import_skills(
    d: State<'_, Desktop>,
    paths: Vec<String>,
    replace: bool,
) -> R<Vec<Value>> {
    let o = d.opened()?;
    let dirs: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
    let out = tokio::task::spawn_blocking(move || o.app.import_library_skills(&dirs, replace))
        .await
        .map_err(err)?;
    Ok(paths
        .iter()
        .zip(out)
        .map(|(p, r)| match r {
            Ok(name) => json!({ "path": p, "name": name }),
            Err(e) => json!({ "path": p, "error": e }),
        })
        .collect())
}
