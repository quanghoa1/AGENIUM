//! opencode CLI driver, checked with opencode 2.1.2 on 2026-10-25 (ADR-68). It exists for OpenCode's free models
//! (`opencode/*-free`), which answer only to OpenCode itself; paid OpenCode models run through the API backend.
//!
//! - Transport: `opencode serve` on the host, driven through its HTTP API by an in-process bridge
//!   ([`bridge`]): a session in the ticket worktree, one `prompt` per user message, the server's event stream
//!   (`/api/event`) turned into this driver's events, and the end of a turn read from
//!   `session.execution.succeeded|failed`. `opencode run` cannot be used: it rejects every permission request
//!   itself, and inside Agenium's sandbox it fails at start (it resolves paths with `lstat` on the drive root,
//!   which the container denies).
//! - Isolation: `XDG_CONFIG_HOME`, `XDG_DATA_HOME`, `XDG_STATE_HOME`, `XDG_CACHE_HOME` and opencode's home all point
//!   into the per-run home, project config discovery and Claude Code imports are off, so the user's config,
//!   plugins, skills, sessions and login are never read or written. Free models need no login.
//! - Control (`Control::Hook`): opencode asks before edits, shell commands and other tools (`permission: ask`);
//!   the bridge hands each request to Agenium's hook endpoint, like Claude Code's hook: edits only inside the
//!   worktree, shell commands through the run's command gate, subagents only for Solo, Agenium's own MCP tools
//!   allowed; reads inside the worktree are allowed by the config, web tools and other folders denied.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use agenium_core::Effort;
use agenium_llm::Usage;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::mpsc;

use crate::driver::HOOK_URL_ENV;
use crate::driver::{inside, Check, CheckCtx, Control, Driver, Event, Setup, Turns, NO_SUBAGENTS};

pub struct Opencode;

const TOKEN_ENV: &str = "AGENIUM_MCP_TOKEN";
const PASSWORD_ENV: &str = "OPENCODE_SERVER_PASSWORD";
/// Read by the bridge only (not by opencode): the model of the session.
const MODEL_ENV: &str = "AGENIUM_OPENCODE_MODEL";

/// opencode model ids are `provider/model`; a bare id means OpenCode's own provider.
fn model_ref(model: &str) -> Value {
    let (provider, id) = model.split_once('/').unwrap_or(("opencode", model));
    json!({"providerID": provider, "id": id})
}

impl Driver for Opencode {
    fn control(&self) -> Control {
        Control::Hook
    }

    fn turns(&self) -> Turns {
        Turns::Stdin
    }

    fn served(&self) -> bool {
        true
    }

    fn program(&self, found: &Path) -> PathBuf {
        // npm installs a shim (`opencode`, `opencode.cmd`) next to `node_modules/@opencode/cli/bin/opencode(.exe)`.
        let dir = found.parent().unwrap_or(Path::new(""));
        let bin = dir
            .join("node_modules")
            .join("@opencode")
            .join("cli")
            .join("bin");
        for name in ["opencode.exe", "opencode"] {
            if bin.join(name).is_file() {
                return bin.join(name);
            }
        }
        found.to_path_buf()
    }

    fn args(
        &self,
        _model: &str,
        _effort: Option<&Effort>,
        _home: &Path,
        _resume: Option<&str>,
        _subagents: bool,
    ) -> Vec<String> {
        ["serve", "--hostname", "127.0.0.1", "--port", "0"]
            .map(str::to_owned)
            .to_vec()
    }

    fn prepare(&self, s: &Setup) -> std::io::Result<BTreeMap<String, String>> {
        let dirs = ["config", "data", "state", "cache"].map(|d| s.home.join(d));
        for d in &dirs {
            std::fs::create_dir_all(d)?;
        }
        std::fs::create_dir_all(dirs[0].join("opencode"))?;
        let mut permission = json!({
            "*": "ask",
            "read": "allow",
            "glob": "allow",
            "grep": "allow",
            "list": "allow",
            "todowrite": "allow",
            "todoread": "allow",
            "webfetch": "deny",
            "websearch": "deny",
            "external_directory": "deny",
        });
        permission[format!("{}_*", s.mcp_name)] = json!("allow");
        if !s.subagents {
            permission["task"] = json!("deny");
        }
        std::fs::write(
            dirs[0].join("opencode").join("opencode.json"),
            serde_json::to_vec_pretty(&json!({
                "autoupdate": false,
                "share": "disabled",
                "permission": permission,
                "mcp": {s.mcp_name: {
                    "type": "remote",
                    "url": s.mcp_url,
                    "enabled": true,
                    "headers": {"Authorization": format!("Bearer {{env:{TOKEN_ENV}}}")},
                }},
            }))?,
        )?;
        let token = s
            .mcp_authorization
            .strip_prefix("Bearer ")
            .unwrap_or(s.mcp_authorization);
        let password: String = ulid::Ulid::new().to_string();
        let mut env: BTreeMap<String, String> = [
            ("XDG_CONFIG_HOME", &dirs[0]),
            ("XDG_DATA_HOME", &dirs[1]),
            ("XDG_STATE_HOME", &dirs[2]),
            ("XDG_CACHE_HOME", &dirs[3]),
            ("OPENCODE_TEST_HOME", &s.home.to_path_buf()),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.display().to_string()))
        .collect();
        for (k, v) in [
            (TOKEN_ENV, token),
            (HOOK_URL_ENV, s.hook_url),
            (PASSWORD_ENV, password.as_str()),
            (MODEL_ENV, s.model),
            ("OPENCODE_DISABLE_AUTOUPDATE", "1"),
            ("OPENCODE_DISABLE_PROJECT_CONFIG", "1"),
            ("OPENCODE_CONFIG_PROJECT_DISABLE", "1"),
            ("OPENCODE_DISABLE_CLAUDE_CODE", "1"),
            ("OPENCODE_DISABLE_CLAUDE_CODE_SKILLS", "1"),
            ("OPENCODE_DISABLE_FILEWATCHER", "1"),
        ] {
            env.insert(k.to_owned(), v.to_owned());
        }
        Ok(env)
    }

    fn user_input(&self, text: &str) -> String {
        json!({"text": text}).to_string()
    }

    fn parse(&self, v: &Value) -> Vec<Event> {
        let d = &v["data"];
        let tool = |done: bool| Event::Tool {
            id: d["id"].as_str().unwrap_or("").to_owned(),
            name: d["name"].as_str().unwrap_or("tool").to_owned(),
            input: d["input"].clone(),
            done,
        };
        match v["type"].as_str() {
            Some("session.created") => d["sessionID"]
                .as_str()
                .map(|s| vec![Event::Session(s.to_owned())])
                .unwrap_or_default(),
            Some("session.tool.called") => vec![tool(false)],
            Some("session.tool.success" | "session.tool.error") => vec![tool(true)],
            Some("session.text.ended") => d["text"]
                .as_str()
                .filter(|t| !t.is_empty())
                .map(|t| vec![Event::Text(format!("{t}\n"))])
                .unwrap_or_default(),
            Some("session.step.ended") => {
                let t = &d["tokens"];
                let n = |x: &Value| x.as_u64().unwrap_or(0);
                vec![Event::Usage(Usage {
                    input: n(&t["input"]),
                    output: n(&t["output"]),
                    cache_read: n(&t["cache"]["read"]),
                    cache_write: n(&t["cache"]["write"]),
                    reasoning: n(&t["reasoning"]),
                })]
            }
            Some("session.execution.succeeded") => vec![Event::TurnEnd {
                usage: Usage::default(),
                response: String::new(),
                denied: vec![],
                error: None,
            }],
            Some(t) if t.starts_with("session.execution.") && t != "session.execution.started" => {
                let e = &d["error"];
                let msg = e["data"]["message"]
                    .as_str()
                    .or_else(|| e["message"].as_str())
                    .or_else(|| e.as_str())
                    .map(str::to_owned)
                    .unwrap_or_else(|| t.to_owned());
                vec![Event::TurnEnd {
                    usage: Usage::default(),
                    response: String::new(),
                    denied: vec![],
                    error: Some(format!("opencode turn failed: {msg}")),
                }]
            }
            _ => vec![],
        }
    }

    fn command(&self, tool: &str, input: &Value) -> Option<String> {
        matches!(tool, "shell" | "bash")
            .then(|| input["command"].as_str().map(str::to_owned))
            .flatten()
    }

    fn is_agenium_call(&self, tool: &str, _input: &Value, server: &str) -> bool {
        tool.starts_with(&format!("{server}_"))
    }

    /// A `permission.asked` request of opencode (`action`, `resources`).
    fn check(&self, payload: &Value, cx: &CheckCtx) -> Check {
        let action = payload["action"].as_str().unwrap_or("");
        let resources: Vec<&str> = payload["resources"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        match action {
            "edit" | "write" | "patch" | "apply_patch" => {
                match resources
                    .iter()
                    .find(|r| !inside(Path::new(r), cx.worktree, cx.worktree))
                {
                    None => Check::Allow,
                    Some(r) => Check::Deny(format!("{r} is outside the ticket worktree")),
                }
            }
            "shell" | "bash" => match resources.first() {
                Some(c) => Check::Command((*c).to_owned()),
                None => Check::Deny("empty shell command".into()),
            },
            "task" if cx.subagents => Check::Allow,
            "task" => Check::Deny(NO_SUBAGENTS.into()),
            a if a.starts_with(&format!("{}_", cx.mcp_name)) => Check::Allow,
            a => Check::Deny(format!("{a} is not available in an Agenium run")),
        }
    }
}

/// Runs a turn-by-turn conversation with an `opencode serve` process: reads the server address from its stdout,
/// creates the session in `worktree`, posts each input line (`{"text": ...}`) as a prompt and sends the server's
/// events to `out` (tool events carry the tool's name and input). Permission requests go to Agenium's hook
/// endpoint and its decision back to opencode. Ends when `input` closes.
pub(crate) fn bridge(
    stdout: impl tokio::io::AsyncRead + Unpin + Send + 'static,
    mut input: mpsc::UnboundedReceiver<String>,
    out: mpsc::UnboundedSender<Value>,
    env: &BTreeMap<String, String>,
    worktree: PathBuf,
    tail: Arc<Mutex<Vec<u8>>>,
) {
    let password = env.get(PASSWORD_ENV).cloned().unwrap_or_default();
    let hook = env.get(HOOK_URL_ENV).cloned().unwrap_or_default();
    let token = env.get(TOKEN_ENV).cloned().unwrap_or_default();
    let model = env.get(MODEL_ENV).cloned().unwrap_or_default();
    tokio::spawn(async move {
        let fail = |out: &mpsc::UnboundedSender<Value>, msg: String| {
            let _ = out.send(json!({"type": "session.execution.failed", "data": {"error": msg}}));
        };
        let mut lines = BufReader::new(stdout).lines();
        let base = tokio::time::timeout(std::time::Duration::from_secs(90), async {
            while let Ok(Some(l)) = lines.next_line().await {
                crate::run::keep_tail(&tail, format!("{l}\n").as_bytes());
                if let Some(u) = l.split("listening on ").nth(1) {
                    return Some(u.trim().trim_end_matches('/').to_owned());
                }
            }
            None
        })
        .await
        .ok()
        .flatten();
        let Some(base) = base else {
            return fail(&out, "opencode serve did not start".into());
        };
        // Keep draining the server's stdout (its tail explains failures).
        let t = tail.clone();
        tokio::spawn(async move {
            while let Ok(Some(l)) = lines.next_line().await {
                crate::run::keep_tail(&t, format!("{l}\n").as_bytes());
            }
        });
        let http = reqwest::Client::new();
        let api = |path: &str| format!("{base}/api{path}");
        let auth = |rb: reqwest::RequestBuilder| rb.basic_auth("opencode", Some(&password));
        let events = match auth(http.get(api("/event")))
            .header("accept", "text/event-stream")
            .send()
            .await
        {
            Ok(r) if r.status().is_success() => r,
            Ok(r) => return fail(&out, format!("opencode events: HTTP {}", r.status())),
            Err(e) => return fail(&out, format!("opencode events: {e}")),
        };
        let mut body = json!({"location": {"directory": worktree.display().to_string()}});
        if !model.is_empty() {
            body["model"] = model_ref(&model);
        }
        let session = match auth(http.post(api("/session"))).json(&body).send().await {
            Ok(r) if r.status().is_success() => r.json::<Value>().await.ok(),
            Ok(r) => {
                let s = r.status();
                let text = r.text().await.unwrap_or_default();
                return fail(&out, format!("opencode session: HTTP {s}: {text}"));
            }
            Err(e) => return fail(&out, format!("opencode session: {e}")),
        };
        let Some(sid) = session
            .as_ref()
            .and_then(|s| s["data"]["id"].as_str().or_else(|| s["id"].as_str()))
            .map(str::to_owned)
        else {
            return fail(&out, "opencode created no session".into());
        };
        // Events of this session, tool events completed with the tool's name and input.
        {
            let (out, http, sid) = (out.clone(), http.clone(), sid.clone());
            let (api_base, password) = (base.clone(), password.clone());
            tokio::spawn(async move {
                use futures::StreamExt;
                let mut stream = events.bytes_stream();
                let mut sse = agenium_llm::sse::SseDecoder::new();
                let mut names: BTreeMap<String, (String, Value)> = BTreeMap::new();
                while let Some(Ok(chunk)) = stream.next().await {
                    for e in sse.feed(&chunk) {
                        let Ok(mut v) = serde_json::from_str::<Value>(&e.data) else {
                            continue;
                        };
                        if v["data"]["sessionID"].as_str().is_some_and(|s| s != sid) {
                            continue;
                        }
                        let id = v["data"]["id"].as_str().unwrap_or("").to_owned();
                        match v["type"].as_str().unwrap_or("") {
                            "session.tool.input.started" => {
                                let name = v["data"]["name"].as_str().unwrap_or("tool").to_owned();
                                names.insert(id, (name, Value::Null));
                            }
                            "session.tool.called" => {
                                let e = names
                                    .entry(id)
                                    .or_insert_with(|| ("tool".into(), Value::Null));
                                e.1 = v["data"]["input"].clone();
                                v["data"]["name"] = json!(e.0);
                            }
                            "session.tool.success" | "session.tool.error" => {
                                if let Some((name, input)) = names.get(&id) {
                                    v["data"]["name"] = json!(name);
                                    v["data"]["input"] = input.clone();
                                }
                            }
                            "permission.asked" => {
                                let (http, hook, token) =
                                    (http.clone(), hook.clone(), token.clone());
                                let (base, password, data) =
                                    (api_base.clone(), password.clone(), v["data"].clone());
                                tokio::spawn(async move {
                                    answer(&http, &hook, &token, &base, &password, data).await
                                });
                            }
                            _ => {}
                        }
                        if out.send(v).is_err() {
                            return;
                        }
                    }
                }
            });
        }
        while let Some(line) = input.recv().await {
            let text = serde_json::from_str::<Value>(line.trim())
                .ok()
                .and_then(|v| v["text"].as_str().map(str::to_owned))
                .unwrap_or_else(|| line.trim().to_owned());
            match auth(http.post(api(&format!("/session/{sid}/prompt"))))
                .json(&json!({"text": text}))
                .send()
                .await
            {
                Ok(r) if r.status().is_success() => {}
                Ok(r) => fail(&out, format!("opencode prompt: HTTP {}", r.status())),
                Err(e) => fail(&out, format!("opencode prompt: {e}")),
            }
        }
    });
}

/// Asks Agenium's hook endpoint about one permission request and replies to opencode (`once` or `reject` with the
/// reason). A hook that cannot be reached rejects (fail-closed).
async fn answer(
    http: &reqwest::Client,
    hook: &str,
    token: &str,
    base: &str,
    password: &str,
    req: Value,
) {
    let decision = match http.post(hook).bearer_auth(token).json(&req).send().await {
        Ok(r) => r.json::<Value>().await.unwrap_or(Value::Null),
        Err(e) => json!({"decision": "deny", "reason": format!("Agenium could not be asked: {e}")}),
    };
    let allow = decision["decision"] == "allow";
    let mut reply = json!({"decision": if allow { "once" } else { "reject" }});
    if !allow {
        reply["message"] = json!(decision["reason"].as_str().unwrap_or("refused by Agenium"));
    }
    let (sid, id) = (
        req["sessionID"].as_str().unwrap_or(""),
        req["id"].as_str().unwrap_or(""),
    );
    let _ = http
        .post(format!("{base}/api/session/{sid}/permission/{id}/reply"))
        .basic_auth("opencode", Some(password))
        .json(&reply)
        .send()
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(s: &str) -> Vec<Event> {
        Opencode.parse(&serde_json::from_str(s).unwrap())
    }

    /// Server events captured from opencode 2.1.2 (`opencode/deepseek-v4.1-flash-free`), tool events as the bridge
    /// completes them.
    #[test]
    fn parses_the_event_stream() {
        assert_eq!(
            ev(r#"{"type":"session.created","data":{"sessionID":"ses_1"}}"#),
            [Event::Session("ses_1".into())]
        );
        let called = ev(
            r#"{"type":"session.tool.called","data":{"sessionID":"ses_1","id":"call_1","name":"shell","input":{"command":"Get-Content a.txt"}}}"#,
        );
        match &called[0] {
            Event::Tool {
                name, input, done, ..
            } => {
                assert!(!done);
                assert_eq!(
                    Opencode.command(name, input).as_deref(),
                    Some("Get-Content a.txt")
                );
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            &ev(
                r#"{"type":"session.tool.success","data":{"id":"call_1","name":"shell","input":{}}}"#
            )[0],
            Event::Tool { done: true, .. }
        ));
        assert_eq!(
            ev(
                r#"{"type":"session.step.ended","data":{"finish":"tool-calls","tokens":{"input":3295,"output":54,"reasoning":2,"cache":{"read":1792,"write":0}}}}"#
            ),
            [Event::Usage(Usage {
                input: 3295,
                output: 54,
                cache_read: 1792,
                cache_write: 0,
                reasoning: 2
            })]
        );
        assert_eq!(
            ev(r#"{"type":"session.text.ended","data":{"text":"done"}}"#),
            [Event::Text("done\n".into())]
        );
        assert!(matches!(
            &ev(r#"{"type":"session.execution.succeeded","data":{"sessionID":"ses_1"}}"#)[0],
            Event::TurnEnd { error: None, .. }
        ));
        assert!(matches!(
            &ev(r#"{"type":"session.execution.failed","data":{"error":"FreeTierError"}}"#)[0],
            Event::TurnEnd { error: Some(e), .. } if e.contains("FreeTierError")
        ));
        assert!(ev(r#"{"type":"session.execution.started","data":{}}"#).is_empty());
        assert!(Opencode.is_agenium_call("agenium_submit_report", &Value::Null, "agenium"));
    }

    #[test]
    fn permission_requests_follow_agenium_rules() {
        let wt = std::env::temp_dir().join("agenium-oc-wt");
        let cx = |subagents| CheckCtx {
            worktree: &wt,
            home: Path::new("h"),
            mcp_name: "agenium",
            subagents,
        };
        let ask = |action: &str, res: &[&str]| json!({"id": "per_1", "sessionID": "ses_1", "action": action, "resources": res});
        assert_eq!(
            Opencode.check(&ask("edit", &["src/a.rs"]), &cx(false)),
            Check::Allow
        );
        assert!(matches!(
            Opencode.check(&ask("edit", &["../x.rs"]), &cx(false)),
            Check::Deny(_)
        ));
        assert_eq!(
            Opencode.check(&ask("shell", &["cargo test"]), &cx(false)),
            Check::Command("cargo test".into())
        );
        assert_eq!(
            Opencode.check(&ask("task", &[]), &cx(false)),
            Check::Deny(NO_SUBAGENTS.into())
        );
        assert_eq!(Opencode.check(&ask("task", &[]), &cx(true)), Check::Allow);
        assert_eq!(
            Opencode.check(&ask("agenium_submit_report", &[]), &cx(false)),
            Check::Allow
        );
        assert!(matches!(
            Opencode.check(&ask("codesearch", &[]), &cx(false)),
            Check::Deny(_)
        ));
    }

    #[test]
    fn the_home_isolates_opencode() {
        let home = std::env::temp_dir().join(format!("agenium-oc-{}", std::process::id()));
        std::fs::create_dir_all(&home).unwrap();
        let env = Opencode
            .prepare(&Setup {
                home: &home,
                worktree: &home,
                mcp_name: "agenium",
                mcp_url: "http://127.0.0.1:1/mcp",
                mcp_authorization: "Bearer t0k",
                hook_url: "http://127.0.0.1:1/hook",
                subagents: false,
                model: "deepseek-v4.1-flash-free",
            })
            .unwrap();
        for k in [
            "XDG_CONFIG_HOME",
            "XDG_DATA_HOME",
            "XDG_STATE_HOME",
            "XDG_CACHE_HOME",
            "OPENCODE_TEST_HOME",
        ] {
            assert!(env[k].starts_with(&home.display().to_string()), "{k}");
        }
        assert_eq!(env["OPENCODE_DISABLE_CLAUDE_CODE"], "1");
        assert_eq!(env["OPENCODE_DISABLE_PROJECT_CONFIG"], "1");
        assert_eq!(env[TOKEN_ENV], "t0k");
        assert!(!env[PASSWORD_ENV].is_empty());
        let cfg: Value = serde_json::from_slice(
            &std::fs::read(home.join("config").join("opencode").join("opencode.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(cfg["permission"]["*"], "ask");
        assert_eq!(cfg["permission"]["task"], "deny");
        assert_eq!(cfg["permission"]["agenium_*"], "allow");
        assert_eq!(
            cfg["mcp"]["agenium"]["headers"]["Authorization"],
            "Bearer {env:AGENIUM_MCP_TOKEN}"
        );
        assert_eq!(
            model_ref("deepseek-v4.1-flash-free"),
            json!({"providerID": "opencode", "id": "deepseek-v4.1-flash-free"})
        );
        std::fs::remove_dir_all(home).unwrap();
    }
}
