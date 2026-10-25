// No console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod api;
mod live;
mod manage;
mod state;

use std::time::Duration;

use tauri::{Emitter, Manager};

use state::Desktop;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("AGENIUM_LOG")
                .unwrap_or_else(|_| "warn".into()),
        )
        .init();
    let root = agenium_config::resolve_root()
        .map(|(r, _)| r)
        .unwrap_or_else(|_| dirs::home_dir().unwrap_or_default().join(".agenium"));
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .manage(Desktop::new(root))
        .setup(|app| {
            let handle = app.handle().clone();
            let d = app.state::<Desktop>();
            if !agenium_config::setup::needs_setup(&d.root) {
                if let Err(e) = d.open(&handle) {
                    tracing::error!("cannot open the Agenium root: {e}");
                }
            }
            spawn_event_forwarder(handle.clone());
            spawn_usage_refresh(handle.clone());
            spawn_team_loop(handle);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            api::status,
            api::command_policy,
            api::set_command_policy,
            api::setup,
            api::projects,
            api::overview,
            api::needs_you,
            api::inbox,
            api::board,
            api::plan,
            api::ticket,
            api::ticket_diff,
            api::agent_detail,
            api::set_team_lead,
            api::audit,
            api::agents,
            api::runs,
            api::run_detail,
            api::events,
            api::capabilities,
            api::templates,
            api::chat,
            api::chat_history,
            api::approve,
            api::answer,
            api::decide_permission,
            api::decide_command,
            api::comment,
            api::ticket_action,
            api::create_project,
            api::create_team,
            api::create_solo,
            api::set_agent_model,
            api::rename_agent,
            api::prices,
            api::refresh_prices,
            api::vendor_usage,
            api::refresh_vendor_usage,
            api::read_claude_usage,
            api::decide_role_proposal,
            api::save_template,
            api::remove_template,
            api::template_from_team,
            api::template_toml,
            api::memory,
            api::memory_action,
            api::set_agent_status,
            api::pause_project,
            api::resume_project,
            api::stop_all,
            api::support_bundle,
            manage::roles,
            manage::put_role,
            manage::remove_role,
            manage::create_agent,
            manage::set_agent_role,
            manage::toggle_agent_skill,
            manage::toggle_agent_mcp,
            manage::agent_mcp_keys,
            manage::set_agent_mcp_key,
            manage::delete_agent,
            manage::teams,
            manage::compose_team,
            manage::suggest_lead,
            manage::team_member,
            manage::set_lead_of_team,
            manage::set_team_project,
            manage::dissolve_team,
            manage::skill_library,
            manage::import_skill,
            manage::remove_skill,
            manage::skill_text,
            manage::mcp_servers,
            manage::put_mcp,
            manage::import_mcp_json,
            manage::remove_mcp,
            manage::test_mcp,
            manage::put_provider,
            manage::remove_provider,
            manage::provider_models,
            manage::local_servers,
            live::live,
            live::agent_bus,
            live::agent_trace,
        ])
        .run(tauri::generate_context!())
        .expect("error while running the Agenium desktop app");
}

/// Committed events go to the UI as `agenium://event` (the UI refreshes what they touch; it resyncs from the log
/// after a gap).
fn spawn_event_forwarder(handle: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            let opened = handle.state::<Desktop>().opened();
            let Ok(o) = opened else {
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
            };
            let last = o
                .app
                .store()
                .read(agenium_store::core::events::last_seq)
                .unwrap_or(0);
            let mut sub = o.app.subscribe(last);
            while let Ok(e) = sub.recv().await {
                // Desktop notifications only for what needs the user (decision: selective notifications).
                if e.kind == "NeedsUser" {
                    if let Some(reason) = e.payload.get("reason").and_then(|r| r.as_str()) {
                        use tauri_plugin_notification::NotificationExt;
                        let _ = handle
                            .notification()
                            .builder()
                            .title("Agenium needs you")
                            .body(reason)
                            .show();
                    }
                }
                let _ = handle.emit("agenium://event", &e);
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    });
}

/// Every 15 minutes: the vendors' subscription usage (Codex, OpenCode Go), and the model price catalogue when it
/// is missing or a week old (runs that cost 0 for want of a price are priced then).
fn spawn_usage_refresh(handle: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            let Ok(o) = handle.state::<Desktop>().opened() else {
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            };
            let clis = api::detected_clis(&handle.state::<Desktop>(), false)
                .await
                .unwrap_or_default();
            if let Err(e) =
                agenium_orchestrator::vendor_usage::refresh_vendor_usage(&o.app, &clis).await
            {
                tracing::warn!(error = %e, "vendor usage not refreshed");
            }
            let _ = handle.emit("agenium://usage", ());
            if o.app.price_catalogue_stale() {
                match agenium_orchestrator::prices::refresh_price_catalogue(&o.app).await {
                    Ok(Some(n)) => tracing::info!(priced = n, "price catalogue refreshed"),
                    Ok(None) => {}
                    Err(e) => tracing::warn!(error = %e, "price catalogue not refreshed"),
                }
            }
            tokio::time::sleep(Duration::from_secs(
                agenium_orchestrator::vendor_usage::REFRESH_MINUTES * 60,
            ))
            .await;
        }
    });
}

/// The team loop (ADR-63): from the agreed requirements to the end, every project's team works on its own, round
/// after round (one round per project at a time, projects side by side), until it is done, waits for the user or
/// the user stops it.
fn spawn_team_loop(handle: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(3)).await;
            let d = handle.state::<Desktop>();
            let Ok(o) = d.opened() else { continue };
            let Ok(projects) = o.app.projects() else {
                continue;
            };
            for p in projects {
                if !d
                    .busy
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(p.id)
                {
                    continue;
                }
                let (o, h) = (o.clone(), handle.clone());
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = o.rt.work(p.id).await {
                        tracing::warn!(project = %p.name, error = %e, "team round failed");
                    }
                    h.state::<Desktop>()
                        .busy
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .remove(&p.id);
                });
            }
        }
    });
}
