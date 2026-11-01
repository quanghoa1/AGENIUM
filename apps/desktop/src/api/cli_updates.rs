//! Agent CLIs updated on the user's request (ADR-153): their versions, and the CLI's own updater with a check after.

use agenium_orchestrator::cli_updates::{CliUpdateInfo, CliUpdated};

use super::*;

/// Each installed CLI with the version it has, the newest one and the one Agenium verified.
#[tauri::command]
pub async fn cli_updates(d: State<'_, Desktop>) -> R<Vec<CliUpdateInfo>> {
    let o = d.opened()?;
    let clis = detected_clis(&d, false).await?;
    Ok(o.app.cli_updates(&clis).await)
}

/// Updates the CLI `cli` (its program name) to the newest version, or to `version`; finds the CLIs again after.
#[tauri::command]
pub async fn update_cli(
    d: State<'_, Desktop>,
    cli: String,
    version: Option<String>,
) -> R<CliUpdated> {
    let o = d.opened()?;
    let clis = detected_clis(&d, true).await?;
    let before = clis
        .into_iter()
        .find(|c| c.kind.program() == cli)
        .ok_or_else(|| format!("{cli} is not an agent CLI Agenium drives"))?;
    let done =
        o.rt.update_cli(&before, version.as_deref())
            .await
            .map_err(err)?;
    detected_clis(&d, true).await?;
    Ok(done)
}
