//! The preview browser (ADR-121): webviews inside the main window, one per tab, that show the app the agents build
//! (`http://localhost:3000` …) or any web page the user wants to point at, with an element picker. A page gets no
//! access to Agenium: no capability names these webviews and their origin is remote, so it cannot call a command. A
//! picked element leaves the page as a navigation to `http://agenium-pick.invalid/?p=<json>`, which is cancelled here
//! and handed to the main webview as an event. The app drives a page only with the fixed scripts below (history,
//! reload, picker on/off), never arbitrary code.

use tauri::{
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, Url, WebviewBuilder, WebviewUrl,
};

/// Webview labels of the tabs: `preview-<tab>`.
pub const PREFIX: &str = "preview-";
const PICK_HOST: &str = "agenium-pick.invalid";
const PICKER: &str = include_str!("browser_picker.js");
/// A pick larger than this is dropped (the page builds it, so it is bounded here too).
const MAX_PICK: usize = 64 * 1024;

type R<T> = Result<T, String>;

/// http(s) pages other than the app's own origins (a page there would run Agenium's interface again).
fn allowed(url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https")
        && !matches!(
            url.host_str(),
            Some("tauri.localhost" | "ipc.localhost" | "asset.localhost")
        )
}

fn parse(url: &str) -> R<Url> {
    let u: Url = url.trim().parse().map_err(|e| format!("not a URL: {e}"))?;
    if !allowed(&u) {
        return Err("only http and https pages open in the browser".into());
    }
    Ok(u)
}

/// The webview label of a tab id the interface made (letters and digits).
fn label(tab: &str) -> R<String> {
    if tab.is_empty() || tab.len() > 16 || !tab.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(format!("bad tab id {tab:?}"));
    }
    Ok(format!("{PREFIX}{tab}"))
}

fn place(wv: &tauri::Webview, x: f64, y: f64, w: f64, h: f64) -> R<()> {
    wv.set_position(LogicalPosition::new(x, y))
        .map_err(|e| e.to_string())?;
    wv.set_size(LogicalSize::new(w.max(1.0), h.max(1.0)))
        .map_err(|e| e.to_string())
}

/// Opens `url` in a tab at the given place of the main window (logical pixels), creating its webview the first time.
/// Async like every command here: creating or driving a webview from a synchronous command deadlocks on Windows.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn browser_open(
    app: AppHandle,
    tab: String,
    url: String,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
) -> R<()> {
    let name = label(&tab)?;
    let url = parse(&url)?;
    if let Some(wv) = app.get_webview(&name) {
        wv.navigate(url).map_err(|e| e.to_string())?;
        place(&wv, x, y, w, h)?;
        return wv.show().map_err(|e| e.to_string());
    }
    let window = app
        .get_window("main")
        .ok_or("the main window is not open")?;
    let handle = app.clone();
    let id = tab.clone();
    let builder = WebviewBuilder::new(&name, WebviewUrl::External(url))
        .initialization_script(PICKER)
        .on_navigation(move |u| {
            if u.host_str() == Some(PICK_HOST) {
                for (k, v) in u.query_pairs() {
                    match k.as_ref() {
                        "p" if v.len() <= MAX_PICK => {
                            let _ = handle.emit_to(
                                "main",
                                "agenium://browser-picked",
                                serde_json::json!({ "tab": id, "pick": v }),
                            );
                        }
                        "off" => {
                            let _ = handle.emit_to(
                                "main",
                                "agenium://browser-pick-off",
                                serde_json::json!({ "tab": id }),
                            );
                        }
                        _ => {}
                    }
                }
                return false;
            }
            allowed(u)
        })
        .on_page_load(|wv, payload| {
            if matches!(payload.event(), tauri::webview::PageLoadEvent::Finished) {
                let tab = wv.label().trim_start_matches(PREFIX).to_owned();
                let _ = wv.emit_to(
                    "main",
                    "agenium://browser-url",
                    serde_json::json!({ "tab": tab, "url": payload.url().to_string() }),
                );
            }
        });
    window
        .add_child(
            builder,
            LogicalPosition::new(x, y),
            LogicalSize::new(w.max(1.0), h.max(1.0)),
        )
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Moves a tab with its place in the interface (the window or the panel was resized).
#[tauri::command]
pub async fn browser_bounds(app: AppHandle, tab: String, x: f64, y: f64, w: f64, h: f64) -> R<()> {
    match app.get_webview(&label(&tab)?) {
        Some(wv) => place(&wv, x, y, w, h),
        None => Ok(()),
    }
}

/// Shows or hides a tab (a native view: it would cover any other page of the interface).
#[tauri::command]
pub async fn browser_visible(app: AppHandle, tab: String, show: bool) -> R<()> {
    match app.get_webview(&label(&tab)?) {
        Some(wv) if show => wv.show().map_err(|e| e.to_string()),
        Some(wv) => wv.hide().map_err(|e| e.to_string()),
        None => Ok(()),
    }
}

/// Hides every tab (the interface left the browser).
#[tauri::command]
pub async fn browser_hide_all(app: AppHandle) -> R<()> {
    for (name, wv) in app.webviews() {
        if name.starts_with(PREFIX) {
            wv.hide().map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// Closes every tab: the interface started afresh (a reload) and knows none of them.
#[tauri::command]
pub async fn browser_close_all(app: AppHandle) -> R<()> {
    for (name, wv) in app.webviews() {
        if name.starts_with(PREFIX) {
            wv.close().map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// Closes a tab and its page.
#[tauri::command]
pub async fn browser_close(app: AppHandle, tab: String) -> R<()> {
    match app.get_webview(&label(&tab)?) {
        Some(wv) => wv.close().map_err(|e| e.to_string()),
        None => Ok(()),
    }
}

/// The page a tab shows, if it is open (the interface asks when it comes back to the browser).
#[tauri::command]
pub async fn browser_url(app: AppHandle, tab: String) -> R<Option<String>> {
    Ok(app
        .get_webview(&label(&tab)?)
        .and_then(|wv| wv.url().ok())
        .map(|u| u.to_string()))
}

/// History, reload and the picker: a fixed script per action.
#[tauri::command]
pub async fn browser_action(app: AppHandle, tab: String, action: String) -> R<()> {
    let js = match action.as_str() {
        "back" => "history.back()",
        "forward" => "history.forward()",
        "reload" => "location.reload()",
        "pick-on" => "window.__ageniumPicker && window.__ageniumPicker.set(true)",
        "pick-off" => "window.__ageniumPicker && window.__ageniumPicker.set(false)",
        other => return Err(format!("unknown browser action {other:?}")),
    };
    match app.get_webview(&label(&tab)?) {
        Some(wv) => wv.eval(js).map_err(|e| e.to_string()),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_web_pages_outside_the_app_open() {
        assert!(parse("http://localhost:3000/cart").is_ok());
        assert!(parse("https://example.com").is_ok());
        for bad in [
            "file:///C:/Windows/win.ini",
            "http://tauri.localhost/",
            "tauri://localhost",
            "javascript:alert(1)",
            "not a url",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
        assert!(label("t1").is_ok());
        for bad in ["", "../main", "main\"", "a-b"] {
            assert!(label(bad).is_err(), "{bad}");
        }
    }
}
