//! opencode CLI driver, checked with opencode 2.1.2 on 2026-10-25 (ADR-68). It runs OpenCode's free models
//! (`opencode/*-free`), which answer only to OpenCode itself and need no login, and the models of a provider the user
//! signed in to in their own opencode (`opencode-go/deepseek-v4.1-flash` with an OpenCode Go subscription): that
//! provider's key is read from the user's `opencode.db` (read-only) and handed to the run through the environment,
//! never written to the per-run home.
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

use agenium_core::Effort;
use agenium_llm::Usage;
use serde_json::{json, Value};

use crate::driver::HOOK_URL_ENV;
use crate::driver::{
    own_tool_off, Check, CheckCtx, Control, Driver, Event, Setup, Turns, NO_SUBAGENTS,
};

pub struct Opencode;

const TOKEN_ENV: &str = "AGENIUM_MCP_TOKEN";
const PASSWORD_ENV: &str = "OPENCODE_SERVER_PASSWORD";
/// Read by the bridge only (not by opencode): the model of the session.
const MODEL_ENV: &str = "AGENIUM_OPENCODE_MODEL";

/// The key of the model's provider, from the user's own opencode (empty when the user has none).
const KEY_ENV: &str = "AGENIUM_OPENCODE_KEY";

mod account;
mod bridge;
pub use account::account_models;
use account::user_login;
pub(crate) use account::user_signed_in;
pub(crate) use bridge::bridge;

/// The model catalogue opencode is given (models.dev format), named by the open root.
static MODELS: std::sync::RwLock<Option<PathBuf>> = std::sync::RwLock::new(None);

/// Names the catalogue each run's opencode reads its model list from (the root's copy of models.dev). Without it a run
/// starts with an empty cache and fetches the list itself: a prompt sent before that fetch landed found only the list
/// built into opencode, and a model newer than it (`opencode-go/iceman-contributor-free`) answered "Model unavailable"
/// to about half the runs (2026-11-01).
pub fn use_models_catalogue(path: PathBuf) {
    *MODELS.write().unwrap_or_else(|p| p.into_inner()) = Some(path);
}

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

    /// opencode waits for a permission answer as long as it takes (the bridge asks Agenium): an approval may take
    /// the whole run.
    fn hook_timeout(&self) -> u64 {
        3600
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
        // ADR-129: its own shell, file, search and to-do tools are off; the work goes through Agenium's MCP tools.
        // The provider's key from the user's own opencode: handed over in the environment, the configuration only
        // names the variable. A free model (`opencode/*-free`) uses the user's OpenCode login too when there is one:
        // without it the run was a guest, and OpenCode's guest allowance ran out ("Free usage limit reached.
        // Connect a provider…", 2026-10-31) while the user was signed in.
        let provider = s.model.split_once('/').map_or("opencode", |(p, _)| p);
        let login = user_login(provider);
        let mut permission = json!({
            "*": "ask",
            "read": "deny",
            "glob": "deny",
            "grep": "deny",
            "list": "deny",
            "edit": "deny",
            "bash": "deny",
            "todowrite": "deny",
            "todoread": "deny",
            "webfetch": "deny",
            "websearch": "deny",
            "external_directory": "deny",
        });
        permission[format!("{}_*", s.mcp_name)] = json!("allow");
        if !s.subagents {
            permission["task"] = json!("deny");
        }
        let mut config = json!({
            "autoupdate": false,
            "share": "disabled",
            "permission": permission,
            // opencode 2's code mode offers its own management tools inside `execute` (`opencode.worktree_create`, its
            // sessions, its models), outside `permission` and `policies` (probe of 2026-10-28): a worktree it would
            // make lands in the run's scratch home, which goes with the run, not beside the project.
            "worktree": {"directory": s.home.join("worktrees").to_string_lossy()},
            "mcp": {s.mcp_name: {
                "type": "remote",
                "url": s.mcp_url,
                "enabled": true,
                "headers": {"Authorization": format!("Bearer {{env:{TOKEN_ENV}}}")},
            }},
        });
        if login.is_some() {
            config["provider"] =
                json!({provider: {"options": {"apiKey": format!("{{env:{KEY_ENV}}}")}}});
        }
        std::fs::write(
            dirs[0].join("opencode").join("opencode.json"),
            serde_json::to_vec_pretty(&config)?,
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
            (KEY_ENV, login.as_deref().unwrap_or("")),
            ("OPENCODE_DISABLE_AUTOUPDATE", "1"),
            ("OPENCODE_DISABLE_PROJECT_CONFIG", "1"),
            ("OPENCODE_CONFIG_PROJECT_DISABLE", "1"),
            ("OPENCODE_DISABLE_CLAUDE_CODE", "1"),
            ("OPENCODE_DISABLE_CLAUDE_CODE_SKILLS", "1"),
            ("OPENCODE_DISABLE_FILEWATCHER", "1"),
        ] {
            env.insert(k.to_owned(), v.to_owned());
        }
        // The model list, copied into the run's own home (readable wherever opencode runs, never changed under it).
        let catalogue = MODELS.read().unwrap_or_else(|p| p.into_inner()).clone();
        if let Some(src) = catalogue.filter(|p| p.is_file()) {
            let copy = dirs[3].join("models.json");
            if std::fs::copy(&src, &copy).is_ok() {
                env.insert(
                    "OPENCODE_MODELS_PATH".to_owned(),
                    copy.display().to_string(),
                );
            }
        }
        Ok(env)
    }

    fn resumes_across_runs(&self) -> bool {
        true
    }

    /// Its caches (the model catalog, packages) are fetched again; its sessions are in `data` and `state`.
    fn not_kept(&self, name: &str) -> bool {
        name == "cache"
    }

    /// opencode 2's code mode: `execute` runs a script whose tool calls go through the permission check one by one
    /// (`tools.agenium.read_file(...)` reaches Agenium over MCP).
    fn runs_other_tools(&self, tool: &str) -> bool {
        tool == "execute"
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
            Some(kind @ ("session.tool.success" | "session.tool.error")) => {
                let error = kind == "session.tool.error";
                let mut text: Vec<String> = d["content"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|c| c["text"].as_str().map(str::to_owned))
                    .collect();
                for k in ["error", "message"] {
                    if let Some(t) = d[k].as_str().or_else(|| d[k]["message"].as_str()) {
                        text.push(t.to_owned());
                    }
                }
                let text = text.join("\n");
                let out = (error || !text.is_empty()).then(|| Event::ToolOutput {
                    id: d["id"].as_str().unwrap_or("").to_owned(),
                    text,
                    error,
                });
                out.into_iter().chain([tool(true)]).collect()
            }
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
                    cache_write_1h: 0,
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
        match action {
            // ADR-129: its own file and shell tools are off; the work goes through Agenium's tools.
            "edit" | "write" | "patch" | "apply_patch" | "shell" | "bash" | "read" | "glob"
            | "grep" | "list" => Check::Deny(own_tool_off(action)),
            "task" if cx.subagents => Check::Allow,
            "task" => Check::Deny(NO_SUBAGENTS.into()),
            a if a.starts_with(&format!("{}_", cx.mcp_name)) => Check::Allow,
            a => Check::Deny(format!("{a} is not available in an Agenium run")),
        }
    }
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
                r#"{"type":"session.tool.success","data":{"id":"call_1","name":"shell","input":{},"content":[{"text":"ok"}]}}"#
            )[1],
            Event::Tool { done: true, .. }
        ));
        // What the call returned (captured from opencode 2.2.5): its content's text, before the end of the step.
        assert_eq!(
            ev(
                r#"{"type":"session.tool.success","data":{"id":"call_1","name":"shell","input":{},"content":[{"text":"ok"}]}}"#
            )[0],
            Event::ToolOutput {
                id: "call_1".into(),
                text: "ok".into(),
                error: false
            }
        );
        assert!(matches!(
            &ev(r#"{"type":"session.tool.error","data":{"id":"c2","name":"x","error":"boom"}}"#)[0],
            Event::ToolOutput { error: true, text, .. } if text == "boom"
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
                cache_write_1h: 0,
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
            read_only: false,
        };
        let ask = |action: &str, res: &[&str]| json!({"id": "per_1", "sessionID": "ses_1", "action": action, "resources": res});
        // ADR-129: its own file and shell tools are off, whatever they name.
        for action in ["edit", "write", "bash", "shell", "read", "grep"] {
            assert_eq!(
                Opencode.check(&ask(action, &["src/a.rs"]), &cx(false)),
                Check::Deny(own_tool_off(action))
            );
        }
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

    /// A paid model's provider key comes from the user's own opencode login (the active one), read-only; none when
    /// the user has no login for that provider.
    #[test]
    fn the_home_isolates_opencode() {
        let home = std::env::temp_dir().join(format!("agenium-oc-{}", std::process::id()));
        std::fs::create_dir_all(&home).unwrap();
        // The root's model catalogue reaches the run as its own copy.
        let catalogue = home.join("models-dev.json");
        std::fs::write(
            &catalogue,
            r#"{"opencode-go":{"models":{"iceman-contributor-free":{}}}}"#,
        )
        .unwrap();
        use_models_catalogue(catalogue);
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
                run_for: std::time::Duration::from_secs(2700),
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
        let models = std::path::Path::new(&env["OPENCODE_MODELS_PATH"]);
        assert!(
            models.starts_with(&home) && models.ends_with("cache/models.json"),
            "{}",
            models.display()
        );
        assert!(std::fs::read_to_string(models)
            .unwrap()
            .contains("iceman-contributor-free"));
        assert_eq!(env["OPENCODE_DISABLE_PROJECT_CONFIG"], "1");
        assert_eq!(env[TOKEN_ENV], "t0k");
        assert!(!env[PASSWORD_ENV].is_empty());
        let cfg: Value = serde_json::from_slice(
            &std::fs::read(home.join("config").join("opencode").join("opencode.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(cfg["permission"]["*"], "ask");
        assert_eq!(cfg["permission"]["bash"], "deny");
        assert_eq!(cfg["permission"]["read"], "deny");
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
