//! Codex CLI driver, checked with codex 0.163.0 and `gpt-6-luna` on 2026-10-24 (ADR-56).
//!
//! - Headless: `codex exec --json ... -` reads the prompt from stdin, runs one turn and exits; a later turn of the
//!   same run is `codex exec resume <thread_id> --json ... -`.
//! - Isolation: `CODEX_HOME` = the per-run home with Agenium's own `config.toml` (its MCP server only) and a copy of
//!   the user's login (`auth.json`); the user's config, hooks, rules, skills and plugins are not there.
//!   If codex refreshes the login during the run, the new login is handed back to the user's `auth.json` afterwards
//!   (only when that file did not change meanwhile), so the user's own codex stays logged in.
//! - Control: codex runs in its YOLO mode (`--dangerously-bypass-approvals-and-sandbox`) inside Agenium's sandbox.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use agenium_core::Effort;
use agenium_llm::Usage;
use serde_json::{json, Value};

use crate::driver::toml_str;
use crate::driver::{copy_login, hand_back_login, Control, Driver, Event, Setup, Turns};

/// codex features an Agenium run never uses (codex 0.163 `codex features list`): browser and computer control, apps,
/// plugins, image generation, and (ADR-129) its own shell, image viewer, sleep and tool suggestions: commands and images
/// go through Agenium's `shell` and `view_image` over MCP, under Agenium's sandbox, gate and write scope. `multi_agent` (subagents) follows
/// the run (Solo only); web search is disabled.
const WITHHELD_FEATURES: [&str; 14] = [
    "browser_use",
    "browser_use_external",
    "computer_use",
    "in_app_browser",
    "apps",
    "plugins",
    "remote_plugin",
    "plugin_sharing",
    "image_generation",
    "shell_tool",
    "unified_exec",
    "view_image",
    "sleep_tool",
    "tool_suggest",
];

pub struct Codex;

const TOKEN_ENV: &str = "AGENIUM_MCP_TOKEN";

/// The user's own codex home (`CODEX_HOME` or `~/.codex`).
pub fn user_codex_home() -> Option<PathBuf> {
    if let Some(h) = std::env::var_os("CODEX_HOME") {
        return Some(PathBuf::from(h));
    }
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(|h| PathBuf::from(h).join(".codex"))
}

impl Driver for Codex {
    fn control(&self) -> Control {
        Control::Sandbox
    }

    fn turns(&self) -> Turns {
        Turns::ProcessPerTurn
    }

    fn args(
        &self,
        model: &str,
        effort: Option<&Effort>,
        _home: &Path,
        resume: Option<&str>,
        _subagents: bool,
    ) -> Vec<String> {
        let mut a = vec!["exec".to_owned()];
        if let Some(id) = resume {
            a.extend(["resume".to_owned(), id.to_owned()]);
        }
        a.extend(
            [
                "--json",
                "--skip-git-repo-check",
                "--ignore-rules",
                "--dangerously-bypass-approvals-and-sandbox",
            ]
            .map(str::to_owned),
        );
        if !model.is_empty() {
            a.extend(["-m".to_owned(), model.to_owned()]);
        }
        // The level as Codex's `model/list` names it for the model (ADR-114: the app offers only those).
        if let Some(e) = effort {
            a.extend(["-c".to_owned(), format!("model_reasoning_effort=\"{e}\"")]);
        }
        a.push("-".to_owned());
        a
    }

    fn prepare(&self, s: &Setup) -> std::io::Result<BTreeMap<String, String>> {
        let user = user_codex_home()
            .ok_or_else(|| std::io::Error::other("cannot find the user's codex home"))?;
        copy_login(&user.join("auth.json"), &s.home.join("auth.json"))?;
        std::fs::write(
            s.home.join("config.toml"),
            format!(
                "web_search = \"disabled\"\n\n[features]\nmulti_agent = {sub}\n{off}\n[mcp_servers.{name}]\nurl = {url}\nbearer_token_env_var = {env}\n",
                sub = s.subagents,
                off = WITHHELD_FEATURES
                    .iter()
                    .map(|f| format!("{f} = false\n"))
                    .collect::<String>(),
                name = s.mcp_name,
                url = toml_str(s.mcp_url),
                env = toml_str(TOKEN_ENV),
            ),
        )?;
        let token = s
            .mcp_authorization
            .strip_prefix("Bearer ")
            .unwrap_or(s.mcp_authorization);
        Ok([
            ("CODEX_HOME".to_owned(), s.home.display().to_string()),
            (TOKEN_ENV.to_owned(), token.to_owned()),
        ]
        .into())
    }

    fn finish(&self, s: &Setup) -> std::io::Result<()> {
        match user_codex_home() {
            Some(user) => hand_back_login(&user.join("auth.json"), &s.home.join("auth.json")),
            None => Ok(()),
        }
    }

    fn read_roots(&self, program: &Path) -> Vec<PathBuf> {
        // `<install>/bin/codex(.exe)`: the whole installation is read-only inside the sandbox.
        program
            .parent()
            .and_then(Path::parent)
            .map(|p| vec![p.to_path_buf()])
            .unwrap_or_default()
    }

    fn user_input(&self, text: &str) -> String {
        text.to_owned()
    }

    fn parse(&self, v: &Value) -> Vec<Event> {
        match v["type"].as_str() {
            Some("thread.started") => v["thread_id"]
                .as_str()
                .map(|t| vec![Event::Session(t.to_owned())])
                .unwrap_or_default(),
            Some(kind @ ("item.started" | "item.completed")) => {
                let item = &v["item"];
                let done = kind == "item.completed";
                match item["type"].as_str().unwrap_or("") {
                    "agent_message" if done => item["text"]
                        .as_str()
                        .filter(|t| !t.is_empty())
                        .map(|t| vec![Event::Text(format!("{t}\n"))])
                        .unwrap_or_default(),
                    "agent_message" | "reasoning" | "todo_list" | "error" => vec![],
                    "command_execution" => vec![Event::Tool {
                        id: item["id"].as_str().unwrap_or("").to_owned(),
                        name: "command_execution".into(),
                        input: json!({"command": item["command"]}),
                        done,
                    }],
                    "mcp_tool_call" => vec![Event::Tool {
                        id: item["id"].as_str().unwrap_or("").to_owned(),
                        name: "mcp_tool_call".into(),
                        input: json!({"server": item["server"], "tool": item["tool"], "arguments": item["arguments"]}),
                        done,
                    }],
                    other => {
                        let mut input = item.clone();
                        if let Some(o) = input.as_object_mut() {
                            o.remove("id");
                            o.remove("type");
                            o.remove("status");
                            o.remove("aggregated_output");
                        }
                        vec![Event::Tool {
                            id: item["id"].as_str().unwrap_or("").to_owned(),
                            name: other.to_owned(),
                            input,
                            done,
                        }]
                    }
                }
            }
            Some("turn.completed") => {
                let u = &v["usage"];
                let n = |k: &str| u[k].as_u64().unwrap_or(0);
                let cached = n("cached_input_tokens");
                let written = n("cache_write_input_tokens");
                vec![Event::TurnEnd {
                    usage: Usage {
                        input: n("input_tokens").saturating_sub(cached + written),
                        output: n("output_tokens"),
                        cache_read: cached,
                        cache_write: written,
                        cache_write_1h: 0,
                        reasoning: n("reasoning_output_tokens"),
                    },
                    response: String::new(),
                    denied: vec![],
                    error: None,
                }]
            }
            Some("turn.failed") => vec![Event::TurnEnd {
                usage: Usage::default(),
                response: String::new(),
                denied: vec![],
                error: Some(format!(
                    "codex turn failed: {}",
                    v["error"]["message"].as_str().unwrap_or("unknown error")
                )),
            }],
            _ => vec![],
        }
    }

    fn command(&self, tool: &str, input: &Value) -> Option<String> {
        (tool == "command_execution")
            .then(|| input["command"].as_str().map(str::to_owned))
            .flatten()
    }

    fn is_agenium_call(&self, tool: &str, input: &Value, server: &str) -> bool {
        tool == "mcp_tool_call" && input["server"] == server
    }
}

/// One request to codex's own app-server (no model turn), in a scratch `CODEX_HOME` holding a copy of the user's
/// login like a run (a refreshed login is handed back the same way). The client names itself `agenium`.
async fn app_server(program: &Path, scratch: &Path, method: &str) -> Result<Value, String> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let user = user_codex_home().ok_or("no codex home")?;
    // Deleted with the login copy when this function returns, whichever way.
    let dir = crate::driver::probe_dir(scratch, "codex-home")?;
    let home = dir.path().join("codex-home");
    let cwd = dir.path().join("cwd");
    for d in [&home, &cwd] {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    copy_login(&user.join("auth.json"), &home.join("auth.json")).map_err(|e| e.to_string())?;
    let mut cmd = tokio::process::Command::new(program);
    cmd.args([
        "-c",
        "approval_policy=never",
        "-s",
        "read-only",
        "app-server",
    ])
    .env("CODEX_HOME", &home)
    .current_dir(&cwd)
    .stdin(std::process::Stdio::piped())
    .stdout(std::process::Stdio::piped())
    .stderr(std::process::Stdio::null())
    .kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("cannot start codex: {e}"))?;
    let (Some(mut stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
        return Err("codex stdio pipes missing".into());
    };
    let ask = async {
        let send = |v: Value| format!("{v}\n");
        stdin
            .write_all(
                send(json!({"id": 1, "method": "initialize",
                    "params": {"clientInfo": {"name": "agenium", "version": env!("CARGO_PKG_VERSION")}}}))
                .as_bytes(),
            )
            .await
            .map_err(|e| e.to_string())?;
        let mut lines = BufReader::new(stdout).lines();
        while let Some(line) = lines.next_line().await.map_err(|e| e.to_string())? {
            let Ok(m) = serde_json::from_str::<Value>(line.trim()) else {
                continue;
            };
            match m["id"].as_u64() {
                Some(1) => {
                    let next = send(json!({"method": "initialized"}))
                        + &send(json!({"id": 2, "method": method, "params": {}}));
                    stdin
                        .write_all(next.as_bytes())
                        .await
                        .map_err(|e| e.to_string())?;
                }
                Some(2) if m["error"].is_object() => {
                    return Err(format!(
                        "codex {method}: {}",
                        m["error"]["message"].as_str().unwrap_or("failed")
                    ))
                }
                Some(2) => return Ok(m["result"].clone()),
                _ => {}
            }
        }
        Err("codex app-server ended without an answer".into())
    };
    let out = tokio::time::timeout(std::time::Duration::from_secs(40), ask)
        .await
        .unwrap_or_else(|_| Err("codex app-server did not answer in 40 s".into()));
    let _ = child.kill().await;
    if let Err(e) = hand_back_login(&user.join("auth.json"), &home.join("auth.json")) {
        tracing::warn!(error = %e, "codex login not handed back");
    }
    out
}

/// Codex's subscription usage through its app-server (`account/rateLimits/read`).
pub async fn rate_limits(
    program: &Path,
    scratch: &Path,
    now: &str,
) -> Result<agenium_llm::vendor_usage::VendorUsage, String> {
    app_server(program, scratch, "account/rateLimits/read")
        .await
        .map(|r| agenium_llm::vendor_usage::from_codex_rate_limits(&r, now))
}

/// Codex's `model/list` answer as it came (no model turn).
pub async fn model_list(program: &Path, scratch: &Path) -> Result<Value, String> {
    app_server(program, scratch, "model/list").await
}

/// The models this codex login can use (`model/list`, hidden ones left out), default first, each with its reasoning
/// efforts and the one Codex uses by default.
pub async fn models(program: &Path, scratch: &Path) -> Result<Vec<agenium_llm::ModelInfo>, String> {
    Ok(parse_model_list(&model_list(program, scratch).await?))
}

fn parse_model_list(r: &Value) -> Vec<agenium_llm::ModelInfo> {
    let mut out: Vec<(bool, agenium_llm::ModelInfo)> = r["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| m["hidden"] != true)
        .filter_map(|m| {
            let efforts = m["supportedReasoningEfforts"].as_array().map(|a| {
                a.iter()
                    .filter_map(|e| e["reasoningEffort"].as_str()?.parse().ok())
                    .collect()
            });
            Some((
                m["isDefault"] == true,
                agenium_llm::ModelInfo {
                    id: m["id"].as_str()?.to_owned(),
                    display_name: m["displayName"].as_str().map(str::to_owned),
                    efforts,
                    default_effort: m["defaultReasoningEffort"]
                        .as_str()
                        .and_then(|e| e.parse().ok()),
                    ..Default::default()
                },
            ))
        })
        .collect();
    out.sort_by_key(|(default, _)| !*default);
    out.into_iter().map(|(_, m)| m).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Review of 2026-10-27: the usage and model-list probes left their copy of the user's `auth.json` (OAuth
    /// tokens) in the persistent scratch folder, also when the probe failed.
    #[tokio::test]
    async fn the_probe_leaves_no_login_behind() {
        let user = tempfile::tempdir().unwrap();
        std::fs::write(user.path().join("auth.json"), "{\"tokens\":{}}").unwrap();
        let scratch = tempfile::tempdir().unwrap();
        let legacy = scratch.path().join("codex-home");
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join("auth.json"), "left by an earlier version").unwrap();
        std::env::set_var("CODEX_HOME", user.path());
        let r = models(&scratch.path().join("no-such-codex"), scratch.path()).await;
        std::env::remove_var("CODEX_HOME");
        assert!(r.is_err());
        let mut left = vec![];
        let mut dirs = vec![scratch.path().to_path_buf()];
        while let Some(d) = dirs.pop() {
            for e in std::fs::read_dir(d).unwrap().flatten() {
                if e.path().is_dir() {
                    dirs.push(e.path());
                } else {
                    left.push(e.path());
                }
            }
        }
        assert!(left.is_empty(), "{left:?}");
    }

    /// Lines captured from codex 0.163.0 (shortened).
    #[test]
    fn parses_the_stream() {
        let c = Codex;
        let ev = |s: &str| c.parse(&serde_json::from_str(s).unwrap());
        assert_eq!(
            ev(r#"{"type":"thread.started","thread_id":"t1"}"#),
            vec![Event::Session("t1".into())]
        );
        assert_eq!(
            ev(
                r#"{"type":"item.completed","item":{"id":"item_2","type":"agent_message","text":"DONE"}}"#
            ),
            vec![Event::Text("DONE\n".into())]
        );
        let started = ev(
            r#"{"type":"item.started","item":{"id":"item_1","type":"command_execution","command":"git status","aggregated_output":"","exit_code":null,"status":"in_progress"}}"#,
        );
        assert_eq!(
            started,
            vec![Event::Tool {
                id: "item_1".into(),
                name: "command_execution".into(),
                input: json!({"command": "git status"}),
                done: false
            }]
        );
        let mcp = ev(
            r#"{"type":"item.completed","item":{"id":"item_3","type":"mcp_tool_call","server":"agenium","tool":"submit_report","arguments":{"summary":"x"},"result":null,"error":null,"status":"completed"}}"#,
        );
        match &mcp[0] {
            Event::Tool {
                name, input, done, ..
            } => {
                assert!(*done);
                assert!(c.is_agenium_call(name, input, "agenium"));
            }
            other => panic!("{other:?}"),
        }
        match &ev(
            r#"{"type":"turn.completed","usage":{"input_tokens":54817,"cached_input_tokens":46208,"cache_write_input_tokens":0,"output_tokens":489,"reasoning_output_tokens":260}}"#,
        )[0]
        {
            Event::TurnEnd { usage, error, .. } => {
                assert_eq!(
                    (usage.input, usage.cache_read, usage.output, usage.reasoning),
                    (8609, 46208, 489, 260)
                );
                assert!(error.is_none());
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            &ev(r#"{"type":"turn.failed","error":{"message":"quota"}}"#)[0],
            Event::TurnEnd { error: Some(e), .. } if e.contains("quota")
        ));
    }

    /// `model/list` of codex 0.163 (shortened): each model names its efforts and its default.
    #[test]
    fn lists_models_with_their_efforts() {
        let r = json!({"data": [
            {"id": "gpt-6-luna", "displayName": "GPT-6-Luna", "isDefault": false, "hidden": false,
             "defaultReasoningEffort": "medium",
             "supportedReasoningEfforts": [{"reasoningEffort": "low"}, {"reasoningEffort": "medium"},
                 {"reasoningEffort": "high"}, {"reasoningEffort": "xhigh"}, {"reasoningEffort": "max"}]},
            {"id": "gpt-6.1-sol", "isDefault": true, "defaultReasoningEffort": "low",
             "supportedReasoningEfforts": [{"reasoningEffort": "low"}, {"reasoningEffort": "ultra"}]},
            {"id": "hidden-one", "hidden": true}
        ]});
        let m = parse_model_list(&r);
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].id, "gpt-6.1-sol", "default first");
        let names = |i: usize| {
            m[i].efforts
                .as_ref()
                .unwrap()
                .iter()
                .map(|e| e.as_str())
                .collect::<Vec<_>>()
                .join(",")
        };
        assert_eq!(names(0), "low,ultra");
        assert_eq!(names(1), "low,medium,high,xhigh,max");
        assert_eq!(m[1].default_effort, Some(Effort::MEDIUM));
    }

    #[test]
    fn args_for_new_and_resumed_turns() {
        let ultra: Effort = "ultra".parse().unwrap();
        let a = Codex.args("gpt-6.1-sol", Some(&ultra), Path::new("/h"), None, false);
        assert!(
            a.contains(&"model_reasoning_effort=\"ultra\"".to_owned()),
            "{a:?}"
        );
        assert_eq!(a[0], "exec");
        assert!(a.contains(&"--dangerously-bypass-approvals-and-sandbox".to_owned()));
        assert_eq!(a.last().unwrap(), "-");
        let r = Codex.args("gpt-6-luna", None, Path::new("/h"), Some("t1"), false);
        assert_eq!(&r[..3], ["exec", "resume", "t1"]);
    }
}
