//! Claude Code CLI (`claude`) driver, checked with claude 2.1.310 and Haiku on 2026-10-24 (ADR-56).
//!
//! - Headless: `claude -p --input-format stream-json --output-format stream-json --verbose`; each stdin line
//!   `{"type":"user","message":{"role":"user","content":"..."}}` runs one turn in the same session; events: `system`
//!   (`init`), `assistant` (text, tool_use), `user` (tool_result), `result` (end of turn, usage).
//! - Isolation: `CLAUDE_CONFIG_DIR` = the per-run home with Agenium's `settings.json` (its hook only), its MCP
//!   config (`--strict-mcp-config`, so no other MCP server) and a copy of the user's login (`.credentials.json`; a
//!   login refreshed during the run is handed back). `--setting-sources user` loads only that directory's settings,
//!   not the repository's. The user's `~/.claude.json`, settings, CLAUDE.md, skills and plugins are not used; the
//!   connectors and skills of the user's claude.ai account, which come with the login, are off
//!   (`ENABLE_CLAUDEAI_MCP_SERVERS=false`, `--disable-slash-commands`). Claude Code's own `@builtin` plugins stay.
//!   (macOS keeps the login in the Keychain: not verified there.)
//! - Control: YOLO (`--dangerously-skip-permissions`) on the host with a `PreToolUse` hook on every tool that asks
//!   Agenium. Claude Code runs hooks through a POSIX shell (Git Bash on Windows) and treats only exit code 2 as
//!   blocking, so the hook script exits 2 whenever Agenium cannot be reached: it fails closed.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use agenium_core::Effort;
use agenium_llm::Usage;
use serde_json::{json, Value};

use crate::driver::{
    copy_login, file_access, hand_back_login, hook_call_secs, original, own_tool_off, probe_dir,
    Check, CheckCtx, Control, Driver, Event, Setup, Turns, NO_SUBAGENTS,
};
use crate::driver::{HOOK_TIMEOUT, HOOK_TOKEN_ENV, HOOK_URL_ENV, MCP_CALL_TIMEOUT};

/// Claude Code tools an Agenium run may use (ADR-129): `ToolSearch`, which loads the Agenium MCP tools that do the
/// work, and its own file and search tools for its scratch folder only (the per-run home, where it saves a long tool
/// result and reads it back); the hook refuses them on the worktree. Its shells are not offered: commands go through
/// Agenium's `shell`. Everything else (scheduling, notifications, plugins, worktrees, web, workflows...) is withheld.
const BASE_TOOLS: [&str; 6] = ["ToolSearch", "Read", "Write", "Edit", "Glob", "Grep"];
/// Subagents and their messaging (Solo only).
/// Claude Code 2.1 calls its subagent tool `Agent` (`Task` before; `--tools Task` still offers it under the new name).
const SUBAGENT_TOOLS: [&str; 6] = [
    "Agent",
    "Task",
    "TaskOutput",
    "SendMessage",
    "ListAgents",
    "TaskStop",
];

fn allowed_tools(subagents: bool) -> Vec<&'static str> {
    let mut v = BASE_TOOLS.to_vec();
    if subagents {
        v.extend(SUBAGENT_TOOLS);
    }
    v
}

pub struct Claude;

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

/// The hook's command line, the script's path quoted for that shell. Unquoted, a path with a space (a profile folder
/// such as `John Smith`) split into two words: `sh` exited 127, which Claude Code takes for a hook error that does not
/// block, so every tool ran without Agenium's decision.
fn hook_command(script: &Path) -> String {
    format!("sh '{}'", posix(script).replace('\'', r"'\''"))
}

/// Agenium's files in the per-run home, out of reach of the model's file tools (`driver::file_access`).
fn protected(home: &Path) -> Vec<PathBuf> {
    let login = home.join(LOGIN);
    vec![
        home.join(HOOK_FILE),
        home.join("settings.json"),
        home.join(MCP_FILE),
        original(&login),
        login,
    ]
}

impl Claude {
    /// The events of one stream line, as the main agent's (a subagent's are wrapped by `parse`).
    fn parse_own(&self, v: &Value) -> Vec<Event> {
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
            Some("rate_limit_event") => {
                agenium_llm::vendor_usage::from_claude_event(v, &agenium_core::now_iso())
                    .map(Event::Limits)
                    .into_iter()
                    .collect()
            }
            Some("result") => {
                let u = &v["usage"];
                let n = |k: &str| u[k].as_u64().unwrap_or(0);
                vec![Event::TurnEnd {
                    usage: Usage {
                        input: n("input_tokens"),
                        output: n("output_tokens"),
                        cache_read: n("cache_read_input_tokens"),
                        cache_write: n("cache_creation_input_tokens"),
                        cache_write_1h: 0,
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
}

impl Driver for Claude {
    fn control(&self) -> Control {
        Control::Hook
    }

    fn turns(&self) -> Turns {
        Turns::Stdin
    }

    fn args(
        &self,
        model: &str,
        effort: Option<&Effort>,
        home: &Path,
        _resume: Option<&str>,
        subagents: bool,
    ) -> Vec<String> {
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
            // Skills and slash commands: the ones of the user's claude.ai account would come with the login.
            // Agenium's skills reach the agent through `load_skill`.
            "--disable-slash-commands",
            "--no-session-persistence",
        ]
        .map(str::to_owned)
        .to_vec();
        a.extend([
            "--mcp-config".to_owned(),
            home.join(MCP_FILE).display().to_string(),
            // Only the tools Agenium allows are offered at all (the hook enforces the same list).
            "--tools".to_owned(),
            allowed_tools(subagents).join(","),
        ]);
        if !model.is_empty() {
            a.extend(["--model".to_owned(), model.to_owned()]);
        }
        // Claude Code takes the API's level names (`--effort`); the model's own level (ADR-114).
        if let Some(l) = effort.and_then(|e| agenium_llm::effort::level_for(model, e)) {
            a.extend(["--effort".to_owned(), l.as_str().to_owned()]);
        }
        a
    }

    fn prepare(&self, s: &Setup) -> std::io::Result<BTreeMap<String, String>> {
        let user = user_claude_dir()
            .ok_or_else(|| std::io::Error::other("cannot find the user's Claude Code folder"))?;
        copy_login(&user.join(LOGIN), &s.home.join(LOGIN))?;
        let script = s.home.join(HOOK_FILE);
        // Claude Code goes on with the tool when it kills a hook that ran out of time: the call to Agenium gives up
        // first (`--max-time`) and the hook exits 2, which blocks.
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\ncurl -s -S --fail-with-body --max-time {} -X POST -H 'Content-Type: application/json' -H \"Authorization: Bearer ${HOOK_TOKEN_ENV}\" --data-binary @- \"${HOOK_URL_ENV}\" || exit 2\n",
                hook_call_secs(HOOK_TIMEOUT)
            ),
        )?;
        std::fs::write(
            s.home.join("settings.json"),
            serde_json::to_vec_pretty(&json!({"hooks": {"PreToolUse": [{
                "matcher": "*",
                "hooks": [{"type": "command", "command": hook_command(&script), "timeout": HOOK_TIMEOUT}],
            }]}}))?,
        )?;
        std::fs::write(
            s.home.join(MCP_FILE),
            serde_json::to_vec_pretty(&json!({"mcpServers": {s.mcp_name: {
                "type": "http",
                "url": s.mcp_url,
                "headers": {"Authorization": format!("Bearer ${{{HOOK_TOKEN_ENV}}}")},
                // Milliseconds: the wait for one tool call (MCP_CALL_TIMEOUT).
                "timeout": MCP_CALL_TIMEOUT * 1000,
            }}}))?,
        )?;
        let token = s
            .mcp_authorization
            .strip_prefix("Bearer ")
            .unwrap_or(s.mcp_authorization);
        Ok([
            ("CLAUDE_CONFIG_DIR".to_owned(), s.home.display().to_string()),
            // The connectors of the user's claude.ai account (Gmail, Drive, Slack ...) come with the login, not the
            // configuration folder: off, so the agent has Agenium's MCP server only.
            ("ENABLE_CLAUDEAI_MCP_SERVERS".to_owned(), "false".to_owned()),
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
        let events = self.parse_own(v);
        // A subagent's messages carry the id of the `Agent` call that started it.
        match v["parent_tool_use_id"].as_str().filter(|p| !p.is_empty()) {
            Some(parent) if !events.is_empty() => vec![Event::Subagent {
                parent: parent.to_owned(),
                events,
            }],
            _ => events,
        }
    }

    fn starts_subagent(&self, tool: &str) -> bool {
        matches!(tool, "Agent" | "Task")
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
            return Check::Deny(
                "use the `fetch` tool from the \"agenium\" MCP server instead (allowlisted and logged); WebFetch/WebSearch are off".into(),
            );
        }
        if SUBAGENT_TOOLS.contains(&name) && !cx.subagents {
            return Check::Deny(NO_SUBAGENTS.into());
        }
        if !allowed_tools(cx.subagents).contains(&name) {
            return Check::Deny(own_tool_off(name));
        }
        if name == "ToolSearch" || SUBAGENT_TOOLS.contains(&name) {
            return Check::Allow;
        }
        // Its own file and search tools: its scratch folder only.
        let paths: Vec<&str> = ["file_path", "path", "notebook_path"]
            .iter()
            .filter_map(|k| input[*k].as_str())
            .collect();
        if paths.is_empty() {
            return Check::Deny(own_tool_off(name));
        }
        for p in paths {
            if let Err(r) = file_access(Path::new(p), cx, &protected(cx.home)) {
                return Check::Deny(r);
            }
        }
        Check::Allow
    }

    fn hook_answer(&self, allow: bool, reason: &str) -> Value {
        json!({"hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": if allow { "allow" } else { "deny" },
            "permissionDecisionReason": reason,
        }})
    }

    fn hook_with_context(&self, mut answer: Value, text: &str) -> Option<Value> {
        answer["hookSpecificOutput"]["additionalContext"] = Value::String(text.to_owned());
        Some(answer)
    }
}

/// Claude's subscription usage, read on the user's request: one tiny Haiku turn without tools (it costs a little
/// quota) whose stream reports `rate_limit_event`. It runs with a scratch `CLAUDE_CONFIG_DIR` holding a copy of the
/// login, like a run, and hands a refreshed login back the same way.
pub async fn rate_limits(
    program: &Path,
    scratch: &Path,
) -> Result<agenium_llm::vendor_usage::VendorUsage, String> {
    use tokio::io::{AsyncBufReadExt, BufReader};
    let user = user_claude_dir().ok_or("cannot find the user's Claude Code folder")?;
    // Deleted with the login copy when this function returns, whichever way.
    let dir = probe_dir(scratch, "claude-home")?;
    let home = dir.path().join("claude-home");
    let cwd = dir.path().join("cwd");
    for d in [&home, &cwd] {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    copy_login(&user.join(LOGIN), &home.join(LOGIN)).map_err(|e| e.to_string())?;
    let mut cmd = tokio::process::Command::new(program);
    cmd.args([
        "-p",
        "Reply with the single word ok.",
        "--model",
        "haiku",
        "--output-format",
        "stream-json",
        "--verbose",
        "--tools",
        "",
        "--strict-mcp-config",
        "--no-session-persistence",
        "--setting-sources",
        "user",
    ])
    .env("CLAUDE_CONFIG_DIR", &home)
    .current_dir(&cwd)
    .stdin(std::process::Stdio::null())
    .stdout(std::process::Stdio::piped())
    .stderr(std::process::Stdio::null())
    .kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("cannot start claude: {e}"))?;
    let stdout = child.stdout.take().ok_or("claude stdout missing")?;
    let read = async {
        let mut lines = BufReader::new(stdout).lines();
        let mut found = None;
        let mut error = None;
        while let Some(line) = lines.next_line().await.map_err(|e| e.to_string())? {
            let Ok(v) = serde_json::from_str::<Value>(line.trim()) else {
                continue;
            };
            match v["type"].as_str() {
                Some("rate_limit_event") => {
                    found =
                        agenium_llm::vendor_usage::from_claude_event(&v, &agenium_core::now_iso())
                }
                Some("result") if v["is_error"] == true => {
                    error = Some(v["result"].as_str().unwrap_or("claude failed").to_owned())
                }
                _ => {}
            }
        }
        found.ok_or_else(|| error.unwrap_or_else(|| "claude reported no rate limits".into()))
    };
    let out = tokio::time::timeout(std::time::Duration::from_secs(120), read)
        .await
        .unwrap_or_else(|_| Err("claude did not answer in 120 s".into()));
    let _ = child.kill().await;
    if let Err(e) = hand_back_login(&user.join(LOGIN), &home.join(LOGIN)) {
        tracing::warn!(error = %e, "claude login not handed back");
    }
    out
}

/// Models Claude Code accepts with `--model`: its aliases (the newest of each family) and full model ids. The CLI
/// has no command that lists them.
pub const MODELS: [&str; 7] = [
    "haiku",
    "sonnet",
    "opus",
    "claude-haiku-5-5",
    "claude-sonnet-5-5",
    "claude-opus-5-5",
    "claude-fable-5-1",
];

#[cfg(test)]
mod tests {
    use super::*;

    /// Lines captured from claude 2.1.310 (shortened).
    /// ADR-129: a subagent's messages carry the id of the `Agent` call that started it; its steps come wrapped with it.
    #[test]
    fn a_subagents_steps_are_wrapped_with_the_call_that_started_it() {
        let c = Claude;
        let ev = |s: &str| c.parse(&serde_json::from_str(s).unwrap());
        assert_eq!(
            ev(
                r#"{"type":"assistant","parent_tool_use_id":"toolu_A","message":{"content":[{"type":"tool_use","id":"toolu_2","name":"mcp__agenium__grep","input":{"pattern":"x"}}]}}"#
            ),
            vec![Event::Subagent {
                parent: "toolu_A".into(),
                events: vec![Event::Tool {
                    id: "toolu_2".into(),
                    name: "mcp__agenium__grep".into(),
                    input: json!({"pattern": "x"}),
                    done: false
                }]
            }]
        );
        // The main agent's messages (no parent, or a null one) are not wrapped.
        assert!(matches!(
            ev(r#"{"type":"assistant","parent_tool_use_id":null,"message":{"content":[{"type":"text","text":"hi"}]}}"#)
                .as_slice(),
            [Event::Text(_)]
        ));
        assert!(
            c.starts_subagent("Agent") && c.starts_subagent("Task") && !c.starts_subagent("Read")
        );
    }

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
        // The subscription usage the CLI reports on its own (claude 2.1.310).
        match &ev(
            r#"{"type":"rate_limit_event","rate_limit_info":{"status":"allowed","resetsAt":1792903200,"rateLimitType":"five_hour","unifiedWindows":{"five_hour":{"utilization":0.09,"resetsAt":1792903200,"status":"allowed"},"seven_day":{"utilization":0.18,"resetsAt":1793224800,"status":"allowed"}}}}"#,
        )[0]
        {
            Event::Limits(u) => {
                assert_eq!(u.vendor, "claude");
                assert_eq!(u.windows.len(), 2);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn checks_tool_calls_and_answers_like_claude_code() {
        let wt = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let cx = CheckCtx {
            worktree: wt.path(),
            home: home.path(),
            mcp_name: "agenium",
            subagents: false,
            read_only: false,
        };
        // Allowlist: subagents only for Solo; scheduling, notifications, plugins, worktrees... never.
        let task = json!({"tool_name": "Task", "tool_input": {"prompt": "x"}});
        assert_eq!(Claude.check(&task, &cx), Check::Deny(NO_SUBAGENTS.into()));
        // Its current name (Claude Code 2.1.312, a Solo run on 2026-10-28 was refused under it).
        let agent = json!({"tool_name": "Agent", "tool_input": {"prompt": "x"}});
        assert_eq!(Claude.check(&agent, &cx), Check::Deny(NO_SUBAGENTS.into()));
        let solo = CheckCtx {
            subagents: true,
            read_only: false,
            ..cx
        };
        assert_eq!(Claude.check(&task, &solo), Check::Allow);
        assert_eq!(Claude.check(&agent, &solo), Check::Allow);
        for name in [
            "CronCreate",
            "RemoteTrigger",
            "PushNotification",
            "PublishPlugin",
            "EnterWorktree",
            "Workflow",
            "Skill",
        ] {
            let c = Claude.check(&json!({"tool_name": name, "tool_input": {}}), &solo);
            assert!(matches!(c, Check::Deny(_)), "{name}: {c:?}");
        }
        let call = |name: &str, input: Value| {
            Claude.check(&json!({"tool_name": name, "tool_input": input}), &cx)
        };
        // ADR-129: its shells are off; its own file and search tools do not reach the worktree (the refusal names
        // Agenium's tools), nor act on it without a path.
        for name in ["Bash", "PowerShell", "NotebookEdit"] {
            assert_eq!(
                call(name, json!({"command": "cargo test"})),
                Check::Deny(own_tool_off(name))
            );
        }
        assert!(own_tool_off("Bash").contains("shell for every command"));
        for name in ["Write", "Read", "Edit", "Glob", "Grep"] {
            assert!(
                matches!(call(name, json!({"file_path": wt.path().join("a.txt")})), Check::Deny(r) if r.contains("read_file")),
                "{name}"
            );
            assert_eq!(
                call(name, json!({"pattern": "x"})),
                Check::Deny(own_tool_off(name))
            );
        }
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
        assert_eq!(call("mcp__agenium__submit_report", json!({})), Check::Allow);
        assert!(matches!(
            call("mcp__github__create_issue", json!({})),
            Check::Deny(_)
        ));
        assert!(matches!(
            call("WebFetch", json!({"url": "https://x"})),
            Check::Deny(_)
        ));
        assert_eq!(
            call("ToolSearch", json!({"query": "agenium"})),
            Check::Allow
        );
        assert!(
            matches!(call("TodoWrite", json!({"todos": []})), Check::Deny(_)),
            "not in the allowlist"
        );
        assert_eq!(
            Claude.hook_answer(false, "no")["hookSpecificOutput"]["permissionDecision"],
            "deny"
        );
    }

    /// Review of 2026-10-27: the per-run home was open to the file tools, the hook script included (rewritten to
    /// `exit 0`, every later shell call skips the gate), and a write into the worktree's `.git` (a hook runs on the host
    /// at the next git call) was allowed.
    #[test]
    fn agenium_files_in_the_home_and_dot_git_are_out_of_reach() {
        let wt = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(wt.path().join(".git").join("hooks")).unwrap();
        let cx = CheckCtx {
            worktree: wt.path(),
            home: home.path(),
            mcp_name: "agenium",
            subagents: false,
            read_only: false,
        };
        let call = |name: &str, key: &str, p: &Path| {
            Claude.check(&json!({"tool_name": name, "tool_input": {key: p}}), &cx)
        };
        let (h, git) = (home.path(), wt.path().join(".git"));
        for (name, key, p) in [
            ("Write", "file_path", h.join(HOOK_FILE)),
            ("Edit", "file_path", h.join("settings.json")),
            ("Write", "file_path", h.join(MCP_FILE)),
            ("Read", "file_path", h.join(LOGIN)),
            ("Grep", "path", h.to_path_buf()),
            ("Write", "file_path", git.join("hooks").join("post-commit")),
            ("Edit", "file_path", wt.path().join("sub/.GIT/config")),
        ] {
            assert!(
                matches!(call(name, key, &p), Check::Deny(_)),
                "{name} {}",
                p.display()
            );
        }
        assert!(matches!(
            call("Read", "file_path", &git.join("HEAD")),
            Check::Deny(_)
        ));
        for (name, key, p) in [
            ("Write", "file_path", h.join("projects").join("notes.md")),
            ("Read", "file_path", h.join("projects/p/tool-results/1.txt")),
            ("Grep", "path", h.join("projects")),
        ] {
            assert_eq!(call(name, key, &p), Check::Allow, "{name} {}", p.display());
        }
    }

    fn files_below(dir: &Path) -> Vec<PathBuf> {
        let mut out = vec![];
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            if e.path().is_dir() {
                out.extend(files_below(&e.path()));
            } else {
                out.push(e.path());
            }
        }
        out
    }

    /// Review of 2026-10-27: the usage probe left its copy of the user's login (an OAuth refresh token) in the
    /// persistent scratch folder; the hook's call to Agenium had no deadline under the CLI's hook timeout, past which
    /// Claude Code goes on with the tool.
    #[tokio::test]
    async fn no_login_stays_behind_and_the_hook_ends_before_the_cli_gives_up() {
        let user = tempfile::tempdir().unwrap();
        std::fs::write(user.path().join(LOGIN), "{\"refresh\":\"r\"}").unwrap();
        let scratch = tempfile::tempdir().unwrap();
        let legacy = scratch.path().join("claude-home");
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join(LOGIN), "left by an earlier version").unwrap();
        let home = tempfile::tempdir().unwrap();
        std::env::set_var("CLAUDE_CONFIG_DIR", user.path());
        let probe = rate_limits(&scratch.path().join("no-such-claude"), scratch.path()).await;
        let env = Claude.prepare(&Setup {
            home: home.path(),
            worktree: Path::new("."),
            mcp_name: "agenium",
            mcp_url: "http://127.0.0.1:9/mcp",
            mcp_authorization: "Bearer t",
            hook_url: "http://127.0.0.1:9/hook",
            subagents: false,
            model: "",
        });
        std::env::remove_var("CLAUDE_CONFIG_DIR");
        assert!(probe.is_err());
        let left = files_below(scratch.path());
        assert!(left.is_empty(), "{left:?}");
        env.unwrap();
        let script = std::fs::read_to_string(home.path().join(HOOK_FILE)).unwrap();
        assert!(
            script.contains("--max-time 800") && script.contains("exit 2"),
            "{script}"
        );
        let settings: Value =
            serde_json::from_slice(&std::fs::read(home.path().join("settings.json")).unwrap())
                .unwrap();
        assert_eq!(
            settings["hooks"]["PreToolUse"][0]["hooks"][0]["timeout"],
            900
        );
        assert!(crate::driver::hook_deadline(&Claude) < std::time::Duration::from_secs(800));
    }

    #[test]
    fn the_hook_command_quotes_a_path_with_a_space_or_a_quote() {
        assert_eq!(
            hook_command(Path::new("/tmp/John Smith/agenium-hook.sh")),
            "sh '/tmp/John Smith/agenium-hook.sh'"
        );
        assert_eq!(
            hook_command(Path::new("/tmp/O'Brien/h.sh")),
            r"sh '/tmp/O'\''Brien/h.sh'"
        );
    }

    #[test]
    fn args_point_at_the_per_run_home() {
        let a = Claude.args("haiku", Some(&Effort::LOW), Path::new("/h"), None, false);
        let at = a.iter().position(|x| x == "--effort").unwrap();
        assert_eq!(a[at + 1], "low");
        let o = Claude.args(
            "claude-opus-4-5",
            Some(&Effort::MAX),
            Path::new("/h"),
            None,
            false,
        );
        let at = o.iter().position(|x| x == "--effort").unwrap();
        assert_eq!(o[at + 1], "high", "Opus 4.5 ends at high");
        assert!(!Claude
            .args("haiku", None, Path::new("/h"), None, false)
            .contains(&"--effort".to_owned()));
        assert!(a.contains(&"--strict-mcp-config".to_owned()));
        assert!(a.contains(&"--disable-slash-commands".to_owned()));
        let i = a.iter().position(|x| x == "--tools").unwrap();
        assert_eq!(a[i + 1], "ToolSearch,Read,Write,Edit,Glob,Grep");
        let s = Claude.args("haiku", None, Path::new("/h"), None, true);
        let i = s.iter().position(|x| x == "--tools").unwrap();
        assert!(s[i + 1].ends_with(",Agent,Task,TaskOutput,SendMessage,ListAgents,TaskStop"));
        let i = a.iter().position(|x| x == "--setting-sources").unwrap();
        assert_eq!(a[i + 1], "user");
        let i = a.iter().position(|x| x == "--mcp-config").unwrap();
        assert!(a[i + 1].ends_with(MCP_FILE));
    }
}
