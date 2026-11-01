//! The first start (ADR-154): no setup screen. Agenium works in the folder the Windows installer asked for (its
//! bootstrap file), `AGENIUM_HOME`, or `~/.agenium`; the folder is set up on its own once the embedding model is in
//! place, in the system's language (changed later with the VI/EN switch). Linux and macOS have no installer step to
//! ask in: there the app asks once, before anything is written, which folder to work in.

use std::path::{Path, PathBuf};

use tauri::Manager;
use tauri_plugin_dialog::DialogExt;

use super::*;

/// The interface language a new root starts in: the system's (Vietnamese or English).
fn system_language() -> String {
    let vi = sys_locale::get_locale().is_some_and(|l| l.to_lowercase().starts_with("vi"));
    if vi { "vi" } else { "en" }.to_owned()
}

/// Sets up the root when it has no `config.toml` yet. Called once the embedding model is in place, before it opens.
pub fn set_up_root(d: &Desktop) -> R<()> {
    let root = d.root();
    if !agenium_config::setup::needs_setup(&root) {
        return Ok(());
    }
    let input = agenium_config::setup::SetupInput {
        root: Some(root.clone()),
        ui_language: system_language(),
    };
    let home = dirs::home_dir().unwrap_or_default().join(".agenium");
    // A root given by AGENIUM_HOME is this process's override: nothing is remembered machine-wide.
    let by_env = std::env::var_os(agenium_config::root::ENV_HOME).is_some_and(|v| !v.is_empty());
    let bootstrap = agenium_config::Bootstrap::default_path().filter(|_| !by_env);
    agenium_config::setup::complete_setup(&input, &home, bootstrap.as_deref()).map_err(err)?;
    // A root set up for a UI test (a throwaway one, under AGENIUM_HOME) is marked as such: it alone opens with the
    // in-memory vault key in a release build.
    if by_env && crate::state::memory_keyring_asked() {
        std::fs::write(root.join(crate::state::TEST_ROOT_MARK), "").map_err(err)?;
    }
    Ok(())
}

/// Where a picked folder puts the root: the folder itself when it holds Agenium's data or is named for it, else an
/// `Agenium` folder inside it ("D:\" becomes "D:\Agenium"), as the Windows installer does.
pub fn root_in(picked: &Path) -> PathBuf {
    let named = picked
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase().ends_with("agenium"))
        .unwrap_or(false);
    if named || picked.join("config.toml").is_file() {
        picked.to_path_buf()
    } else {
        picked.join("Agenium")
    }
}

/// A folder picked in the system's dialog (Linux and macOS first start), or `None` when the user closed it.
#[tauri::command]
pub async fn pick_work_folder(handle: AppHandle) -> R<Option<String>> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    handle.dialog().file().pick_folder(move |p| {
        let _ = tx.send(p);
    });
    let picked = rx.await.map_err(err)?;
    Ok(picked
        .and_then(|p| p.into_path().ok())
        .map(|p| root_in(&p).display().to_string()))
}

/// The first start's answer on Linux and macOS: Agenium works in `folder` (the default when empty). A folder that
/// already holds Agenium's data opens as it is. The folder is remembered (bootstrap file) unless it is the default.
#[tauri::command]
pub fn choose_work_folder(d: State<'_, Desktop>, handle: AppHandle, folder: String) -> R<Value> {
    if !d.asking_root() {
        return Ok(status(d, handle));
    }
    let home = dirs::home_dir().unwrap_or_default().join(".agenium");
    let root = if folder.trim().is_empty() {
        home.clone()
    } else {
        PathBuf::from(folder.trim())
    };
    if !root.is_absolute() {
        return Err(format!("{} is not a full path", root.display()));
    }
    std::fs::create_dir_all(&root).map_err(|e| format!("{}: {e}", root.display()))?;
    if root != home {
        if let Some(file) = agenium_config::Bootstrap::default_path() {
            agenium_config::Bootstrap { root: root.clone() }
                .save(&file)
                .map_err(err)?;
        }
    }
    d.set_root(root);
    tauri::async_runtime::spawn(crate::model::ensure(handle.app_handle().clone()));
    Ok(status(d, handle))
}
