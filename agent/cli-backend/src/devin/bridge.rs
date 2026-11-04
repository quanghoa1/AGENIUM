//! The Devin bridge (ADR-156): `devin acp` driven over JSON-RPC on its stdio. It opens or loads the session, sends each
//! input line as a prompt, passes Devin's `session/update`s on (with each tool's name and input), and answers Devin's
//! requests itself: a file to read or write (`fs/*`) and a permission, each after asking Agenium (the hook endpoint);
//! a hook that cannot be reached refuses.

use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc as smpsc, Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::sync::mpsc;

use super::{MCP_NAME_ENV, MCP_URL_ENV};
use crate::driver::{HOOK_TOKEN_ENV, HOOK_URL_ENV};

/// How long Devin may take to start, open a session or set its mode.
const SETUP: Duration = Duration::from_secs(120);
/// A prompt ends with its turn; the run's own limits end a turn that never does.
const TURN: Duration = Duration::from_secs(24 * 3600);

struct Shared {
    stdin: Mutex<Option<Box<dyn Write + Send>>>,
    pending: Mutex<HashMap<u64, smpsc::Sender<Value>>>,
    next: AtomicU64,
    /// While a session loads, Devin replays its history as updates: not this turn's, not passed on.
    loading: AtomicBool,
    /// Each tool call's name and input, for its later updates (which carry neither).
    tools: Mutex<HashMap<String, (String, Value)>>,
}

impl Shared {
    fn send(&self, v: &Value) -> bool {
        let mut g = self.stdin.lock().unwrap_or_else(|p| p.into_inner());
        let Some(w) = g.as_mut() else { return false };
        w.write_all(format!("{v}\n").as_bytes())
            .and_then(|_| w.flush())
            .is_ok()
    }

    fn request(&self, method: &str, params: Value, wait: Duration) -> Result<Value, String> {
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = smpsc::channel();
        self.pending
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(id, tx);
        if !self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})) {
            return Err(format!("devin is gone ({method})"));
        }
        let r = rx
            .recv_timeout(wait)
            .map_err(|_| format!("devin gave no answer to {method}"))?;
        match r.get("error") {
            Some(e) if !e.is_null() => Err(format!(
                "{method}: {}",
                e["message"].as_str().unwrap_or("error")
            )),
            _ => Ok(r["result"].clone()),
        }
    }
}

/// Runs the ACP conversation with a `devin acp` process (see the module). Ends when `input` closes.
#[allow(clippy::too_many_arguments)]
pub(crate) fn bridge(
    stdin: Box<dyn Write + Send>,
    stdout: Box<dyn Read + Send>,
    mut input: mpsc::UnboundedReceiver<String>,
    out: mpsc::UnboundedSender<Value>,
    env: &BTreeMap<String, String>,
    worktree: PathBuf,
    tail: Arc<Mutex<Vec<u8>>>,
    resume: Option<String>,
) {
    let get = |k: &str| env.get(k).cloned().unwrap_or_default();
    let (hook, token, mcp_url, mcp_name) = (
        get(HOOK_URL_ENV),
        get(HOOK_TOKEN_ENV),
        get(MCP_URL_ENV),
        get(MCP_NAME_ENV),
    );
    let rt = tokio::runtime::Handle::current();
    let shared = Arc::new(Shared {
        stdin: Mutex::new(Some(stdin)),
        pending: Mutex::default(),
        next: AtomicU64::new(1),
        loading: AtomicBool::new(false),
        tools: Mutex::default(),
    });
    let ask = Arc::new(Asker {
        rt: rt.clone(),
        hook,
        token,
    });

    // Devin's output: answers to Agenium's requests, its updates, its own requests.
    {
        let (shared, out, ask, tail) = (shared.clone(), out.clone(), ask.clone(), tail.clone());
        std::thread::spawn(move || {
            let mut r = BufReader::new(stdout);
            let mut line = Vec::new();
            loop {
                line.clear();
                match r.read_until(b'\n', &mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
                crate::run::keep_tail(&tail, &line);
                let Ok(msg) = serde_json::from_slice::<Value>(&line) else {
                    continue;
                };
                if let Some(method) = msg["method"].as_str() {
                    if msg.get("id").is_some_and(|i| !i.is_null()) {
                        let (shared, ask, msg) = (shared.clone(), ask.clone(), msg.clone());
                        std::thread::spawn(move || {
                            let answer = answer(&ask, &msg);
                            let mut reply = json!({"jsonrpc": "2.0", "id": msg["id"]});
                            match answer {
                                Ok(v) => reply["result"] = v,
                                Err(e) => reply["error"] = json!({"code": -32000, "message": e}),
                            }
                            shared.send(&reply);
                        });
                    } else if method == "session/update" && !shared.loading.load(Ordering::SeqCst) {
                        let mut u = msg["params"]["update"].clone();
                        named(&shared, &mut u);
                        let _ = out.send(json!({"type": "update", "update": u}));
                    }
                } else if let Some(id) = msg["id"].as_u64() {
                    if let Some(tx) = shared
                        .pending
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .remove(&id)
                    {
                        let _ = tx.send(msg);
                    }
                }
            }
            // Devin ended: whoever waits for an answer stops waiting.
            shared
                .pending
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clear();
        });
    }

    // The conversation: the session, then each input line as one prompt.
    std::thread::spawn(move || {
        // With what Devin last printed (its stderr among it): why it did not answer is there.
        let fail = |e: String| {
            let said = String::from_utf8_lossy(&tail.lock().unwrap_or_else(|p| p.into_inner()))
                .into_owned();
            let said: String = said
                .chars()
                .rev()
                .take(600)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            let e = if said.trim().is_empty() {
                e
            } else {
                format!("{e}: {}", said.trim())
            };
            let _ = out.send(json!({"type": "failed", "error": e}));
        };
        let caps = json!({"fs": {"readTextFile": true, "writeTextFile": true}, "terminal": false});
        if let Err(e) = shared.request(
            "initialize",
            json!({"protocolVersion": 1, "clientCapabilities": caps}),
            SETUP,
        ) {
            return fail(e);
        }
        let cwd = worktree.display().to_string();
        let servers = json!([{
            "type": "http",
            "name": mcp_name,
            "url": mcp_url,
            "headers": [{"name": "Authorization", "value": format!("Bearer {}", ask.token)}],
        }]);
        let loaded = resume.filter(|id| {
            shared.loading.store(true, Ordering::SeqCst);
            let ok = shared
                .request(
                    "session/load",
                    json!({"sessionId": id, "cwd": cwd, "mcpServers": servers}),
                    SETUP,
                )
                .is_ok();
            shared.loading.store(false, Ordering::SeqCst);
            ok
        });
        let sid = match loaded {
            Some(id) => id,
            None => match shared.request(
                "session/new",
                json!({"cwd": cwd, "mcpServers": servers}),
                SETUP,
            ) {
                Ok(r) => match r["sessionId"].as_str() {
                    Some(s) => s.to_owned(),
                    None => return fail("devin opened no session".into()),
                },
                Err(e) => return fail(e),
            },
        };
        let _ = out.send(json!({"type": "session", "id": sid}));
        // Its YOLO mode: the sandbox bounds what it runs, and Agenium answers every file it hands over.
        let _ = shared.request(
            "session/set_mode",
            json!({"sessionId": sid, "modeId": "bypass"}),
            SETUP,
        );
        while let Some(line) = input.blocking_recv() {
            let text = serde_json::from_str::<Value>(line.trim())
                .ok()
                .and_then(|v| v["text"].as_str().map(str::to_owned))
                .unwrap_or_else(|| line.trim().to_owned());
            let prompt = json!({"sessionId": sid, "prompt": [{"type": "text", "text": text}]});
            match shared.request("session/prompt", prompt, TURN) {
                Ok(r) => {
                    let _ = out.send(json!({"type": "turn", "result": r}));
                }
                Err(e) => fail(e),
            }
        }
        // Input closed: Devin's stdin with it, so it ends.
        shared
            .stdin
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take();
    });
}

/// Puts a tool call's name and input on its update (Devin sends them on the first only).
fn named(shared: &Shared, u: &mut Value) {
    let Some(id) = u["toolCallId"].as_str().map(str::to_owned) else {
        return;
    };
    let mut tools = shared.tools.lock().unwrap_or_else(|p| p.into_inner());
    match u["sessionUpdate"].as_str() {
        Some("tool_call") => {
            tools.insert(id, (super::tool_name(u), u["rawInput"].clone()));
        }
        Some("tool_call_update") => {
            if let Some((name, input)) = tools.get(&id) {
                if u["_meta"]["cognition.ai/inferenceToolName"].is_null() {
                    u["_meta"]["cognition.ai/inferenceToolName"] = json!(name);
                }
                if u["rawInput"].is_null() {
                    u["rawInput"] = input.clone();
                }
            }
        }
        _ => {}
    }
}

/// Asks Agenium's hook endpoint about one tool call (`{"decision": "allow"|"deny", "reason"}`).
struct Asker {
    rt: tokio::runtime::Handle,
    hook: String,
    token: String,
}

impl Asker {
    fn allowed(&self, tool: &str, input: Value) -> Result<(), String> {
        let payload = json!({"tool_name": tool, "tool_input": input});
        let (hook, token) = (self.hook.clone(), self.token.clone());
        let decision = self.rt.block_on(async move {
            let http = reqwest::Client::new();
            match http.post(&hook).bearer_auth(&token).json(&payload).send().await {
                Ok(r) => r.json::<Value>().await.unwrap_or(Value::Null),
                Err(e) => json!({"decision": "deny", "reason": format!("Agenium could not be asked: {e}")}),
            }
        });
        if decision["decision"] == "allow" {
            Ok(())
        } else {
            Err(decision["reason"]
                .as_str()
                .unwrap_or("refused by Agenium")
                .to_owned())
        }
    }
}

/// Agenium's answer to one request of Devin's.
fn answer(ask: &Asker, msg: &Value) -> Result<Value, String> {
    let p = &msg["params"];
    match msg["method"].as_str().unwrap_or("") {
        "fs/read_text_file" => {
            let path = p["path"].as_str().unwrap_or("");
            ask.allowed("read", json!({"path": path}))?;
            let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
            Ok(json!({"content": lines(&text, p["line"].as_u64(), p["limit"].as_u64())}))
        }
        "fs/write_text_file" => {
            let path = p["path"].as_str().unwrap_or("");
            ask.allowed("write", json!({"path": path}))?;
            let file = std::path::Path::new(path);
            if let Some(dir) = file.parent() {
                std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            }
            std::fs::write(file, p["content"].as_str().unwrap_or(""))
                .map_err(|e| format!("{path}: {e}"))?;
            Ok(Value::Null)
        }
        "session/request_permission" => {
            let call = &p["toolCall"];
            let tool = super::tool_name(call);
            let allow = ask.allowed(&tool, call["rawInput"].clone()).is_ok();
            let options = p["options"].as_array().cloned().unwrap_or_default();
            let pick = |kind: &str| {
                options
                    .iter()
                    .find(|o| o["kind"] == kind)
                    .and_then(|o| o["optionId"].as_str())
            };
            let id = if allow {
                pick("allow_once")
            } else {
                pick("reject_once")
            }
            .or_else(|| options.last().and_then(|o| o["optionId"].as_str()))
            .unwrap_or("reject_once");
            Ok(json!({"outcome": {"outcome": "selected", "optionId": id}}))
        }
        m => Err(format!("{m} is not offered by Agenium")),
    }
}

/// `text` from its `line` (1-based) for `limit` lines, as ACP's `fs/read_text_file` asks.
fn lines(text: &str, line: Option<u64>, limit: Option<u64>) -> String {
    if line.is_none() && limit.is_none() {
        return text.to_owned();
    }
    let from = line.unwrap_or(1).saturating_sub(1) as usize;
    let n = limit.map_or(usize::MAX, |l| l as usize);
    text.split_inclusive('\n').skip(from).take(n).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_read_takes_the_lines_asked_for() {
        let t = "a\nb\nc\nd\n";
        assert_eq!(lines(t, None, None), t);
        assert_eq!(lines(t, Some(2), Some(2)), "b\nc\n");
        assert_eq!(lines(t, Some(4), None), "d\n");
        assert_eq!(lines(t, None, Some(1)), "a\n");
    }

    #[test]
    fn a_tools_updates_carry_its_name_and_input() {
        let shared = Shared {
            stdin: Mutex::new(None),
            pending: Mutex::default(),
            next: AtomicU64::new(1),
            loading: AtomicBool::new(false),
            tools: Mutex::default(),
        };
        let mut call = json!({"sessionUpdate": "tool_call", "toolCallId": "exec:0", "rawInput": {"command": "ls"},
            "_meta": {"cognition.ai/inferenceToolName": "exec"}});
        named(&shared, &mut call);
        let mut done = json!({"sessionUpdate": "tool_call_update", "toolCallId": "exec:0", "status": "completed"});
        named(&shared, &mut done);
        assert_eq!(done["_meta"]["cognition.ai/inferenceToolName"], "exec");
        assert_eq!(done["rawInput"]["command"], "ls");
    }
}
