//! Devin CLI (`devin`) driver over ACP (Agent Client Protocol), checked with devin 3000.11.3 on 2026-11-04 (ADR-156).
//!
//! - Headless: `devin --config <home>/config.json acp --model <m>` speaks JSON-RPC on stdio. Each turn is one process
//!   (`Turns::ProcessPerTurn`): `initialize`, then `session/new` (the first turn) or `session/load` (later turns, the
//!   session id kept by the run), `session/set_mode bypass`, then `session/prompt`. `-p` prints only the last text.
//! - Control: the whole process runs inside Agenium's sandbox (`Control::Sandbox`): its own exec runs contained, as
//!   Codex's does (Devin's hooks do not run under ACP, and it asks before a command only when it judges one risky).
//!   Devin hands file reads and writes to the client (`fs/read_text_file`, `fs/write_text_file`): the bridge asks
//!   Agenium (the hook endpoint, `check`) and does them itself, so a read-only run writes nothing and nothing outside
//!   the worktree is touched.
//! - Isolation: Devin keeps its login, configuration and sessions under `%APPDATA%\devin` (the Known Folder, not
//!   `APPDATA`) and loads the user's MCP servers from there and from other tools' configurations. In the sandbox the
//!   other tools' files are not visible, `%APPDATA%\devin` is read-only but for its `cli` folder (logs, sessions),
//!   `--config` replaces the user's `config.json` (hooks, permissions), and the worktree's
//!   `.devin/mcp_config.local.json` turns each of the user's own servers off by name; Agenium's MCP server comes in
//!   `session/new`.

mod bridge;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use agenium_core::Effort;
use agenium_llm::Usage;
use serde_json::{json, Value};

use crate::driver::{inside, normalize, Check, CheckCtx, Control, Driver, Event, Setup, Turns};
use crate::driver::{HOOK_TOKEN_ENV, HOOK_URL_ENV};

pub(crate) use bridge::bridge;

/// Environment the bridge reads its run's settings from (never passed to Devin).
pub(crate) const MCP_URL_ENV: &str = "AGENIUM_DEVIN_MCP_URL";
pub(crate) const MCP_NAME_ENV: &str = "AGENIUM_DEVIN_MCP_NAME";

/// The file in the worktree that turns the user's own Devin MCP servers off for the run.
pub const LOCAL_MCP: &str = ".devin/mcp_config.local.json";

pub struct Devin;

/// Devin's own folder (`%APPDATA%\devin` on Windows, `~/.config/devin` elsewhere).
fn devin_dir() -> Option<PathBuf> {
    if cfg!(windows) {
        std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("devin"))
    } else {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config").join("devin"))
    }
}

/// The names of the user's own Devin MCP servers (names only: their settings hold tokens).
fn user_servers() -> Vec<String> {
    devin_dir()
        .and_then(|d| std::fs::read_to_string(d.join("mcp_config.json")).ok())
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|v| {
            v["mcpServers"]
                .as_object()
                .map(|o| o.keys().cloned().collect())
        })
        .unwrap_or_default()
}

/// Devin's plugin list: a plugin is a `requirements` entry (`{"spec": {"source": "github", "repo": …}}`).
fn plugin_lock() -> Option<PathBuf> {
    devin_dir().map(|d| d.join("cli").join("plugins").join("lock.json"))
}

/// What the run kept of the plugin list when it started.
const PLUGINS_BEFORE: &str = "devin-plugins-before.json";

/// `now` without the plugins added since `before` other than those Devin's account manages (`scope: managed`): a
/// run writes in Devin's `cli` folder, and a plugin it put in the list would load in the user's own Devin later.
/// `None` when nothing was added.
fn without_added(before: &Value, now: &Value) -> Option<Value> {
    let had: Vec<&Value> = before["requirements"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|r| &r["spec"])
        .collect();
    let reqs = now["requirements"].as_array()?;
    let kept: Vec<Value> = reqs
        .iter()
        .filter(|r| had.contains(&&r["spec"]) || r["origin"]["scope"] == "managed")
        .cloned()
        .collect();
    (kept.len() < reqs.len()).then(|| {
        let mut v = now.clone();
        v["requirements"] = Value::Array(kept);
        v
    })
}

/// `.devin/mcp_config.local.json` with each of `servers` off, what the file already held kept.
fn local_mcp(existing: Option<&str>, servers: &[String]) -> Value {
    let mut v = existing
        .and_then(|s| serde_json::from_str::<Value>(s).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}));
    if !v["mcpServers"].is_object() {
        v["mcpServers"] = json!({});
    }
    for s in servers {
        v["mcpServers"][s] = json!({"disabled": true});
    }
    v
}

impl Driver for Devin {
    fn control(&self) -> Control {
        Control::Sandbox
    }

    fn turns(&self) -> Turns {
        Turns::ProcessPerTurn
    }

    /// The effort is part of Devin's model name (`swe-2-max`); the session id goes to the bridge, which loads it.
    fn args(
        &self,
        model: &str,
        _effort: Option<&Effort>,
        home: &Path,
        _resume: Option<&str>,
        _subagents: bool,
    ) -> Vec<String> {
        let mut a = vec![
            "--config".to_owned(),
            home.join("config.json").display().to_string(),
            "--respect-workspace-trust".to_owned(),
            "false".to_owned(),
            "acp".to_owned(),
        ];
        if !model.is_empty() {
            a.extend(["--model".to_owned(), model.to_owned()]);
        }
        a
    }

    fn prepare(&self, s: &Setup) -> std::io::Result<BTreeMap<String, String>> {
        // No hooks, no permissions of the user's: Devin's own config is replaced for the run.
        let mut config = json!({"version": 1});
        if !s.model.is_empty() {
            config["agent"] = json!({"model": s.model});
        }
        std::fs::write(
            s.home.join("config.json"),
            serde_json::to_vec_pretty(&config)?,
        )?;
        let local = s.worktree.join(LOCAL_MCP);
        let existing = std::fs::read_to_string(&local).ok();
        if let Some(dir) = local.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(
            &local,
            serde_json::to_vec_pretty(&local_mcp(existing.as_deref(), &user_servers()))?,
        )?;
        // The plugin list as it is now, to take back what the run adds to it (`finish`).
        let before = plugin_lock()
            .and_then(|f| std::fs::read(f).ok())
            .unwrap_or_else(|| b"{}".to_vec());
        std::fs::write(s.home.join(PLUGINS_BEFORE), before)?;
        let token = s
            .mcp_authorization
            .strip_prefix("Bearer ")
            .unwrap_or(s.mcp_authorization);
        // Devin finds its config directory from these (without them it does not start); the sandbox still shows it
        // only its own folder of the profile.
        let known: Vec<(String, String)> = ["APPDATA", "LOCALAPPDATA", "USERPROFILE"]
            .into_iter()
            .filter_map(|k| std::env::var(k).ok().map(|v| (k.to_owned(), v)))
            .collect();
        Ok([
            (HOOK_URL_ENV, s.hook_url),
            (HOOK_TOKEN_ENV, token),
            (MCP_URL_ENV, s.mcp_url),
            (MCP_NAME_ENV, s.mcp_name),
            // Its own model setting would otherwise win over `--config`.
            ("DEVIN_MODEL", s.model),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .chain(known)
        .collect())
    }

    fn acp(&self) -> bool {
        true
    }

    /// Takes back a plugin the run added to Devin's own list (the entry only, the rest of the file as it is).
    fn finish(&self, s: &Setup) -> std::io::Result<()> {
        let (Some(lock), Ok(before)) = (plugin_lock(), std::fs::read(s.home.join(PLUGINS_BEFORE)))
        else {
            return Ok(());
        };
        let (Ok(before), Some(now)) = (
            serde_json::from_slice::<Value>(&before),
            std::fs::read(&lock)
                .ok()
                .and_then(|b| serde_json::from_slice::<Value>(&b).ok()),
        ) else {
            return Ok(());
        };
        if let Some(v) = without_added(&before, &now) {
            tracing::warn!(file = %lock.display(), "a Devin run added a plugin to Devin's own list: taken back");
            std::fs::write(&lock, serde_json::to_vec_pretty(&v)?)?;
        }
        Ok(())
    }

    /// Devin's binary, read-only, and its own folder: read-only, its login and configuration among it.
    fn read_roots(&self, program: &Path) -> Vec<PathBuf> {
        program
            .parent()
            .map(Path::to_path_buf)
            .into_iter()
            .chain(devin_dir())
            .collect()
    }

    /// Where Devin writes its logs and sessions (it does not start without them).
    fn writable_roots(&self) -> Vec<PathBuf> {
        devin_dir().map(|d| d.join("cli")).into_iter().collect()
    }

    fn resumes_across_runs(&self) -> bool {
        true
    }

    fn user_input(&self, text: &str) -> String {
        format!("{}\n", json!({"text": text}))
    }

    /// The bridge's messages: `{"type":"session"}`, `{"type":"update"}` (an ACP `session/update`), `{"type":"turn"}`
    /// (the prompt's answer) and `{"type":"failed"}`.
    fn parse(&self, v: &Value) -> Vec<Event> {
        match v["type"].as_str() {
            Some("session") => v["id"]
                .as_str()
                .map(|s| vec![Event::Session(s.to_owned())])
                .unwrap_or_default(),
            Some("update") => update(&v["update"]),
            Some("turn") => {
                let u = &v["result"]["usage"];
                let n = |k: &str| u[k].as_u64().unwrap_or(0);
                let cached = n("cachedReadTokens");
                vec![Event::TurnEnd {
                    usage: Usage {
                        input: n("inputTokens").saturating_sub(cached),
                        output: n("outputTokens"),
                        cache_read: cached,
                        ..Default::default()
                    },
                    response: String::new(),
                    denied: vec![],
                    error: match v["result"]["stopReason"].as_str() {
                        Some("end_turn") | None => None,
                        Some(other) => Some(format!("devin stopped: {other}")),
                    },
                }]
            }
            Some("failed") => vec![Event::TurnEnd {
                usage: Usage::default(),
                response: String::new(),
                denied: vec![],
                error: Some(v["error"].as_str().unwrap_or("devin failed").to_owned()),
            }],
            _ => vec![],
        }
    }

    fn command(&self, tool: &str, input: &Value) -> Option<String> {
        (tool == "exec")
            .then(|| input["command"].as_str().map(str::to_owned))
            .flatten()
    }

    fn is_agenium_call(&self, tool: &str, _input: &Value, server: &str) -> bool {
        tool.starts_with(&format!("mcp__{server}__"))
    }

    /// The bridge's questions: a file Devin hands to the client to read or write, and a permission Devin asks for.
    fn check(&self, payload: &Value, cx: &CheckCtx) -> Check {
        let (name, input) = crate::driver::hook_call(payload);
        let path = input["path"]
            .as_str()
            .map(|p| normalize(Path::new(p), cx.worktree));
        match name.as_str() {
            "read" => match path {
                Some(p)
                    if inside(&p, cx.worktree, cx.worktree) || inside(&p, cx.worktree, cx.home) =>
                {
                    Check::Allow
                }
                Some(p) => Check::Deny(format!("{} is outside the ticket worktree", p.display())),
                None => Check::Deny("no path".into()),
            },
            "write" => match path {
                _ if cx.read_only => Check::Deny("this run only reads: no file is written".into()),
                Some(p) if p.ends_with(LOCAL_MCP) => {
                    Check::Deny("that file is Agenium's for this run".into())
                }
                Some(p)
                    if inside(&p, cx.worktree, cx.worktree) || inside(&p, cx.worktree, cx.home) =>
                {
                    Check::Allow
                }
                Some(p) => Check::Deny(format!("{} is outside the ticket worktree", p.display())),
                None => Check::Deny("no path".into()),
            },
            n if self.is_agenium_call(n, &input, cx.mcp_name) => Check::Allow,
            n if n.starts_with("mcp__") => {
                Check::Deny(format!("{n} is not available in an Agenium run"))
            }
            // Its own command or edit Devin judged risky: it runs in the sandbox, which bounds it.
            _ if cx.read_only => Check::Deny("this run only reads".into()),
            _ => Check::Allow,
        }
    }
}

/// The models Devin offers (`devin models list --format json`): its id, label and window; the effort is part of the
/// name (`swe-2-max`), so none is set apart.
pub async fn models(program: &Path) -> Result<Vec<agenium_llm::ModelInfo>, String> {
    let mut cmd = tokio::process::Command::new(program);
    cmd.args(["models", "list", "--format", "json"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    let out = tokio::time::timeout(std::time::Duration::from_secs(40), cmd.output())
        .await
        .map_err(|_| "devin models list did not answer in 40 s".to_owned())?
        .map_err(|e| format!("cannot start devin: {e}"))?;
    parse_models(&String::from_utf8_lossy(&out.stdout))
}

fn parse_models(text: &str) -> Result<Vec<agenium_llm::ModelInfo>, String> {
    let v: Value = serde_json::from_str(text.trim_start_matches('\u{feff}').trim())
        .map_err(|e| format!("devin models list: {e}"))?;
    Ok(v["families"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|f| f["variants"].as_array().cloned().unwrap_or_default())
        .filter_map(|m| {
            Some(agenium_llm::ModelInfo {
                id: m["model_uid"].as_str()?.to_owned(),
                display_name: m["label"].as_str().map(str::to_owned),
                context_window: m["max_context_tokens"].as_u64(),
                max_output: m["max_output_tokens"].as_u64(),
                efforts: Some(vec![]),
                ..Default::default()
            })
        })
        .collect())
}

/// One ACP `session/update` as the run's events (the bridge has put a tool's name and input on its updates).
fn update(u: &Value) -> Vec<Event> {
    match u["sessionUpdate"].as_str() {
        Some("agent_message_chunk") => u["content"]["text"]
            .as_str()
            .filter(|t| !t.is_empty())
            .map(|t| vec![Event::Text(t.to_owned())])
            .unwrap_or_default(),
        Some(kind @ ("tool_call" | "tool_call_update")) => {
            let done = matches!(u["status"].as_str(), Some("completed" | "failed"));
            if kind == "tool_call_update" && !done {
                return vec![];
            }
            vec![Event::Tool {
                id: u["toolCallId"].as_str().unwrap_or("").to_owned(),
                name: tool_name(u),
                input: u["rawInput"].clone(),
                done,
            }]
        }
        Some("usage_update") => u["used"]
            .as_u64()
            .or_else(|| u["usage"]["inputTokens"].as_u64())
            .map(|n| vec![Event::Context(n)])
            .unwrap_or_default(),
        _ => vec![],
    }
}

/// A tool call's own name: Devin's (`write`, `exec`, `mcp__agenium__submit_report`), else its kind.
fn tool_name(u: &Value) -> String {
    let m = &u["_meta"];
    m["cognition.ai/inferenceToolName"]
        .as_str()
        .or_else(|| m["cognition.ai/toolName"].as_str())
        .or_else(|| u["kind"].as_str())
        .unwrap_or("tool")
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Updates captured from devin 3000.11.3 (`swe-2-max`, 2026-11-04), shortened.
    #[test]
    fn reads_the_acp_updates() {
        let ev = |s: &str| Devin.parse(&serde_json::from_str(s).unwrap());
        assert_eq!(
            ev(r#"{"type":"session","id":"flax-poppyseed"}"#),
            [Event::Session("flax-poppyseed".into())]
        );
        assert_eq!(
            ev(
                r#"{"type":"update","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"DONE"}}}"#
            ),
            [Event::Text("DONE".into())]
        );
        let call = ev(
            r#"{"type":"update","update":{"sessionUpdate":"tool_call","toolCallId":"exec:0#1","title":"Ran echo","kind":"execute","rawInput":{"command":"echo hi > cmd.txt","workdir":"w"},"_meta":{"cognition.ai/inferenceToolName":"exec"}}}"#,
        );
        match &call[0] {
            Event::Tool {
                name, input, done, ..
            } => {
                assert_eq!(name, "exec");
                assert!(!done);
                assert_eq!(
                    Devin.command(name, input).as_deref(),
                    Some("echo hi > cmd.txt")
                );
            }
            e => panic!("{e:?}"),
        }
        assert!(ev(r#"{"type":"update","update":{"sessionUpdate":"tool_call_update","toolCallId":"exec:0#1","status":"in_progress"}}"#).is_empty());
        assert!(matches!(
            &ev(
                r#"{"type":"update","update":{"sessionUpdate":"tool_call_update","toolCallId":"exec:0#1","status":"completed","_meta":{"cognition.ai/inferenceToolName":"exec"}}}"#
            )[0],
            Event::Tool { done: true, .. }
        ));
        match &ev(
            r#"{"type":"turn","result":{"stopReason":"end_turn","usage":{"inputTokens":18000,"outputTokens":30,"cachedReadTokens":17900}}}"#,
        )[0]
        {
            Event::TurnEnd { usage, error, .. } => {
                assert_eq!(
                    (usage.input, usage.cache_read, usage.output),
                    (100, 17_900, 30)
                );
                assert!(error.is_none());
            }
            e => panic!("{e:?}"),
        }
        assert!(Devin.is_agenium_call("mcp__agenium__submit_report", &Value::Null, "agenium"));
        assert!(!Devin.is_agenium_call("mcp__github__search", &Value::Null, "agenium"));
    }

    #[test]
    fn files_are_read_and_written_in_the_worktree_only() {
        let w = std::env::temp_dir().join("agenium-devin-check").join("wt");
        let h = std::env::temp_dir()
            .join("agenium-devin-check")
            .join("home");
        let cx = |read_only| CheckCtx {
            worktree: &w,
            home: &h,
            mcp_name: "agenium",
            subagents: false,
            read_only,
        };
        let ask =
            |tool: &str, path: &Path| json!({"tool_name": tool, "tool_input": {"path": path}});
        let inside_file = w.join("src").join("a.rs");
        let outside = std::env::temp_dir().join("elsewhere.txt");
        assert_eq!(
            Devin.check(&ask("read", &inside_file), &cx(true)),
            Check::Allow
        );
        assert_eq!(
            Devin.check(&ask("write", &inside_file), &cx(false)),
            Check::Allow
        );
        assert!(matches!(
            Devin.check(&ask("write", &inside_file), &cx(true)),
            Check::Deny(_)
        ));
        assert!(matches!(
            Devin.check(&ask("write", &outside), &cx(false)),
            Check::Deny(_)
        ));
        assert!(matches!(
            Devin.check(&ask("read", &outside), &cx(false)),
            Check::Deny(_)
        ));
        assert!(matches!(
            Devin.check(&ask("write", &w.join(LOCAL_MCP)), &cx(false)),
            Check::Deny(_)
        ));
        let mcp = |n: &str| json!({"tool_name": n, "tool_input": {}});
        assert_eq!(
            Devin.check(&mcp("mcp__agenium__submit_report"), &cx(false)),
            Check::Allow
        );
        assert!(matches!(
            Devin.check(&mcp("mcp__github__search"), &cx(false)),
            Check::Deny(_)
        ));
    }

    #[test]
    fn reads_the_model_list() {
        let m = parse_models(
            r#"{"families":[{"family_uid":"swe-2","variants":[{"model_uid":"swe-2-max","label":"SWE-2 Max","max_context_tokens":262000,"max_output_tokens":128000,"cost_tier":"Free"}]},{"family_uid":"adaptive","variants":[{"model_uid":"adaptive","label":"Adaptive"}]}]}"#,
        )
        .unwrap();
        assert_eq!(m.len(), 2);
        assert_eq!(
            (m[0].id.as_str(), m[0].context_window),
            ("swe-2-max", Some(262_000))
        );
        assert_eq!(m[0].efforts, Some(vec![]));
    }

    #[test]
    fn a_plugin_the_run_adds_is_taken_back() {
        let req = |repo: &str, scope: &str| json!({"spec": {"source": "github", "repo": repo}, "origin": {"scope": scope}});
        let before = json!({"requirements": [req("a/one", "managed")], "resolved": []});
        let now = json!({"requirements": [req("a/one", "managed"), req("evil/x", "user"), req("team/new", "managed")], "resolved": [1]});
        let v = without_added(&before, &now).unwrap();
        let repos: Vec<&str> = v["requirements"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["spec"]["repo"].as_str().unwrap())
            .collect();
        assert_eq!(repos, ["a/one", "team/new"]);
        assert_eq!(v["resolved"], json!([1]));
        assert!(without_added(&before, &before).is_none());
    }

    #[test]
    fn the_users_servers_are_turned_off_and_the_rest_kept() {
        let v = local_mcp(
            Some(r#"{"mcpServers":{"mine":{"command":"x"}},"other":1}"#),
            &["github".into(), "memory".into()],
        );
        assert_eq!(v["mcpServers"]["github"], json!({"disabled": true}));
        assert_eq!(v["mcpServers"]["memory"], json!({"disabled": true}));
        assert_eq!(v["mcpServers"]["mine"]["command"], "x");
        assert_eq!(v["other"], 1);
        assert_eq!(
            local_mcp(Some("not json"), &["a".into()])["mcpServers"]["a"]["disabled"],
            true
        );
    }
}
