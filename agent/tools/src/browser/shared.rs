//! Agenium's own browser (ADR-165): one Edge or Chrome per profile folder, shared by the runs that reach the network,
//! each run in a tab of its own, its cookies and sign-ins kept in the profile like a person's browser. Headless while
//! agents use it; shown in a window when the user is to act in it (sign in, sign up, a CAPTCHA, a second factor), and
//! headless again once the user closes that window. Closed gracefully (its cookies written) when no run has used it
//! for `IDLE`, or before it starts again with or without its window. A browser left by an earlier Agenium on the same
//! profile (it ended without closing it) is used as it is: its DevTools port is in the profile.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::Message as WsMessage;

use super::{err, launch};
use crate::ToolError;

/// How long the browser stays without a tab of a run before it is closed.
const IDLE: Duration = Duration::from_secs(5 * 60);

struct Proc {
    /// `None`: a browser an earlier Agenium left running.
    child: Option<tokio::process::Child>,
    _tree: Option<agenium_sandbox::TreeGuard>,
    port: u16,
    gen: u64,
    visible: bool,
}

fn procs() -> &'static tokio::sync::Mutex<HashMap<PathBuf, Proc>> {
    static P: OnceLock<tokio::sync::Mutex<HashMap<PathBuf, Proc>>> = OnceLock::new();
    P.get_or_init(Default::default)
}

/// Tabs of runs open in each profile's browser, and a count of uses that an idle wait checks it was not passed by
/// (kept apart from `procs`: a tab is closed in a `Drop`).
fn tabs() -> &'static std::sync::Mutex<HashMap<PathBuf, (usize, u64)>> {
    static T: OnceLock<std::sync::Mutex<HashMap<PathBuf, (usize, u64)>>> = OnceLock::new();
    T.get_or_init(Default::default)
}

/// `profile`'s browser is being used: an idle wait started before ends without closing it.
fn used(profile: &Path) {
    tabs()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry(profile.to_path_buf())
        .or_default()
        .1 += 1;
}

static GEN: AtomicU64 = AtomicU64::new(0);

/// The DevTools port of `profile`'s browser, started (`visible`: with its window) when it does not run, or started
/// again when it runs the other way; with its generation (a new one each start) and whether it shows its window.
pub(super) async fn port(
    profile: &Path,
    visible: Option<bool>,
) -> Result<(u16, u64, bool), ToolError> {
    let mut m = procs().lock().await;
    // Counted before its tab opens: an idle wait ending in between does not close the browser under it.
    used(profile);
    if let Some(p) = m.get_mut(profile) {
        let alive = match &mut p.child {
            Some(c) => matches!(c.try_wait(), Ok(None)),
            None => answers(p.port).await,
        };
        if alive && visible.is_none_or(|v| v == p.visible) {
            return Ok((p.port, p.gen, p.visible));
        }
        if alive {
            close(p.port).await;
        }
        m.remove(profile);
    }
    let visible = visible.unwrap_or(false);
    if let Some((port, headless)) = left_running(profile).await {
        // A window left open is the user's: used as it is, and never closed for being idle.
        if !visible || !headless {
            let gen = GEN.fetch_add(1, Ordering::Relaxed) + 1;
            m.insert(
                profile.to_path_buf(),
                Proc {
                    child: None,
                    _tree: None,
                    port,
                    gen,
                    visible: !headless,
                },
            );
            return Ok((port, gen, !headless));
        }
        close(port).await;
    }
    if visible && !has_screen() {
        return Err(err(
            "no_screen",
            "this machine has no screen for a browser window (no display): the user cannot act in the page here",
        ));
    }
    std::fs::create_dir_all(profile).map_err(|e| err("browser", e))?;
    // Its cookies are the user's sign-ins: the folder is the user's alone.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(profile, std::fs::Permissions::from_mode(0o700));
    }
    let mut args: Vec<String> = Vec::new();
    if !visible {
        args.extend(["--headless=new".into(), "--mute-audio".into()]);
    }
    args.push(format!("--user-data-dir={}", profile.display()));
    let (child, tree, port) = launch(&args, &profile.join("browser.log")).await?;
    let gen = GEN.fetch_add(1, Ordering::Relaxed) + 1;
    m.insert(
        profile.to_path_buf(),
        Proc {
            child: Some(child),
            _tree: Some(tree),
            port,
            gen,
            visible,
        },
    );
    Ok((port, gen, visible))
}

/// Closes `profile`'s browser, its cookies written first.
pub async fn shutdown(profile: &Path) {
    let mut m = procs().lock().await;
    if let Some(p) = m.remove(profile) {
        close(p.port).await;
    }
}

/// Closes every browser of this process, their cookies written first (Agenium quits).
pub async fn shutdown_all() {
    let mut m = procs().lock().await;
    for (_, p) in m.drain() {
        close(p.port).await;
    }
}

/// A tab of a run opened in `profile`'s browser; closing a run's last tab starts the idle wait.
pub(super) fn opened(profile: &Path) {
    let mut t = tabs().lock().unwrap_or_else(|e| e.into_inner());
    let e = t.entry(profile.to_path_buf()).or_default();
    e.0 += 1;
    e.1 += 1;
}

pub(super) fn closed(profile: &Path, port: u16, id: &str) {
    let (left, uses) = {
        let mut t = tabs().lock().unwrap_or_else(|e| e.into_inner());
        let e = t.entry(profile.to_path_buf()).or_default();
        e.0 = e.0.saturating_sub(1);
        *e
    };

    // This runs in a `Drop`: with a runtime, nothing here waits on the browser.
    let Ok(rt) = tokio::runtime::Handle::try_current() else {
        let _ = get_sync(port, &format!("/json/close/{id}"));
        return;
    };
    let (profile, id) = (profile.to_path_buf(), id.to_owned());
    rt.spawn(async move {
        let _ = reqwest::Client::new()
            .get(format!("http://127.0.0.1:{port}/json/close/{id}"))
            .timeout(Duration::from_secs(2))
            .send()
            .await;
        if left > 0 {
            return;
        }
        tokio::time::sleep(IDLE).await;
        let mut m = procs().lock().await;
        // Unused since this wait began (no tab open, no run asked for it), and not the user's window.
        let unused = tabs()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&profile)
            .is_none_or(|e| e.0 == 0 && e.1 == uses);
        if unused && m.get(&profile).is_some_and(|p| !p.visible) {
            if let Some(p) = m.remove(&profile) {
                close(p.port).await;
            }
        }
    });
}

/// A new tab: its target id and DevTools socket.
pub(super) async fn new_tab(port: u16) -> Result<(String, String), ToolError> {
    let r = reqwest::Client::new()
        .put(format!("http://127.0.0.1:{port}/json/new?about:blank"))
        .send()
        .await
        .map_err(|e| err("browser", format!("a new tab: {e}")))?
        .text()
        .await
        .map_err(|e| err("browser", e))?;
    let v: Value = serde_json::from_str(&r).unwrap_or(Value::Null);
    match (v["id"].as_str(), v["webSocketDebuggerUrl"].as_str()) {
        (Some(id), Some(ws)) => Ok((id.to_owned(), ws.to_owned())),
        _ => Err(err("browser", format!("a new tab: {r}"))),
    }
}

/// Brings tab `id` to the front of the window.
pub(super) async fn activate(port: u16, id: &str) {
    let _ = reqwest::Client::new()
        .get(format!("http://127.0.0.1:{port}/json/activate/{id}"))
        .send()
        .await;
}

async fn answers(port: u16) -> bool {
    reqwest::Client::new()
        .get(format!("http://127.0.0.1:{port}/json/version"))
        .timeout(Duration::from_secs(2))
        .send()
        .await
        .is_ok_and(|r| r.status().is_success())
}

/// The port of a browser still running on `profile` (it writes the port to `DevToolsActivePort`), and whether it is
/// headless (its user agent says so).
async fn left_running(profile: &Path) -> Option<(u16, bool)> {
    let text = std::fs::read_to_string(profile.join("DevToolsActivePort")).ok()?;
    let port: u16 = text.lines().next()?.trim().parse().ok()?;
    let v: Value = reqwest::Client::new()
        .get(format!("http://127.0.0.1:{port}/json/version"))
        .timeout(Duration::from_secs(2))
        .send()
        .await
        .ok()?
        .text()
        .await
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())?;
    let headless = v["User-Agent"]
        .as_str()
        .is_some_and(|a| a.contains("HeadlessChrome"));
    Some((port, headless))
}

/// Asks the browser to close (it writes its cookies and session), then waits for its port to go quiet.
async fn close(port: u16) {
    let ws = async {
        let r = reqwest::Client::new()
            .get(format!("http://127.0.0.1:{port}/json/version"))
            .timeout(Duration::from_secs(2))
            .send()
            .await
            .ok()?
            .text()
            .await
            .ok()?;
        let v: Value = serde_json::from_str(&r).ok()?;
        v["webSocketDebuggerUrl"].as_str().map(str::to_owned)
    }
    .await;
    if let Some(ws) = ws {
        if let Ok((mut s, _)) = tokio_tungstenite::connect_async(ws.as_str()).await {
            let _ = s
                .send(WsMessage::text(
                    json!({"id": 1, "method": "Browser.close"}).to_string(),
                ))
                .await;
            let _ = tokio::time::timeout(Duration::from_secs(2), s.next()).await;
        }
    }
    for _ in 0..50 {
        if !answers(port).await {
            // Its profile lock goes with the last process.
            tokio::time::sleep(Duration::from_millis(500)).await;
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// A plain HTTP GET to the browser's DevTools (no runtime needed).
fn get_sync(port: u16, path: &str) -> std::io::Result<()> {
    use std::io::{Read, Write};
    let mut s = std::net::TcpStream::connect(("127.0.0.1", port))?;
    s.set_read_timeout(Some(Duration::from_secs(2)))?;
    s.write_all(
        format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n").as_bytes(),
    )?;
    let mut buf = [0u8; 512];
    let _ = s.read(&mut buf);
    Ok(())
}

/// Whether a window can be shown: always on Windows and macOS; on Linux with a display.
fn has_screen() -> bool {
    if cfg!(target_os = "linux") {
        ["DISPLAY", "WAYLAND_DISPLAY"]
            .iter()
            .any(|v| std::env::var_os(v).is_some_and(|s| !s.is_empty()))
    } else {
        true
    }
}
