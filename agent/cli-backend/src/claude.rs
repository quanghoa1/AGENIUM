//! Claude Code CLI (`claude`) driver, checked with claude 2.1.310 and Haiku on 2026-10-24 (ADR-56).
//!
//! - Headless: `claude -p --input-format stream-json --output-format stream-json --verbose`; each stdin line
//!   `{"type":"user","message":{"role":"user","content":"..."}}` runs one turn in the same session; events: `system`
//!   (`init`), `assistant` (text, tool_use), `user` (tool_result), `result` (end of turn, usage).
//! - Isolation: `CLAUDE_CONFIG_DIR` = the per-run home with Agenium's `settings.json` (its hook only), its MCP
//!   config (`--strict-mcp-config`, so no other MCP server) and a copy of the user's login (`.credentials.json`; a
//!   login refreshed during the run is handed back). `--setting-sources user` loads only that directory's settings,
//!   not the repository's. The user's `~/.claude.json`, settings, CLAUDE.md, skills and plugins are not used.
//!   (macOS keeps the login in the Keychain: not verified there.)
//! - Control: YOLO (`--dangerously-skip-permissions`) on the host with a `PreToolUse` hook on every tool that asks
//!   Agenium. Claude Code runs hooks through a POSIX shell (Git Bash on Windows) and treats only exit code 2 as
//!   blocking, so the hook script exits 2 whenever Agenium cannot be reached: it fails closed.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use agenium_llm::Usage;
use serde_json::{json, Value};

use crate::driver::{
    copy_login, hand_back_login, inside, Check, CheckCtx, Control, Driver, Event, Setup, Turns,
};

pub struct Claude;

const HOOK_URL_ENV: &str = "AGENIUM_HOOK_URL";
const HOOK_TOKEN_ENV: &str = "AGENIUM_HOOK_TOKEN";
const MCP_FILE: &str = "agenium-mcp.json";
const HOOK_FILE: &str = "agenium-hook.sh";
const LOGIN: &str = ".credentials.json";

/// The user's own Claude Code configuration folder (`CLAUDE_CONFIG_DIR` or `~/.claude`).
pub fn user_claude_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("CLAUDE_CONFIG_DIR") {
        return Some(PathBuf::from(d));
    }
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(|h| PathBuf::from(h).join(".claude"))
}

/// Forward slashes: the hook command runs in a POSIX shell, also on Windows.
fn posix(p: &Path) -> String {
    p.display().to_string().replace('\\', "/")
}

impl Driver for Claude {
    fn control(&self) -> Control {
        Control::Hook
    }

    fn turns(&self) -> Turns {
        Turns::Stdin
    }

    fn args(&self, model: &str, home: &Path, _resume: Option<&str>) -> Vec<String> {
        let mut a: Vec<String> = [
            "-p",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--verbose",
            "--dangerously-skip-permissions",
            "--setting-sources",
            "user",
            "--strict-mcp-config",
            "--no-session-persistence",
        ]
        .map(str::to_owned)
        .to_vec();
        a.extend([
            "--mcp-config".to_owned(),
            home.join(MCP_FILE).display().to_string(),
        ]);
        if !model.is_empty() {
            a.extend(["--model".to_owned(), model.to_owned()]);
        }
        a
    }

    fn prepare(&self, s: &Setup) -> std::io::Result<BTreeMap<String, String>> {
        let user = user_claude_dir()
            .ok_or_else(|| std::io::Error::other("cannot find the user's Claude Code folder"))?;
        copy_login(&user.join(LOGIN), &s.home.join(LOGIN))?;
        let script = s.home.join(HOOK_FILE);
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\ncurl -s -S --fail-with-body -X POST -H 'Content-Type: application/json' -H \"Authorization: Bearer ${HOOK_TOKEN_ENV}\" --data-binary @- \"${HOOK_URL_ENV}\" || exit 2\n"
            ),
        )?;
        std::fs::write(
            s.home.join("settings.json"),
            serde_json::to_vec_pretty(&json!({"hooks": {"PreToolUse": [{
                "matcher": "*",
                "hooks": [{"type": "command", "command": format!("sh {}", posix(&script)), "timeout": 900}],
            }]}}))?,
        )?;
        std::fs::write(
            s.home.join(MCP_FILE),
            serde_json::to_vec_pretty(&json!({"mcpServers": {s.mcp_name: {
                "type": "http",
                "url": s.mcp_url,
                "headers": {"Authorization": format!("Bearer ${{{HOOK_TOKEN_ENV}}}")},
            }}}))?,
        )?;
        let token = s
            .mcp_authorization
            .strip_prefix("Bearer ")
            .unwrap_or(s.mcp_authorization);
        Ok([
            ("CLAUDE_CONFIG_DIR".to_owned(), s.home.display().to_string()),
            (HOOK_URL_ENV.to_owned(), s.hook_url.to_owned()),
            (HOOK_TOKEN_ENV.to_owned(), token.to_owned()),
        ]
        .into())
    }

    fn finish(&self, s: &Setup) -> std::io::Result<()> {
        match user_claude_dir() {
            Some(user) => hand_back_login(&user.join(LOGIN), &s.home.join(LOGIN)),
            None => Ok(()),
        }
    }

    fn user_input(&self, text: &str) -> String {
        json!({"type": "user", "message": {"role": "user", "content": text}}).to_string()
    }

    fn parse(&self, v: &Value) -> Vec<Event> {
        match v["type"].as_str() {
            Some("system") if v["subtype"] == "init" => v["session_id"]
                .as_str()
                .map(|s| vec![Event::Session(s.to_owned())])
                .unwrap_or_default(),
            Some("assistant") => v["message"]["content"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|c| match c["type"].as_str() {
                    Some("text") => c["text"]
                        .as_str()
                        .filter(|t| !t.is_empty())
                        .map(|t| Event::Text(format!("{t}\n"))),
                    Some("tool_use") => Some(Event::Tool {
                        id: c["id"].as_str().unwrap_or("").to_owned(),
                        name: c["name"].as_str().unwrap_or("tool").to_owned(),
                        input: c["input"].clone(),
                        done: false,
                    }),
                    _ => None,
                })
                .collect(),
            Some("user") => v["message"]["content"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|c| c["type"] == "tool_result")
                .map(|c| Event::Tool {
                    id: c["tool_use_id"].as_str().unwrap_or("").to_owned(),
                    name: String::new(),
                    input: Value::Null,
                    done: true,
                })
                .collect(),
            Some("result") => {
                let u = &v["usage"];
                let n = |k: &str| u[k].as_u64().unwrap_or(0);
                vec![Event::TurnEnd {
                    usage: Usage {
                        input: n("input_tokens"),
                        output: n("output_tokens"),
                        cache_read: n("cache_read_input_tokens"),
                        cache_write: n("cache_creation_input_tokens"),
                        reasoning: u["output_tokens_details"]["thinking_tokens"]
                            .as_u64()
                            .unwrap_or(0),
                    },
                    response: v["result"].as_str().unwrap_or("").to_owned(),
                    denied: vec![],
                    error: (v["is_error"] == true).then(|| {
                        format!(
                            "claude turn ended with {}: {}",
                            v["subtype"].as_str().unwrap_or("an error"),
                            v["result"].as_str().unwrap_or("")
                        )
                    }),
                }]
            }
            _ => vec![],
        }
    }

    fn command(&self, tool: &str, input: &Value) -> Option<String> {
        matches!(tool, "Bash" | "PowerShell")
            .then(|| input["command"].as_str().map(str::to_owned))
            .flatten()
    }

    fn is_agenium_call(&self, tool: &str, _input: &Value, server: &str) -> bool {
        tool.starts_with(&format!("mcp__{server}__"))
    }

    fn check(&self, payload: &Value, cx: &CheckCtx) -> Check {
        let name = payload["tool_name"].as_str().unwrap_or("");
        let input = &payload["tool_input"];
        if name.starts_with("mcp__") {
            return if self.is_agenium_call(name, input, cx.mcp_name) {
                Check::Allow
            } else {
                Check::Deny(format!("{name} is not available in Agenium runs"))
            };
        }
        if matches!(name, "WebFetch" | "WebSearch") {
            return Check::Deny("web tools are disabled in Agenium runs".into());
        }
        for key in ["file_path", "path", "notebook_path"] {
            if let Some(p) = input[key].as_str() {
                let p = Path::new(p);
                if !inside(p, cx.worktree, cx.worktree) && !inside(p, cx.worktree, cx.home) {
                    return Check::Deny(format!("{} is outside the ticket worktree", p.display()));
                }
            }
        }
        match self.command(name, input) {
            Some(c) => Check::Command(c),
            None if matches!(name, "Bash" | "PowerShell") => {
                Check::Deny("the command text is missing".into())
            }
            None => Check::Allow,
        }
    }

    fn hook_answer(&self, allow: bool, reason: &str) -> Value {
        json!({"hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": if allow { "allow" } else { "deny" },
            "permissionDecisionReason": reason,
        }})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lines captured from claude 2.1.310 (shortened).
    #[test]
    fn parses_the_stream() {
        let c = Claude;
        let ev = |s: &str| c.parse(&serde_json::from_str(s).unwrap());
        assert_eq!(
            ev(
                r#"{"type":"system","subtype":"init","session_id":"s1","mcp_servers":[{"name":"agenium","status":"connected"}]}"#
            ),
            vec![Event::Session("s1".into())]
        );
        assert_eq!(
            ev(
                r#"{"type":"assistant","message":{"content":[{"type":"text","text":"On it."},{"type":"tool_use","id":"toolu_1","name":"Bash","input":{"command":"git status"}}]}}"#
            ),
            vec![
                Event::Text("On it.\n".into()),
                Event::Tool {
                    id: "toolu_1".into(),
                    name: "Bash".into(),
                    input: json!({"command": "git status"}),
                    done: false
                }
            ]
        );
        assert_eq!(
            ev(
                r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_1","content":"ok"}]}}"#
            ),
            vec![Event::Tool {
                id: "toolu_1".into(),
                name: String::new(),
                input: Value::Null,
                done: true
            }]
        );
        match &ev(
            r#"{"type":"result","subtype":"success","is_error":false,"result":"Done.","usage":{"input_tokens":10,"cache_creation_input_tokens":7564,"cache_read_input_tokens":114296,"output_tokens":764,"output_tokens_details":{"thinking_tokens":226}}}"#,
        )[0]
        {
            Event::TurnEnd {
                usage,
                response,
                error,
                ..
            } => {
                assert_eq!(
                    (
                        usage.input,
                        usage.cache_read,
                        usage.cache_write,
                        usage.output,
                        usage.reasoning
                    ),
                    (10, 114296, 7564, 764, 226)
                );
                assert_eq!(response, "Done.");
                assert!(error.is_none());
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            &ev(r#"{"type":"result","subtype":"error_max_turns","is_error":true,"result":""}"#)[0],
            Event::TurnEnd { error: Some(_), .. }
        ));
    }

    #[test]
    fn checks_tool_calls_and_answers_like_claude_code() {
        let wt = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let cx = CheckCtx {
            worktree: wt.path(),
            home: home.path(),
            mcp_name: "agenium",
        };
        let call = |name: &str, input: Value| {
            Claude.check(&json!({"tool_name": name, "tool_input": input}), &cx)
        };
        assert_eq!(
            call(
                "Write",
                json!({"file_path": wt.path().join("a.txt"), "content": "x"})
            ),
            Check::Allow
        );
        assert!(matches!(
            call("Edit", json!({"file_path": wt.path().join("../x.txt")})),
            Check::Deny(_)
        ));
        assert!(matches!(
            call(
                "Read",
                json!({"file_path": std::env::temp_dir().join("secret.txt")})
            ),
            Check::Deny(_)
        ));
        assert_eq!(
            call("Bash", json!({"command": "cargo test"})),
            Check::Command("cargo test".into())
        );
        assert_eq!(call("mcp__agenium__submit_report", json!({})), Check::Allow);
        assert!(matches!(
            call("mcp__github__create_issue", json!({})),
            Check::Deny(_)
        ));
        assert!(matches!(
            call("WebFetch", json!({"url": "https://x"})),
            Check::Deny(_)
        ));
        assert_eq!(call("TodoWrite", json!({"todos": []})), Check::Allow);
        assert_eq!(
            Claude.hook_answer(false, "no")["hookSpecificOutput"]["permissionDecision"],
            "deny"
        );
    }

    #[test]
    fn args_point_at_the_per_run_home() {
        let a = Claude.args("haiku", Path::new("/h"), None);
        assert!(a.contains(&"--strict-mcp-config".to_owned()));
        let i = a.iter().position(|x| x == "--setting-sources").unwrap();
        assert_eq!(a[i + 1], "user");
        let i = a.iter().position(|x| x == "--mcp-config").unwrap();
        assert!(a[i + 1].ends_with(MCP_FILE));
    }
}
