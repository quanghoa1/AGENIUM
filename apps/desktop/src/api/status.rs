//! Setup and status: the root, the command policy, the sandbox, model downloads, how MCP tools are found.

use super::*;

#[tauri::command]
pub fn project_report(d: State<'_, Desktop>, project: String) -> R<String> {
    d.opened()?
        .app
        .project_report_md(pid(&project)?)
        .map_err(err)
}

#[tauri::command]
pub async fn support_bundle(d: State<'_, Desktop>, out: String) -> R<String> {
    let o = d.opened()?;
    let dir = std::path::PathBuf::from(if out.trim().is_empty() {
        std::env::temp_dir().display().to_string()
    } else {
        out
    });
    blocking(move || {
        Ok(o.app
            .support_bundle(&dir)
            .map_err(err)?
            .display()
            .to_string())
    })
    .await
}

#[tauri::command]
pub fn status(d: State<'_, Desktop>) -> Value {
    let opened = d.opened().ok();
    let cfg = opened.as_ref().map(|o| o.app.config());
    json!({
        "root": d.root.display().to_string(),
        "ready": opened.is_some(),
        "version": env!("CARGO_PKG_VERSION"),
        "ui_language": cfg.as_ref().map(|c| c.app.language.clone()),
        "model": d.model_prep(),
    })
}

/// How agent shell commands are approved (ADR-64) and which sandbox backend works on this machine.
#[tauri::command]
pub fn command_policy(d: State<'_, Desktop>) -> R<Value> {
    let o = d.opened()?;
    let sb = agenium_sandbox::detect();
    Ok(
        json!({ "approval": o.app.config().sandbox.approval, "unsandboxed": o.app.config().sandbox.unsandboxed,
                "backend": sb.backend, "reason": sb.reason }),
    )
}

#[tauri::command]
pub fn set_command_policy(
    d: State<'_, Desktop>,
    approval: agenium_config::ApprovalPolicy,
) -> R<Value> {
    d.opened()?
        .app
        .update_config(|c| c.sandbox.approval = approval)
        .map_err(err)?;
    command_policy(d)
}

/// Whether agents may run their commands without a sandbox on a machine that has none (ADR-117).
#[tauri::command]
pub fn set_unsandboxed(d: State<'_, Desktop>, on: bool) -> R<Value> {
    d.opened()?
        .app
        .update_config(|c| c.sandbox.unsandboxed = on)
        .map_err(err)?;
    command_policy(d)
}

#[tauri::command]
pub fn setup(d: State<'_, Desktop>, handle: AppHandle, ui_language: String) -> R<Value> {
    // A second click (or a second window) while the root is open: one App and one runtime per root.
    if d.opened().is_ok() {
        return Ok(status(d));
    }
    if d.model_prep().phase != "ready" {
        return Err("the embedding model is not in place yet".into());
    }
    let input = agenium_config::setup::SetupInput {
        root: Some(d.root.clone()),
        ui_language,
    };
    let home = dirs::home_dir().unwrap_or_default().join(".agenium");
    // A root given by AGENIUM_HOME is this process's override: nothing is remembered machine-wide.
    let by_env = std::env::var_os(agenium_config::root::ENV_HOME).is_some_and(|v| !v.is_empty());
    let bootstrap = agenium_config::Bootstrap::default_path().filter(|_| !by_env);
    let fresh = agenium_config::setup::needs_setup(&d.root);
    agenium_config::setup::complete_setup(&input, &home, bootstrap.as_deref()).map_err(err)?;
    // A root set up for a UI test (a throwaway one, under AGENIUM_HOME) is marked as such: it alone opens with the
    // in-memory vault key in a release build.
    if fresh && by_env && crate::state::memory_keyring_asked() {
        std::fs::write(d.root.join(crate::state::TEST_ROOT_MARK), "").map_err(err)?;
    }
    d.open(&handle)?;
    Ok(status(d))
}

/// How agents reach many MCP tools (ADR-139): found with `tool_search` past the thresholds, the tools used most kept
/// listed, the judge reading a search's candidates.
#[tauri::command]
pub fn tool_search_settings(d: State<'_, Desktop>) -> R<agenium_config::ToolSearchSection> {
    Ok(d.opened()?.app.config().tool_search)
}

#[tauri::command]
pub fn set_tool_search(
    d: State<'_, Desktop>,
    section: agenium_config::ToolSearchSection,
) -> R<agenium_config::ToolSearchSection> {
    if !["auto", "always", "never"].contains(&section.mode.as_str()) {
        return Err(format!("unknown mode {:?}", section.mode));
    }
    if !["unsure", "always", "off"].contains(&section.judge.as_str()) {
        return Err(format!("unknown judge setting {:?}", section.judge));
    }
    if section.over_tools == 0 || section.over_tokens == 0 {
        return Err("the thresholds must be above 0".into());
    }
    let c = d
        .opened()?
        .app
        .update_config(|c| c.tool_search = section)
        .map_err(err)?;
    Ok(c.tool_search)
}
