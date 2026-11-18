//! The browser of a run (ADR-158, ADR-165): Edge or Chrome as found on the machine, driven over the DevTools
//! protocol, started on the first browser tool call. A run that reaches the network gets a tab of Agenium's own
//! browser (`shared`): one profile kept across runs, signed in where the user signed in. Any other run gets a private
//! headless browser with a fresh profile in the run's temp folder, ended with the run, that reaches only the machine
//! itself and the project's network allowlist: every other host goes to a closed proxy port and fails. Both run
//! outside the sandbox, as MCP servers do.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::Mutex;
use tokio_tungstenite::tungstenite::Message as WsMessage;

use crate::ToolError;

mod find;
mod shared;
pub use find::find_browser;
pub use shared::{shutdown, shutdown_all};

/// How long the browser has to start and say where its DevTools listen.
const START: Duration = Duration::from_secs(20);
/// How long one protocol call may take (a page load waits in steps of it).
const CALL: Duration = Duration::from_secs(30);
/// How long the page may keep changing after it loaded before a step reads it anyway.
const QUIET_CAP_MS: u32 = 3000;

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// The run's browser, started on first use.
#[derive(Default, Clone)]
pub struct Session(Arc<Mutex<Option<Browser>>>);

enum Owner {
    /// The run's own browser, ended with it.
    Private {
        /// Taken when the run ends: ended off the run's thread (see `Drop`).
        proc: Option<Box<PrivateProc>>,
    },
    /// A tab of Agenium's own browser, closed with the run.
    Shared {
        profile: PathBuf,
        port: u16,
        gen: u64,
        id: String,
    },
}

/// The run's private browser process and its profile folder.
struct PrivateProc {
    child: tokio::process::Child,
    _tree: agenium_sandbox::TreeGuard,
    /// Removed after the browser exited (on Windows it stays locked until then).
    _profile: tempfile::TempDir,
}

pub struct Browser {
    owner: Owner,
    ws: Ws,
    next: u64,
    /// The connection to the page is gone (the browser closed or started again).
    broken: bool,
    /// Console errors and uncaught exceptions of the page since the last tool call read them.
    pub errors: Vec<String>,
    /// The page's address after the last step: where a new tab of the run opens again.
    pub url: String,
    /// The page is in a window on the user's screen (Agenium's own browser, shown for the user to act).
    pub visible: bool,
}

impl Session {
    /// The run's page: a tab of Agenium's browser of `profile` (opened again at the same address when that browser
    /// started again), or the run's private browser fenced to `allow`, started when it is not yet.
    pub async fn get(
        &self,
        temp: &Path,
        allow: &[String],
        profile: Option<&Path>,
    ) -> Result<tokio::sync::MappedMutexGuard<'_, Browser>, ToolError> {
        let mut g = self.0.lock().await;
        match profile {
            Some(p) => {
                let (port, gen, visible) = shared::port(p, None).await?;
                let current = g.as_ref().is_some_and(|b| {
                    !b.broken && matches!(&b.owner, Owner::Shared { gen: n, .. } if *n == gen)
                });
                if !current {
                    let url = g.take().map(|b| b.url.clone()).unwrap_or_default();
                    *g = Some(Browser::tab(p, port, gen, visible, &url).await?);
                }
            }
            None if g.is_none() => *g = Some(Browser::start(temp, allow).await?),
            None => {}
        }
        Ok(tokio::sync::MutexGuard::map(g, |b| b.as_mut().unwrap()))
    }

    /// Shows the run's page in a window of Agenium's browser of `profile`, for the user to act in it; its address.
    pub async fn show(&self, profile: &Path) -> Result<String, ToolError> {
        let mut g = self.0.lock().await;
        let url = g
            .as_ref()
            .map(|b| b.url.clone())
            .filter(|u| !u.is_empty() && u != "about:blank")
            .ok_or_else(|| err("invalid_arguments", "open the page first"))?;
        *g = None;
        let (port, gen, visible) = shared::port(profile, Some(true)).await?;
        let b = Browser::tab(profile, port, gen, visible, &url).await?;
        if let Owner::Shared { id, .. } = &b.owner {
            shared::activate(port, id).await;
        }
        *g = Some(b);
        Ok(url)
    }
}

pub(crate) fn err(code: &str, m: impl std::fmt::Display) -> ToolError {
    ToolError::new(code, m.to_string())
}

/// Starts the browser with `extra` arguments, what it prints going to the file `log`; it, its process tree's guard and
/// its DevTools port. Not a pipe: a reader of one (a blocking thread on Windows) would hold the runtime's shutdown
/// for as long as Agenium's own browser lives.
pub(super) async fn launch(
    extra: &[String],
    log: &Path,
) -> Result<(tokio::process::Child, agenium_sandbox::TreeGuard, u16), ToolError> {
    let exe = find_browser().ok_or_else(|| {
        err(
            "no_browser",
            "no Edge or Chrome found on this machine (set AGENIUM_BROWSER to a Chromium browser's program)",
        )
    })?;
    let mut cmd = tokio::process::Command::new(&exe);
    cmd.args([
        "--remote-debugging-port=0",
        "--no-first-run",
        "--no-default-browser-check",
        "--disable-extensions",
        "--disable-sync",
        "--disable-background-networking",
        "--disable-component-update",
        "--window-size=1280,800",
    ])
    .args(extra)
    // On Linux the first cookie of a page waited forever for a keyring nobody could unlock (no screen, or a locked
    // one): the browser keeps its own. One store for every start, so a sign-in made in the window reads headless too.
    .args(if cfg!(target_os = "linux") {
        &["--password-store=basic"][..]
    } else {
        &[][..]
    })
    // A Windows compatibility layer inherited from whatever started Agenium (`__COMPAT_LAYER=DetectorsAppHealth` seen
    // 2026-11-19) makes Edge relaunch itself without it and exit: the process watched here ended with no DevTools.
    .env_remove("__COMPAT_LAYER")
    .stdin(std::process::Stdio::null())
    .stdout(std::process::Stdio::null())
    .stderr(
        std::fs::File::create(log)
            .map_err(|e| err("browser", format!("{}: {e}", log.display())))?,
    )
    .kill_on_drop(true);
    #[cfg(unix)]
    cmd.process_group(0);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    let mut child = cmd
        .spawn()
        .map_err(|e| err("browser", format!("{}: {e}", exe.display())))?;
    let tree = agenium_sandbox::TreeGuard::attach(&child);
    // The browser says where its DevTools listen: "DevTools listening on ws://127.0.0.1:<port>/...".
    let deadline = tokio::time::Instant::now() + START;
    loop {
        let said = std::fs::read_to_string(log).unwrap_or_default();
        if let Some(port) = said
            .split("ws://127.0.0.1:")
            .nth(1)
            .and_then(|rest| rest.split('/').next())
            .and_then(|p| p.parse::<u16>().ok())
        {
            return Ok((child, tree, port));
        }
        let ended = !matches!(child.try_wait(), Ok(None));
        if ended || tokio::time::Instant::now() >= deadline {
            // What it said before it gave up: the reason, when it does not start.
            let lines: Vec<String> = said.lines().take(40).map(str::to_owned).collect();
            return Err(err("browser", not_started(&exe, &lines)));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

impl Browser {
    async fn start(temp: &Path, allow: &[String]) -> Result<Browser, ToolError> {
        std::fs::create_dir_all(temp).map_err(|e| err("browser", e))?;
        let profile = tempfile::Builder::new()
            .prefix("browser-")
            .tempdir_in(temp)
            .map_err(|e| err("browser", e))?;
        // Every host but the machine itself and the allowlist goes to a closed port (port 9, discard): its request fails.
        let mut bypass = vec![
            "localhost".to_owned(),
            "127.0.0.1".to_owned(),
            "[::1]".to_owned(),
            "*.localhost".to_owned(),
        ];
        bypass.extend(
            allow
                .iter()
                .map(|a| a.trim().to_owned())
                .filter(|a| !a.is_empty()),
        );
        let (child, tree, port) = launch(
            &[
                "--headless=new".into(),
                "--mute-audio".into(),
                "--proxy-server=http://127.0.0.1:9".into(),
                format!("--proxy-bypass-list={}", bypass.join(";")),
                format!("--user-data-dir={}", profile.path().display()),
                "about:blank".into(),
            ],
            &profile.path().join("browser.log"),
        )
        .await?;
        let page = page_socket(port).await?;
        let owner = Owner::Private {
            proc: Some(Box::new(PrivateProc {
                child,
                _tree: tree,
                _profile: profile,
            })),
        };
        Browser::connect(owner, &page, false).await
    }

    /// A new tab of Agenium's browser, at `url` when one is given (the run's page before that browser started again).
    async fn tab(
        profile: &Path,
        port: u16,
        gen: u64,
        visible: bool,
        url: &str,
    ) -> Result<Browser, ToolError> {
        let (id, ws) = shared::new_tab(port).await?;
        shared::opened(profile);
        let owner = Owner::Shared {
            profile: profile.to_path_buf(),
            port,
            gen,
            id,
        };
        let mut b = Browser::connect(owner, &ws, visible).await?;
        if !url.is_empty() && url != "about:blank" {
            b.call("Page.navigate", json!({ "url": url })).await?;
            b.url = url.to_owned();
            b.settle().await?;
        }
        Ok(b)
    }

    async fn connect(owner: Owner, page: &str, visible: bool) -> Result<Browser, ToolError> {
        let ws = tokio_tungstenite::connect_async(page)
            .await
            .map_err(|e| err("browser", format!("DevTools: {e}")));
        // A tab that cannot be reached is closed again with `owner` (its `Drop`).
        let (ws, _) = match ws {
            Ok(w) => w,
            Err(e) => {
                drop(owner);
                return Err(e);
            }
        };
        let mut b = Browser {
            owner,
            ws,
            next: 0,
            broken: false,
            errors: vec![],
            url: String::new(),
            visible,
        };
        b.call("Runtime.enable", json!({})).await?;
        b.call("Page.enable", json!({})).await?;
        Ok(b)
    }

    /// One protocol call on the page; events met while waiting are kept (console errors, exceptions).
    pub async fn call(&mut self, method: &str, params: Value) -> Result<Value, ToolError> {
        let r = self.call_inner(method, params).await;
        // The connection, not the call, failed: the page is gone (the browser closed or started again).
        if let Err(e) = &r {
            if e.code == "browser" && !e.message.starts_with(method) {
                self.broken = true;
                // Another run showed the browser to the user, which starts it again: said so, not a bare failure.
                if matches!(self.owner, Owner::Shared { .. }) {
                    return Err(err(
                        "browser_restarted",
                        "Agenium's browser started again (it was shown to the user, or closed): do the step again; \
                         your page opens again at its last address",
                    ));
                }
            }
        }
        r
    }

    async fn call_inner(&mut self, method: &str, params: Value) -> Result<Value, ToolError> {
        self.next += 1;
        let id = self.next;
        let msg = json!({ "id": id, "method": method, "params": params }).to_string();
        self.ws
            .send(WsMessage::text(msg))
            .await
            .map_err(|e| err("browser", e))?;
        let deadline = tokio::time::Instant::now() + CALL;
        loop {
            let m = tokio::time::timeout_at(deadline, self.ws.next())
                .await
                .map_err(|_| {
                    err(
                        "browser_timeout",
                        format!("{method} took longer than {}s", CALL.as_secs()),
                    )
                })?
                .ok_or_else(|| err("browser", "the browser closed"))?
                .map_err(|e| err("browser", e))?;
            let WsMessage::Text(t) = m else { continue };
            let v: Value = serde_json::from_str(t.as_str()).unwrap_or(Value::Null);
            if v["id"] == id {
                if let Some(e) = v.get("error") {
                    return Err(err(
                        "browser",
                        format!("{method}: {}", e["message"].as_str().unwrap_or("failed")),
                    ));
                }
                return Ok(v["result"].clone());
            }
            self.keep_event(&v);
        }
    }

    fn keep_event(&mut self, v: &Value) {
        let p = &v["params"];
        let text = match v["method"].as_str() {
            Some("Runtime.exceptionThrown") => p["exceptionDetails"]["exception"]["description"]
                .as_str()
                .or(p["exceptionDetails"]["text"].as_str())
                .map(str::to_owned),
            Some("Runtime.consoleAPICalled") if p["type"] == "error" => Some(
                p["args"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|a| {
                        a["value"]
                            .as_str()
                            .map(str::to_owned)
                            .unwrap_or_else(|| a["description"].as_str().unwrap_or("").to_owned())
                    })
                    .collect::<Vec<_>>()
                    .join(" "),
            ),
            _ => None,
        };
        if let Some(t) = text.filter(|t| !t.is_empty()) {
            if self.errors.len() < 20 {
                self.errors.push(t.chars().take(300).collect());
            }
        }
    }

    /// Evaluates `expression` in the page and returns its value (awaiting a promise).
    pub async fn eval(&mut self, expression: &str) -> Result<Value, ToolError> {
        let r = self
            .call(
                "Runtime.evaluate",
                json!({ "expression": expression, "returnByValue": true, "awaitPromise": true }),
            )
            .await?;
        if let Some(e) = r.get("exceptionDetails") {
            let d = e["exception"]["description"]
                .as_str()
                .or(e["text"].as_str())
                .unwrap_or("failed");
            return Err(err("script_error", d.chars().take(500).collect::<String>()));
        }
        Ok(r["result"]["value"].clone())
    }

    /// Waits until the page has loaded (or `CALL` passed), then until it stops changing for a moment (at most
    /// `QUIET_CAP_MS`): a page that renders after its load is read as it shows, not half built.
    pub async fn settle(&mut self) -> Result<(), ToolError> {
        let deadline = tokio::time::Instant::now() + CALL;
        while tokio::time::Instant::now() < deadline {
            match self.eval("document.readyState").await {
                Ok(v) if v == "complete" => break,
                // A page being replaced (a navigation) has no context for a moment.
                Err(e) if self.broken => return Err(e),
                _ => {}
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        let quiet = format!(
            "new Promise((done) => {{ let t; const o = new MutationObserver(() => {{ clearTimeout(t); t = setTimeout(end, 300); }});
              const end = () => {{ o.disconnect(); done(true); }};
              o.observe(document, {{ subtree: true, childList: true, attributes: true, characterData: true }});
              t = setTimeout(end, 300); setTimeout(end, {QUIET_CAP_MS}); }})"
        );
        match self.eval(&quiet).await {
            Err(e) if self.broken => Err(e),
            _ => Ok(()),
        }
    }
}

impl Drop for Browser {
    // A private profile folder is removed once the browser exited (on Windows it stays locked until then): with a
    // runtime, that wait is not the run's.
    fn drop(&mut self) {
        let Owner::Private { proc } = &mut self.owner else {
            return;
        };
        let Some(mut p) = proc.take() else { return };
        let _ = p.child.start_kill();
        if let Ok(rt) = tokio::runtime::Handle::try_current() {
            rt.spawn(async move {
                let _ = tokio::time::timeout(Duration::from_secs(2), p.child.wait()).await;
                drop(p);
            });
            return;
        }
        for _ in 0..30 {
            if matches!(p.child.try_wait(), Ok(Some(_)) | Err(_)) {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

impl Drop for Owner {
    fn drop(&mut self) {
        if let Owner::Shared {
            profile, port, id, ..
        } = self
        {
            shared::closed(profile, *port, id);
        }
    }
}

/// Why the browser at `exe` did not start, from what it printed. On Ubuntu 23.10 and later AppArmor keeps a browser
/// without its own profile (a downloaded build, Chrome for Testing) from making its sandbox, and it exits: the browser
/// packaged by the distribution (snap, .deb) has that profile. The sandbox is never turned off here: the pages run the
/// code an agent wrote.
fn not_started(exe: &Path, said: &[String]) -> String {
    let all = said.join("\n");
    if all.contains("No usable sandbox") {
        return format!(
            "{} cannot make its sandbox on this Linux (AppArmor restricts user namespaces, Ubuntu 23.10+): use the \
             Chrome or Chromium your distribution packages (.deb, snap), or set AGENIUM_BROWSER to one",
            exe.display()
        );
    }
    let last = said
        .iter()
        .rev()
        .find(|l| l.contains("FATAL") || l.contains("ERROR"))
        .map(|l| format!(": {}", l.chars().take(300).collect::<String>()))
        .unwrap_or_default();
    format!("{} did not start its DevTools{last}", exe.display())
}

/// The DevTools socket of the browser's first page.
async fn page_socket(port: u16) -> Result<String, ToolError> {
    let http = reqwest::Client::new();
    for _ in 0..50 {
        if let Ok(r) = http
            .get(format!("http://127.0.0.1:{port}/json/list"))
            .send()
            .await
        {
            if let Ok(v) = r
                .text()
                .await
                .map(|t| serde_json::from_str::<Value>(&t).unwrap_or(Value::Null))
            {
                if let Some(ws) = v
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|t| t["type"] == "page")
                    .and_then(|t| t["webSocketDebuggerUrl"].as_str())
                {
                    return Ok(ws.to_owned());
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err(err("browser", "the browser has no page"))
}

/// Whether a page at `url` may be opened: the machine itself, a host of the allowlist, or a file of `root`; with
/// `open` (a run that reaches the network, ADR-165) any public site too, but no other private address.
pub fn url_allowed(
    url: &str,
    allow: &[String],
    root: &Path,
    open: bool,
) -> Result<String, ToolError> {
    let u =
        reqwest::Url::parse(url.trim()).map_err(|e| err("invalid_url", format!("{url}: {e}")))?;
    match u.scheme() {
        "http" | "https" => {
            let host = u
                .host_str()
                .unwrap_or_default()
                .trim_matches(['[', ']'])
                .to_ascii_lowercase();
            let local = host == "localhost"
                || host.ends_with(".localhost")
                || host == "127.0.0.1"
                || host == "::1";
            let public = match host.parse::<std::net::IpAddr>() {
                Ok(ip) => crate::work::public_ip(ip),
                Err(_) => true,
            };
            if local || crate::work::domain_allowed(&host, allow) || (open && public) {
                Ok(u.to_string())
            } else if open {
                Err(err(
                    "address_not_allowed",
                    format!("{host} is on a private network; only a host of the network allowlist opens there"),
                ))
            } else {
                Err(err(
                    "domain_not_allowed",
                    format!("{host} is not this machine nor in the network allowlist; use request_permission (kind=network)"),
                ))
            }
        }
        "file" => {
            let p: PathBuf = u.to_file_path().map_err(|_| err("invalid_url", url))?;
            if crate::paths::confined(&p, root) {
                Ok(u.to_string())
            } else {
                Err(err(
                    "outside_worktree",
                    format!("{} is not a file of your worktree", p.display()),
                ))
            }
        }
        s => Err(err(
            "invalid_url",
            format!("{s}: only http, https and files of your worktree open"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_browser_that_does_not_start_says_why() {
        let exe = Path::new("/home/u/cft/chrome");
        let sandbox = ["[1:1:FATAL:zygote_host_impl_linux.cc:144] No usable sandbox! If you are running on Ubuntu 23.10+".into()];
        assert!(not_started(exe, &sandbox).contains("AppArmor"));
        let other = [
            "starting".into(),
            "[2:2:ERROR:gpu.cc:1] cannot open display".into(),
        ];
        assert!(not_started(exe, &other).ends_with("cannot open display"));
        assert!(not_started(exe, &[]).ends_with("did not start its DevTools"));
    }
}
