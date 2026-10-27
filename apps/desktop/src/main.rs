// No console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod api;
mod browser;
mod live;
mod manage;
mod model;
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
        .plugin(nav_guard())
        .manage(Desktop::new(root))
        .setup(|app| {
            let handle = app.handle().clone();
            // The root opens once the embedding model is in place (memory needs it).
            tauri::async_runtime::spawn(model::ensure(handle.clone()));
            tauri::async_runtime::spawn(api::warm_cli_caches(handle.clone()));
            spawn_event_forwarder(handle.clone());
            spawn_usage_refresh(handle.clone());
            spawn_maintenance(handle.clone());
            spawn_team_loop(handle);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            api::status,
            api::command_policy,
            api::set_command_policy,
            api::set_unsandboxed,
            browser::browser_open,
            browser::browser_bounds,
            browser::browser_visible,
            browser::browser_action,
            browser::browser_url,
            browser::browser_hide_all,
            browser::browser_close,
            browser::browser_close_all,
            browser::browser_capture,
            browser::browser_save_pick,
            api::setup,
            api::projects,
            api::overview,
            api::needs_you,
            api::inbox,
            api::board,
            api::plan,
            api::ticket,
            api::ticket_diff,
            api::ticket_file,
            api::agent_detail,
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
            api::embedding_model,
            api::retry_embedding_model,
            api::install_embedding_model,
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
            api::project_report,
            api::judge_status,
            api::enable_judge,
            api::judge_config,
            api::judge_activity,
            api::set_ui_language,
            api::set_judge,
            api::export_agent,
            api::import_agent,
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
            manage::provider_presets,
            manage::login_mcp,
            manage::logout_mcp,
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
                        let title = if o.app.config().app.language == "vi" {
                            "Agenium cần bạn"
                        } else {
                            "Agenium needs you"
                        };
                        let _ = handle
                            .notification()
                            .builder()
                            .title(title)
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

/// Every minute: the maintenance tick — WAL gate on every database, scheduled backup with retention, run-log
/// compaction (it ran nowhere outside tests until 2026-10-25). Off the async runtime: a backup can take a while.
fn spawn_maintenance(handle: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
            let Ok(o) = handle.state::<Desktop>().opened() else {
                continue;
            };
            let app = o.app.clone();
            match tokio::task::spawn_blocking(move || app.maintenance_tick_now()).await {
                Ok(Ok(r)) => {
                    if let Some(b) = &r.backup {
                        tracing::info!(backup = %b, pruned = r.pruned.len(), "scheduled backup made");
                    }
                }
                Ok(Err(e)) => tracing::warn!(error = %e, "maintenance tick failed"),
                Err(e) => tracing::warn!(error = %e, "maintenance tick panicked"),
            }
        }
    });
}

/// The window never leaves the app: a link, a dropped URL or a debugger that would load another page in the webview is
/// refused (that page has no way back, and every command from it is refused). Seen when a worker checking its own web
/// page drove the first DevTools target it found, the app's window, which a test run had opened for automation.
fn nav_guard<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri::plugin::Builder::new("nav-guard")
        .on_navigation(|webview, url| {
            // The preview browser (ADR-121) shows other pages by design; its own rule (http and https only, never the
            // app's origins) is in `browser.rs`.
            if webview.label().starts_with(browser::PREFIX) {
                return true;
            }
            let ok = app_page(
                url,
                webview
                    .app_handle()
                    .config()
                    .build
                    .dev_url
                    .as_ref()
                    .filter(|_| tauri::is_dev()),
            );
            if !ok {
                tracing::warn!(%url, "a navigation away from the app was refused");
            }
            ok
        })
        .build()
}

/// The app's own pages: the bundled UI (`tauri://localhost`, `http(s)://tauri.localhost` on Windows), the dev
/// server in development, and `about:blank`.
fn app_page(url: &tauri::Url, dev: Option<&tauri::Url>) -> bool {
    let same = |d: &tauri::Url| {
        url.scheme() == d.scheme()
            && url.host_str() == d.host_str()
            && url.port_or_known_default() == d.port_or_known_default()
    };
    url.scheme() == "tauri"
        || url.as_str() == "about:blank"
        || (matches!(url.scheme(), "http" | "https") && url.host_str() == Some("tauri.localhost"))
        || dev.is_some_and(same)
}

/// Frees a project for the next round when dropped.
struct Free(
    std::sync::Arc<std::sync::Mutex<std::collections::HashSet<agenium_core::ProjectId>>>,
    agenium_core::ProjectId,
);

impl Drop for Free {
    fn drop(&mut self) {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.1);
    }
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
                let (o, busy) = (o.clone(), d.busy.clone());
                tauri::async_runtime::spawn(async move {
                    // The project is freed on every way out, a panic included: one bad round must not stop its
                    // team until the app restarts.
                    let _free = Free(busy, p.id);
                    if let Err(e) = o.rt.work(p.id).await {
                        tracing::warn!(project = %p.name, error = %e, "team round failed");
                    }
                });
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::app_page;

    #[test]
    fn only_the_apps_own_pages_load_in_the_window() {
        let u = |s: &str| tauri::Url::parse(s).unwrap();
        let dev = u("http://localhost:5173");
        for ok in [
            "tauri://localhost/#/live",
            "http://tauri.localhost/index.html",
            "http://localhost:5173/#/ticket/AGN-1",
            "about:blank",
        ] {
            assert!(app_page(&u(ok), Some(&dev)), "{ok}");
        }
        for no in [
            "http://127.0.0.1:8765/web/index.html",
            "http://localhost:5174/",
            "https://example.com/",
            "file:///C:/x.html",
        ] {
            assert!(!app_page(&u(no), Some(&dev)), "{no}");
        }
        assert!(
            !app_page(&u("http://localhost:5173/"), None),
            "no dev server in a release build"
        );
    }
}
