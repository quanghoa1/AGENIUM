//! Antigravity CLI (`agy`) driver, checked with agy 1.4.4 on 2026-10-24 (ADR-56).
//!
//! - Headless: `agy --input-format stream-json --output-format stream-json --mode accept-edits
//!   --dangerously-skip-permissions --model <m> -p=`; each stdin line
//!   `{"event":"user","message":{"role":"user","content":"..."}}` runs one turn in the same session.
//! - Isolation: agy keeps its configuration under `<home>/.gemini` and its login in the OS credential store, so
//!   `USERPROFILE`/`HOME` pointing at the per-run home gives a clean agy (none of the user's MCP servers, hooks, rules
//!   or permissions) that is still logged in. Agenium writes `.gemini/config/mcp_config.json` (its own MCP server
//!   only), `.gemini/config/hooks.json` and `.gemini/antigravity-cli/settings.json` (workspace = the worktree).
//! - Control: agy runs in its YOLO mode on the host (a sandboxed process cannot reach the credential store). Its
//!   `PreToolUse` hook runs before every tool call and posts the call to Agenium (`curl`, token from the
//!   environment); Agenium answers allow/deny. A hook that fails (Agenium unreachable) blocks the tool.

use std::collections::BTreeMap;
use std::path::Path;

use agenium_llm::Usage;
use serde_json::{json, Value};

use crate::driver::{inside, Check, CheckCtx, Control, Driver, Event, Setup, Turns};

pub struct Agy;

const HOOK_URL_ENV: &str = "AGENIUM_HOOK_URL";
const HOOK_TOKEN_ENV: &str = "AGENIUM_HOOK_TOKEN";

fn write_json(path: &Path, v: &Value) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, serde_json::to_vec_pretty(v)?)
}

/// The hook: a script in the per-run config folder (the hook's working directory) that posts the hook payload
/// (stdin) to Agenium and prints Agenium's answer; `--fail-with-body` makes a refused or failed request fail the
/// hook, which blocks the tool. A script, because agy's own `cmd /c` call mangles quoted arguments. Returns the hook
/// command and the script (file name, content).
fn hook_script() -> (&'static str, &'static str, String) {
    if cfg!(windows) {
        (
            "agenium-hook.cmd",
            "agenium-hook.cmd",
            format!(
                "@curl.exe -s -S --fail-with-body -X POST -H \"Content-Type: application/json\" -H \"Authorization: Bearer %{HOOK_TOKEN_ENV}%\" --data-binary @- \"%{HOOK_URL_ENV}%\"\r\n"
            ),
        )
    } else {
        (
            "sh ./agenium-hook.sh",
            "agenium-hook.sh",
            format!(
                "#!/bin/sh\nexec curl -s -S --fail-with-body -X POST -H 'Content-Type: application/json' -H \"Authorization: Bearer ${HOOK_TOKEN_ENV}\" --data-binary @- \"${HOOK_URL_ENV}\"\n"
            ),
        )
    }
}

/// Argument names that hold file system paths.
fn is_path_key(k: &str) -> bool {
    let k = k.to_ascii_lowercase();
    [
        "path",
        "paths",
        "file",
        "files",
        "directory",
        "dir",
        "cwd",
        "uri",
    ]
    .iter()
    .any(|s| k.ends_with(s))
}

fn path_values(v: &Value) -> Vec<String> {
    match v {
        Value::String(s) => vec![s.strip_prefix("file:///").unwrap_or(s).to_owned()],
        Value::Array(a) => a.iter().flat_map(path_values).collect(),
        _ => vec![],
    }
}

impl Driver for Agy {
    fn control(&self) -> Control {
        Control::Hook
    }

    fn turns(&self) -> Turns {
        Turns::Stdin
    }

    fn args(&self, model: &str, _resume: Option<&str>) -> Vec<String> {
        let mut a: Vec<String> = [
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--mode",
            "accept-edits",
            "--dangerously-skip-permissions",
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
        let (command, script, content) = hook_script();
        std::fs::write(gemini.join("config").join(script), content)?;
        write_json(
            &gemini.join("config").join("hooks.json"),
            &json!({"agenium-permissions": {"PreToolUse": [{
                "matcher": "*",
                "hooks": [{"type": "command", "command": command, "timeout": 900}],
            }]}}),
        )?;
        write_json(
            &gemini.join("antigravity-cli").join("settings.json"),
            &json!({
                "allowNonWorkspaceAccess": false,
                "trustedWorkspaces": [s.worktree],
            }),
        )?;
        let home = s.home.display().to_string();
        let token = s
            .mcp_authorization
            .strip_prefix("Bearer ")
            .unwrap_or(s.mcp_authorization);
        Ok([
            ("USERPROFILE".to_owned(), home.clone()),
            ("HOME".to_owned(), home),
            (HOOK_URL_ENV.to_owned(), s.hook_url.to_owned()),
            (HOOK_TOKEN_ENV.to_owned(), token.to_owned()),
        ]
        .into())
    }

    fn user_input(&self, text: &str) -> String {
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

    fn check(&self, payload: &Value, cx: &CheckCtx) -> Check {
        let call = &payload["toolCall"];
        let name = call["name"].as_str().unwrap_or("");
        let args = &call["args"];
        let lower = name.to_ascii_lowercase();
        if ["browser", "url", "web"].iter().any(|w| lower.contains(w)) {
            return Check::Deny("web and browser tools are disabled in Agenium runs".into());
        }
        if name == "call_mcp_tool" {
            return if args["ServerName"] == cx.mcp_name {
                Check::Allow
            } else {
                Check::Deny(format!(
                    "MCP server {} is not available in Agenium runs",
                    args["ServerName"]
                ))
            };
        }
        for (k, v) in args.as_object().into_iter().flatten() {
            if !is_path_key(k) {
                continue;
            }
            for p in path_values(v) {
                let p = Path::new(&p);
                if !inside(p, cx.worktree, cx.worktree) && !inside(p, cx.worktree, cx.home) {
                    return Check::Deny(format!("{} is outside the ticket worktree", p.display()));
                }
            }
        }
        match name {
            "run_command" => match args["CommandLine"].as_str() {
                Some(c) => Check::Command(c.to_owned()),
                None => Check::Deny("the command text is missing".into()),
            },
            _ => Check::Allow,
        }
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
    }

    #[test]
    fn checks_tool_calls() {
        let wt = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let cx = CheckCtx {
            worktree: wt.path(),
            home: home.path(),
            mcp_name: "agenium",
        };
        let call = |name: &str, args: Value| {
            Agy.check(&json!({"toolCall": {"name": name, "args": args}}), &cx)
        };
        assert_eq!(
            call(
                "write_file",
                json!({"Path": wt.path().join("a.txt"), "Content": "x"})
            ),
            Check::Allow
        );
        assert_eq!(
            call("view_file", json!({"AbsolutePath": "src/lib.rs"})),
            Check::Allow
        );
        assert!(matches!(
            call(
                "write_file",
                json!({"Path": wt.path().join("../escape.txt")})
            ),
            Check::Deny(_)
        ));
        assert!(matches!(
            call("list_dir", json!({"DirectoryPath": std::env::temp_dir()})),
            Check::Deny(_)
        ));
        assert_eq!(
            call("run_command", json!({"CommandLine": "cargo test"})),
            Check::Command("cargo test".into())
        );
        assert!(matches!(
            call(
                "run_command",
                json!({"CommandLine": "dir", "Cwd": std::env::temp_dir()})
            ),
            Check::Deny(_)
        ));
        assert_eq!(
            call(
                "call_mcp_tool",
                json!({"ServerName": "agenium", "ToolName": "submit_report"})
            ),
            Check::Allow
        );
        assert!(matches!(
            call("call_mcp_tool", json!({"ServerName": "figma"})),
            Check::Deny(_)
        ));
        assert!(matches!(
            call("read_url_content", json!({"Url": "https://example.com"})),
            Check::Deny(_)
        ));
        assert!(matches!(
            call("browser_click_element", json!({})),
            Check::Deny(_)
        ));
    }

    #[test]
    fn writes_an_isolated_home() {
        let home = tempfile::tempdir().unwrap();
        let env = Agy
            .prepare(&Setup {
                home: home.path(),
                worktree: Path::new("/wt"),
                mcp_name: "agenium",
                mcp_url: "http://127.0.0.1:1/mcp",
                mcp_authorization: "Bearer t",
                hook_url: "http://127.0.0.1:1/hook",
            })
            .unwrap();
        assert_eq!(env["USERPROFILE"], home.path().display().to_string());
        assert_eq!(env[HOOK_TOKEN_ENV], "t");
        let read = |p: &str| -> Value {
            serde_json::from_slice(&std::fs::read(home.path().join(p)).unwrap()).unwrap()
        };
        let mcp = read(".gemini/config/mcp_config.json");
        assert_eq!(mcp["mcpServers"].as_object().unwrap().len(), 1);
        assert_eq!(
            mcp["mcpServers"]["agenium"]["headers"]["Authorization"],
            "Bearer t"
        );
        let hooks = read(".gemini/config/hooks.json");
        let cmd = hooks["agenium-permissions"]["PreToolUse"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap();
        assert!(cmd.contains("agenium-hook") && !cmd.contains('"'), "{cmd}");
        let script =
            std::fs::read_to_string(home.path().join(".gemini/config").join(hook_script().1))
                .unwrap();
        assert!(script.contains("--fail-with-body") && script.contains(HOOK_URL_ENV));
        assert!(
            !script.contains("Bearer t"),
            "the token stays in the environment"
        );
        let s = read(".gemini/antigravity-cli/settings.json");
        assert_eq!(s["allowNonWorkspaceAccess"], false);
        assert!(s.get("permissions").is_none());
    }
}
