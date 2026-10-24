//! Antigravity CLI (`agy`) driver, checked with agy 1.4.4 on 2026-10-24 (ADR-56).
//!
//! - Headless: `agy --input-format stream-json --output-format stream-json --mode accept-edits --model <m> -p=`;
//!   each stdin line `{"event":"user","message":{"role":"user","content":"..."}}` runs one turn in the same session.
//! - Isolation: agy keeps its configuration under `<home>/.gemini` and its login outside it, so `USERPROFILE`/`HOME`
//!   pointing at the per-run home gives a clean agy (none of the user's MCP servers, hooks, rules or permissions) that
//!   is still logged in. Agenium writes `.gemini/config/mcp_config.json` (its own MCP server only) and
//!   `.gemini/antigravity-cli/settings.json` (workspace = the worktree, allow-rules = the project allowlist +
//!   Agenium's MCP tools).
//! - Headless agy cannot ask for permission: anything without an allow-rule is refused and the turn ends; the
//!   `result` event lists `denied_actions`.

use std::collections::BTreeMap;

use agenium_llm::Usage;
use serde_json::{json, Value};

use crate::driver::{Driver, Event, Setup};

pub struct Agy;

fn write_json(path: &std::path::Path, v: &Value) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, serde_json::to_vec_pretty(v)?)
}

impl Driver for Agy {
    fn args(&self, model: &str) -> Vec<String> {
        let mut a: Vec<String> = [
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--mode",
            "accept-edits",
        ]
        .map(str::to_owned)
        .to_vec();
        if !model.is_empty() {
            a.extend(["--model".to_owned(), model.to_owned()]);
        }
        // `-p` takes a value; the prompts come from stdin.
        a.push("-p=".to_owned());
        a
    }

    fn prepare(&self, s: &Setup) -> std::io::Result<BTreeMap<String, String>> {
        let gemini = s.home.join(".gemini");
        write_json(
            &gemini.join("config").join("mcp_config.json"),
            &json!({"mcpServers": {s.mcp_name: {
                "disabled": false,
                "serverUrl": s.mcp_url,
                "headers": {"Authorization": s.mcp_authorization},
            }}}),
        )?;
        let mut allow: Vec<String> = s
            .allowed_commands
            .iter()
            .map(|c| format!("command({})", c.trim()))
            .collect();
        allow.push(format!("mcp({}/*)", s.mcp_name));
        write_json(
            &gemini.join("antigravity-cli").join("settings.json"),
            &json!({
                "allowNonWorkspaceAccess": false,
                "permissions": {"allow": allow},
                "trustedWorkspaces": [s.worktree],
            }),
        )?;
        let home = s.home.display().to_string();
        Ok([
            ("USERPROFILE".to_owned(), home.clone()),
            ("HOME".to_owned(), home),
        ]
        .into())
    }

    fn user_line(&self, text: &str) -> String {
        json!({"event": "user", "message": {"role": "user", "content": text}}).to_string()
    }

    fn parse(&self, v: &Value) -> Vec<Event> {
        match v["event"].as_str() {
            Some("init") => v["conversation_id"]
                .as_str()
                .map(|c| vec![Event::Session(c.to_owned())])
                .unwrap_or_default(),
            Some("step_update") => {
                let s = &v["step_update"];
                match s["step_type"].as_str() {
                    Some("agent_response") => s["text_delta"]
                        .as_str()
                        .filter(|t| !t.is_empty())
                        .map(|t| vec![Event::Text(t.to_owned())])
                        .unwrap_or_default(),
                    Some("tool") => vec![Event::Tool {
                        id: s["step_index"].to_string(),
                        name: s["tool_name"].as_str().unwrap_or("tool").to_owned(),
                        input: s["tool_info"]["parameters"].clone(),
                        done: s["state"] == "DONE",
                    }],
                    _ => vec![],
                }
            }
            Some("result") => {
                let r = &v["result"];
                let n = |k: &str| r["usage"][k].as_u64().unwrap_or(0);
                let cached = n("cache_read_tokens");
                let status = r["status"].as_str().unwrap_or("");
                vec![Event::TurnEnd {
                    usage: Usage {
                        input: n("input_tokens").saturating_sub(cached),
                        output: n("output_tokens"),
                        cache_read: cached,
                        cache_write: 0,
                        reasoning: n("thinking_tokens"),
                    },
                    response: r["response"].as_str().unwrap_or("").to_owned(),
                    denied: r["denied_actions"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|d| d["action"].as_str().map(str::to_owned))
                        .collect(),
                    error: (status != "SUCCESS").then(|| {
                        let e = r["error"].as_str().unwrap_or("");
                        format!("agy turn ended with status {status} {e}")
                            .trim()
                            .to_owned()
                    }),
                }]
            }
            _ => vec![],
        }
    }

    fn command(&self, tool: &str, input: &Value) -> Option<String> {
        (tool == "run_command")
            .then(|| input["CommandLine"].as_str().map(str::to_owned))
            .flatten()
    }

    fn is_agenium_call(&self, tool: &str, input: &Value, server: &str) -> bool {
        tool == "call_mcp_tool" && input["ServerName"] == server
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lines captured from agy 1.4.4 (shortened).
    #[test]
    fn parses_the_stream() {
        let a = Agy;
        let ev = |s: &str| a.parse(&serde_json::from_str(s).unwrap());
        assert_eq!(
            ev(r#"{"event":"init","conversation_id":"c1","init":{"cwd":"x"}}"#),
            vec![Event::Session("c1".into())]
        );
        assert_eq!(
            ev(
                r#"{"event":"step_update","step_update":{"step_index":1,"state":"DONE","step_type":"agent_response","text_delta":"PONG"}}"#
            ),
            vec![Event::Text("PONG".into())]
        );
        assert_eq!(
            ev(
                r#"{"event":"step_update","step_update":{"step_index":4,"state":"ACTIVE","step_type":"tool","tool_name":"run_command","tool_info":{"name":"run_command","parameters":{"CommandLine":"echo hi"}}}}"#
            ),
            vec![Event::Tool {
                id: "4".into(),
                name: "run_command".into(),
                input: json!({"CommandLine": "echo hi"}),
                done: false
            }]
        );
        let end = ev(
            r#"{"event":"result","result":{"status":"SUCCESS","response":"","usage":{"input_tokens":23438,"output_tokens":165,"thinking_tokens":0,"cache_read_tokens":438,"total_tokens":23603},"denied_actions":[{"action":"command","display_name":"RunCommand"}]}}"#,
        );
        match &end[0] {
            Event::TurnEnd {
                usage,
                denied,
                error,
                ..
            } => {
                assert_eq!(
                    (usage.input, usage.cache_read, usage.output),
                    (23000, 438, 165)
                );
                assert_eq!(denied, &vec!["command".to_owned()]);
                assert!(error.is_none());
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            a.command("run_command", &json!({"CommandLine": "git status"}))
                .as_deref(),
            Some("git status")
        );
        assert!(a.is_agenium_call(
            "call_mcp_tool",
            &json!({"ServerName": "agenium", "ToolName": "submit_report"}),
            "agenium"
        ));
        assert!(!a.is_agenium_call("call_mcp_tool", &json!({"ServerName": "figma"}), "agenium"));
    }

    #[test]
    fn writes_an_isolated_home() {
        let home = tempfile::tempdir().unwrap();
        let env = Agy
            .prepare(&Setup {
                home: home.path(),
                worktree: std::path::Path::new("/wt"),
                mcp_name: "agenium",
                mcp_url: "http://127.0.0.1:1/mcp",
                mcp_authorization: "Bearer t",
                allowed_commands: &["cargo test".to_owned()],
            })
            .unwrap();
        assert_eq!(env["USERPROFILE"], home.path().display().to_string());
        let mcp: Value = serde_json::from_slice(
            &std::fs::read(home.path().join(".gemini/config/mcp_config.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(mcp["mcpServers"].as_object().unwrap().len(), 1);
        assert_eq!(
            mcp["mcpServers"]["agenium"]["headers"]["Authorization"],
            "Bearer t"
        );
        let s: Value = serde_json::from_slice(
            &std::fs::read(home.path().join(".gemini/antigravity-cli/settings.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            s["permissions"]["allow"],
            json!(["command(cargo test)", "mcp(agenium/*)"])
        );
        assert_eq!(s["allowNonWorkspaceAccess"], false);
    }
}
