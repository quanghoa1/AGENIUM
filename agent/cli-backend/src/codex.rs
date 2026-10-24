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

use agenium_llm::Usage;
use serde_json::{json, Value};

use crate::driver::{Control, Driver, Event, Setup, Turns};

pub struct Codex;

const TOKEN_ENV: &str = "AGENIUM_MCP_TOKEN";
const ORIGINAL_LOGIN: &str = "auth.json.agenium-original";

/// The user's own codex home (`CODEX_HOME` or `~/.codex`).
pub fn user_codex_home() -> Option<PathBuf> {
    if let Some(h) = std::env::var_os("CODEX_HOME") {
        return Some(PathBuf::from(h));
    }
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(|h| PathBuf::from(h).join(".codex"))
}

fn toml_str(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_default()
}

impl Codex {
    fn finish_login(&self, home: &Path, user: &Path) -> std::io::Result<()> {
        let (Ok(now), Ok(before)) = (
            std::fs::read(home.join("auth.json")),
            std::fs::read(home.join(ORIGINAL_LOGIN)),
        ) else {
            return Ok(());
        };
        if now == before || now.is_empty() {
            return Ok(());
        }
        let target = user.join("auth.json");
        if std::fs::read(&target)? != before {
            tracing::warn!(
                "codex login changed both inside and outside the run; keeping the user's"
            );
            return Ok(());
        }
        let tmp = user.join("auth.json.agenium-tmp");
        std::fs::write(&tmp, &now)?;
        std::fs::rename(&tmp, &target)
    }
}

impl Driver for Codex {
    fn control(&self) -> Control {
        Control::Sandbox
    }

    fn turns(&self) -> Turns {
        Turns::ProcessPerTurn
    }

    fn args(&self, model: &str, resume: Option<&str>) -> Vec<String> {
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
        a.push("-".to_owned());
        a
    }

    fn prepare(&self, s: &Setup) -> std::io::Result<BTreeMap<String, String>> {
        let user = user_codex_home()
            .ok_or_else(|| std::io::Error::other("cannot find the user's codex home"))?;
        let login = std::fs::read(user.join("auth.json")).map_err(|e| {
            std::io::Error::other(format!("codex is not logged in ({}): {e}", user.display()))
        })?;
        std::fs::write(s.home.join("auth.json"), &login)?;
        std::fs::write(s.home.join(ORIGINAL_LOGIN), &login)?;
        std::fs::write(
            s.home.join("config.toml"),
            format!(
                "[mcp_servers.{name}]\nurl = {url}\nbearer_token_env_var = {env}\n",
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
            Some(user) => self.finish_login(s.home, &user),
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

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn args_for_new_and_resumed_turns() {
        let a = Codex.args("gpt-6-luna", None);
        assert_eq!(a[0], "exec");
        assert!(a.contains(&"--dangerously-bypass-approvals-and-sandbox".to_owned()));
        assert_eq!(a.last().unwrap(), "-");
        let r = Codex.args("gpt-6-luna", Some("t1"));
        assert_eq!(&r[..3], ["exec", "resume", "t1"]);
    }

    #[test]
    fn refreshed_login_goes_back_only_when_the_user_file_is_unchanged() {
        let home = tempfile::tempdir().unwrap();
        let user = tempfile::tempdir().unwrap();
        std::fs::write(user.path().join("auth.json"), "old").unwrap();
        std::fs::write(home.path().join(ORIGINAL_LOGIN), "old").unwrap();
        std::fs::write(home.path().join("auth.json"), "new").unwrap();
        Codex.finish_login(home.path(), user.path()).unwrap();
        assert_eq!(
            std::fs::read_to_string(user.path().join("auth.json")).unwrap(),
            "new"
        );
        // The user logged in again meanwhile: theirs wins.
        std::fs::write(user.path().join("auth.json"), "other").unwrap();
        std::fs::write(home.path().join(ORIGINAL_LOGIN), "new").unwrap();
        std::fs::write(home.path().join("auth.json"), "newer").unwrap();
        Codex.finish_login(home.path(), user.path()).unwrap();
        assert_eq!(
            std::fs::read_to_string(user.path().join("auth.json")).unwrap(),
            "other"
        );
    }
}
