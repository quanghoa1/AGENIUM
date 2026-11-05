//! Status: the root, the command policy, the sandbox, model downloads, how MCP tools are found.

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
pub fn status(d: State<'_, Desktop>, handle: AppHandle) -> Value {
    let opened = d.opened().ok();
    let cfg = opened.as_ref().map(|o| o.app.config());
    json!({
        "root": d.root().display().to_string(),
        // Linux and macOS, first start: the folder to work in is asked before anything is written (ADR-154).
        "choose_root": d.asking_root().then(|| d.root().display().to_string()),
        "ready": opened.is_some(),
        // The installer's version (tauri.conf.json), the one Windows, the .deb and the .app show; not the crate's.
        "version": handle.package_info().version.to_string(),
        "ui_language": cfg.as_ref().map(|c| c.app.language.clone()),
        // This machine's ceiling on ticket runs at once per project (`runtime.max_parallel_agents`, lowered by the
        // setup on a smaller machine): the live view sums, per running project, its workers up to this ceiling.
        "max_parallel_agents": cfg.as_ref().map(|c| c.runtime.max_parallel_agents),
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

/// Which MCP tool calls wait for the user (ADR-143): the default of every server and each server's own setting.
#[tauri::command]
pub fn mcp_approval(d: State<'_, Desktop>) -> R<Value> {
    let c = d.opened()?.app.config();
    let servers: serde_json::Map<String, Value> = c
        .mcp
        .iter()
        .map(|s| (s.id.clone(), json!(s.approval)))
        .collect();
    Ok(json!({ "default": c.sandbox.mcp_tools, "servers": servers }))
}

/// Sets the default (`server` empty) or one server's setting (`approval` empty: the server follows the default).
#[tauri::command]
pub fn set_mcp_approval(
    d: State<'_, Desktop>,
    server: Option<String>,
    approval: Option<agenium_config::McpApproval>,
) -> R<Value> {
    let o = d.opened()?;
    match server.filter(|s| !s.is_empty()) {
        None => {
            let a = approval.ok_or("the default needs a setting")?;
            o.app
                .update_config(|c| c.sandbox.mcp_tools = a)
                .map_err(err)?;
        }
        Some(id) => {
            let mut cfg = o
                .app
                .config()
                .mcp_server(&id)
                .cloned()
                .ok_or(format!("no MCP server {id}"))?;
            cfg.approval = approval;
            o.app.put_mcp_server(cfg).map_err(err)?;
        }
    }
    mcp_approval(d)
}
