//! Grok Build CLI (`grok`) driver, checked with grok 1.0.41 and grok-4.7 on 2026-10-26 (ADR-108).
//!
//! - Headless: one process per turn, `grok --prompt-file <file> --output-format streaming-messages-json`; the prompt
//!   cannot come on stdin (headless mode does not read it), so each turn's message is written to a file in the per-run
//!   home. Later turns add `--resume <session id>`. The stream is the Anthropic Messages `stream-json` shape that
//!   Claude Code also writes (`system`/`init`, `assistant`, `user` tool results, `result` with usage).
//! - Isolation: `GROK_HOME` = the per-run home with Agenium's `config.toml` (its MCP server and its hook only; Claude,
//!   Codex and Cursor compatibility scanning off; no memory; no shared leader process) and a copy of the user's
//!   login (`auth.json`; a login refreshed during the run is handed back). The user's `~/.grok` config, hooks, MCP
//!   servers, skills and memory are not used.
//! - Control: always-approve (`--permission-mode bypassPermissions`) with a `PreToolUse` hook on every tool that asks
//!   Agenium, and only the tools Agenium allows are offered (`--tools`). Grok's hooks fail open on a timeout or a
//!   crash, so the hook's own HTTP call has the shorter deadline, and whenever Agenium cannot be reached the hook
//!   writes `deny` and exits 2: it fails closed.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::driver::{
    copy_login, hand_back_login, inside, Check, CheckCtx, Control, Driver, Event, Setup, Turns,
    NO_SUBAGENTS,
};

pub struct Grok;

/// Grok tools an Agenium run may use: shell, files, search, the MCP dispatchers (Agenium's protocol tools are MCP
/// tools) and its to-do list. Scheduling, monitors, workflows, plan mode, questions to a user who is not there,
/// feedback, image and video generation and web search are withheld.
const BASE_TOOLS: [&str; 11] = [
    "run_terminal_command",
    "get_command_or_subagent_output",
    "kill_command_or_subagent",
    "read_file",
    "write",
    "search_replace",
    "list_dir",
    "grep",
    "todo_write",
    "search_tool",
    "use_tool",
];
/// Subagents (Solo only).
const SUBAGENT_TOOLS: [&str; 1] = ["spawn_subagent"];

fn allowed_tools(subagents: bool) -> Vec<&'static str> {
    let mut v = BASE_TOOLS.to_vec();
    if subagents {
        v.extend(SUBAGENT_TOOLS);
    }
    v
}

const HOOK_URL_ENV: &str = "AGENIUM_HOOK_URL";
const HOOK_TOKEN_ENV: &str = "AGENIUM_HOOK_TOKEN";
const LOGIN: &str = "auth.json";
const TURN_FILE: &str = "agenium-turn.txt";
/// Seconds Grok waits for the hook; the hook's own call to Agenium gives up 100 s earlier, so a slow approval ends
/// in Agenium's answer or the hook's deny, never in Grok's fail-open timeout.
const HOOK_TIMEOUT: u64 = 3600;
const DENY: &str = r#"{"decision":"deny","reason":"Agenium could not check this tool call"}"#;

/// The user's own Grok folder (`GROK_HOME` or `~/.grok`).
pub fn user_grok_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("GROK_HOME") {
        return Some(PathBuf::from(d));
    }
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(|h| PathBuf::from(h).join(".grok"))
}

/// A TOML basic string.
fn toml_str(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_default()
}

/// The hook script and the command that runs it (PowerShell on Windows, where Grok runs hooks through it; `sh`
/// elsewhere).
fn hook(home: &Path) -> (PathBuf, String, String) {
    let t = HOOK_TIMEOUT - 100;
    if cfg!(windows) {
        let script = home.join("agenium-hook.ps1");
        let body = format!(
            "$ErrorActionPreference = 'Stop'\r\n\
             [Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false)\r\n\
             try {{\r\n\
             \x20 $body = [Console]::In.ReadToEnd()\r\n\
             \x20 $r = Invoke-WebRequest -UseBasicParsing -Method Post -Uri $env:{HOOK_URL_ENV} \
             -Headers @{{ Authorization = \"Bearer $env:{HOOK_TOKEN_ENV}\" }} \
             -ContentType 'application/json; charset=utf-8' -Body ([Text.Encoding]::UTF8.GetBytes($body)) -TimeoutSec {t}\r\n\
             \x20 [Console]::Out.Write($r.Content)\r\n\
             }} catch {{\r\n\
             \x20 [Console]::Out.Write('{DENY}')\r\n\
             \x20 [Console]::Error.WriteLine(\"agenium hook: $($_.Exception.Message)\")\r\n\
             \x20 exit 2\r\n\
             }}\r\n"
        );
        let cmd = format!(
            "powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -File \"{}\"",
            script.display()
        );
        (script, body, cmd)
    } else {
        let script = home.join("agenium-hook.sh");
        let body = format!(
            "#!/bin/sh\nout=$(curl -s -S --fail-with-body --max-time {t} -X POST -H 'Content-Type: application/json' \
             -H \"Authorization: Bearer ${HOOK_TOKEN_ENV}\" --data-binary @- \"${HOOK_URL_ENV}\") || \
             {{ printf '%s' '{DENY}'; exit 2; }}\nprintf '%s' \"$out\"\n"
        );
        let cmd = format!("sh '{}'", script.display());
        (script, body, cmd)
    }
}

/// Grok's model list (`grok models`): lines `  * grok-4.7 (default)` and `  - grok-4.6`.
pub fn parse_models(out: &str) -> Vec<String> {
    out.lines()
        .filter_map(|l| {
            let l = l.trim();
            let rest = l.strip_prefix("* ").or_else(|| l.strip_prefix("- "))?;
            rest.split_whitespace().next().map(str::to_owned)
        })
        .filter(|m| m.starts_with("grok"))
        .collect()
}

pub async fn models(program: &Path) -> Result<Vec<String>, String> {
    let out = tokio::process::Command::new(program)
        .arg("models")
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .map_err(|e| e.to_string())?;
    let m = parse_models(&String::from_utf8_lossy(&out.stdout));
    if m.is_empty() {
        Err("grok models listed nothing".into())
    } else {
        Ok(m)
    }
}

impl Driver for Grok {
    fn control(&self) -> Control {
        Control::Hook
    }

    fn turns(&self) -> Turns {
        Turns::ProcessPerTurn
    }

    fn prompt_file(&self, home: &Path) -> Option<PathBuf> {
        Some(home.join(TURN_FILE))
    }

    fn args(&self, model: &str, home: &Path, resume: Option<&str>, subagents: bool) -> Vec<String> {
        let mut a: Vec<String> = vec![
            "--prompt-file".into(),
            home.join(TURN_FILE).display().to_string(),
            "--output-format".into(),
            "streaming-messages-json".into(),
            "--permission-mode".into(),
            "bypassPermissions".into(),
            "--disable-web-search".into(),
            "--no-plan".into(),
            // Only the tools Agenium allows are offered at all (the hook enforces the same list).
            "--tools".into(),
            allowed_tools(subagents).join(","),
        ];
        if !subagents {
            a.push("--no-subagents".into());
        }
        if !model.is_empty() {
            a.extend(["--model".into(), model.into()]);
        }
        if let Some(id) = resume {
            a.extend(["--resume".into(), id.into()]);
        }
        a
    }

    fn prepare(&self, s: &Setup) -> std::io::Result<BTreeMap<String, String>> {
        let user = user_grok_dir()
            .ok_or_else(|| std::io::Error::other("cannot find the user's Grok folder"))?;
        copy_login(&user.join(LOGIN), &s.home.join(LOGIN))?;
        let (script, body, command) = hook(s.home);
        std::fs::write(&script, body)?;
        let mut cfg = String::new();
        cfg.push_str("[cli]\nuse_leader = false\n\n[memory]\nenabled = false\n\n");
        for (tool, keys) in [
            (
                "claude",
                &["agents", "hooks", "mcps", "rules", "skills"][..],
            ),
            ("codex", &["hooks", "skills"][..]),
            (
                "cursor",
                &["agents", "hooks", "mcps", "rules", "skills"][..],
            ),
        ] {
            cfg.push_str(&format!("[compat.{tool}]\n"));
            for k in keys {
                cfg.push_str(&format!("{k} = false\n"));
            }
            cfg.push('\n');
        }
        cfg.push_str(&format!(
            "[mcp_servers.{name}]\nurl = {url}\nheaders = {{ Authorization = {auth} }}\n\n",
            name = s.mcp_name,
            url = toml_str(s.mcp_url),
            auth = toml_str(s.mcp_authorization),
        ));
        cfg.push_str(&format!(
            "[[hooks.PreToolUse]]\nmatcher = \"\"\nhooks = [ {{ type = \"command\", command = {}, timeout = {HOOK_TIMEOUT} }} ]\n",
            toml_str(&command)
        ));
        std::fs::write(s.home.join("config.toml"), cfg)?;
        let token = s
            .mcp_authorization
            .strip_prefix("Bearer ")
            .unwrap_or(s.mcp_authorization);
        Ok([
            ("GROK_HOME".to_owned(), s.home.display().to_string()),
            ("GROK_MEMORY".to_owned(), "0".to_owned()),
            (HOOK_URL_ENV.to_owned(), s.hook_url.to_owned()),
            (HOOK_TOKEN_ENV.to_owned(), token.to_owned()),
        ]
        .into())
    }

    fn finish(&self, s: &Setup) -> std::io::Result<()> {
        match user_grok_dir() {
            Some(user) => hand_back_login(&user.join(LOGIN), &s.home.join(LOGIN)),
            None => Ok(()),
        }
    }

    fn user_input(&self, text: &str) -> String {
        text.to_owned()
    }

    fn parse(&self, v: &Value) -> Vec<Event> {
        // The same stream shape as Claude Code's.
        crate::claude::Claude
            .parse(v)
            .into_iter()
            .map(|e| match e {
                Event::TurnEnd {
                    usage,
                    response,
                    denied,
                    error,
                } => Event::TurnEnd {
                    usage,
                    response,
                    denied,
                    error: error.map(|m| m.replacen("claude turn", "grok turn", 1)),
                },
                other => other,
            })
            .collect()
    }

    fn command(&self, tool: &str, input: &Value) -> Option<String> {
        (tool == "run_terminal_command")
            .then(|| input["command"].as_str().map(str::to_owned))
            .flatten()
    }

    fn is_agenium_call(&self, tool: &str, input: &Value, server: &str) -> bool {
        tool.starts_with(&format!("{server}__"))
            || tool.starts_with(&format!("mcp__{server}__"))
            || (tool == "use_tool"
                && [
                    &input["server"],
                    &input["server_name"],
                    &input["serverName"],
                ]
                .iter()
                .any(|v| v.as_str() == Some(server)))
    }

    fn check(&self, payload: &Value, cx: &CheckCtx) -> Check {
        let name = payload["toolName"]
            .as_str()
            .or(payload["tool_name"].as_str())
            .unwrap_or("");
        let input = if payload["toolInput"].is_null() {
            &payload["tool_input"]
        } else {
            &payload["toolInput"]
        };
        // MCP calls appear under their qualified `server__tool` name.
        if name.contains("__") || name == "use_tool" {
            return if self.is_agenium_call(name, input, cx.mcp_name) {
                Check::Allow
            } else if name == "use_tool" {
                Check::Deny("only the \"agenium\" MCP server is available in Agenium runs".into())
            } else {
                Check::Deny(format!("{name} is not available in Agenium runs"))
            };
        }
        if matches!(name, "web_search" | "web_fetch") {
            return Check::Deny(
                "use the `fetch` tool from the \"agenium\" MCP server instead (allowlisted and logged)".into(),
            );
        }
        if SUBAGENT_TOOLS.contains(&name) && !cx.subagents {
            return Check::Deny(NO_SUBAGENTS.into());
        }
        if !allowed_tools(cx.subagents).contains(&name) {
            return Check::Deny(format!("{name} is not available in Agenium runs"));
        }
        for key in [
            "path",
            "file_path",
            "filePath",
            "target_file",
            "targetFile",
            "target_directory",
            "directory",
            "dir",
        ] {
            if let Some(p) = input[key].as_str() {
                let p = Path::new(p);
                if !inside(p, cx.worktree, cx.worktree) && !inside(p, cx.worktree, cx.home) {
                    return Check::Deny(format!("{} is outside the ticket worktree", p.display()));
                }
            }
        }
        match self.command(name, input) {
            Some(c) => Check::Command(c),
            None if name == "run_terminal_command" => {
                Check::Deny("the command text is missing".into())
            }
            None => Check::Allow,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn checks_tool_calls_like_the_other_hooked_clis() {
        let wt = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let cx = CheckCtx {
            worktree: wt.path(),
            home: home.path(),
            mcp_name: "agenium",
            subagents: false,
        };
        let call = |name: &str, input: Value| json!({"hookEventName": "pre_tool_use", "toolName": name, "toolInput": input});
        assert_eq!(
            Grok.check(
                &call("run_terminal_command", json!({"command": "cargo test"})),
                &cx
            ),
            Check::Command("cargo test".into())
        );
        let inside_file = wt.path().join("src").join("a.rs");
        assert_eq!(
            Grok.check(
                &call(
                    "search_replace",
                    json!({"file_path": inside_file.display().to_string()})
                ),
                &cx
            ),
            Check::Allow
        );
        assert!(matches!(
            Grok.check(&call("write", json!({"path": "../../escape.rs"})), &cx),
            Check::Deny(_)
        ));
        assert_eq!(
            Grok.check(&call("agenium__submit_report", json!({})), &cx),
            Check::Allow
        );
        assert!(matches!(
            Grok.check(&call("linear__save_issue", json!({})), &cx),
            Check::Deny(_)
        ));
        assert_eq!(
            Grok.check(&call("spawn_subagent", json!({})), &cx),
            Check::Deny(NO_SUBAGENTS.into())
        );
        for withheld in [
            "scheduler_create",
            "image_gen",
            "ask_user_question",
            "workflow",
            "web_search",
        ] {
            assert!(
                matches!(Grok.check(&call(withheld, json!({})), &cx), Check::Deny(_)),
                "{withheld}"
            );
        }
        // Unknown tools are refused (fail closed).
        assert!(matches!(
            Grok.check(&call("new_tool_2027", json!({})), &cx),
            Check::Deny(_)
        ));
    }

    #[test]
    fn args_and_isolated_home() {
        let a = Grok.args("grok-4.7", Path::new("/h"), None, false);
        assert!(a
            .windows(2)
            .any(|w| w == ["--permission-mode", "bypassPermissions"]));
        assert!(
            a.contains(&"--no-subagents".to_owned())
                && a.contains(&"--disable-web-search".to_owned())
        );
        assert!(!a.iter().any(|x| x.contains("spawn_subagent")));
        let r = Grok.args("", Path::new("/h"), Some("s1"), true);
        assert!(r.ends_with(&["--resume".to_owned(), "s1".to_owned()]));
        assert!(r.iter().any(|x| x.contains("spawn_subagent")));

        let home = tempfile::tempdir().unwrap();
        let user = tempfile::tempdir().unwrap();
        std::fs::write(user.path().join(LOGIN), "{\"token\":\"t\"}").unwrap();
        std::env::set_var("GROK_HOME", user.path());
        let env = Grok
            .prepare(&Setup {
                home: home.path(),
                worktree: Path::new("."),
                mcp_name: "agenium",
                mcp_url: "http://127.0.0.1:9/mcp",
                mcp_authorization: "Bearer tok",
                hook_url: "http://127.0.0.1:9/hook",
                subagents: false,
                model: "",
            })
            .unwrap();
        std::env::remove_var("GROK_HOME");
        assert_eq!(env["GROK_HOME"], home.path().display().to_string());
        assert_eq!(env["GROK_MEMORY"], "0");
        let cfg = std::fs::read_to_string(home.path().join("config.toml")).unwrap();
        for want in [
            "use_leader = false",
            "[compat.claude]",
            "[mcp_servers.agenium]",
            "Bearer tok",
            "[[hooks.PreToolUse]]",
            "timeout = 3600",
        ] {
            assert!(cfg.contains(want), "{want} in {cfg}");
        }
        assert!(home.path().join(LOGIN).is_file(), "the login is copied in");
        let script = std::fs::read_to_string(hook(home.path()).0).unwrap();
        assert!(
            script.contains("exit 2") && script.contains("deny"),
            "fails closed: {script}"
        );
    }

    #[test]
    fn models_listing() {
        let out = "You are not authenticated.\n\nDefault model: grok-4.7\n\nAvailable models:\n  * grok-4.7 (default)\n  - grok-4.7-build-fast\n  - grok-4.6\n";
        assert_eq!(
            parse_models(out),
            ["grok-4.7", "grok-4.7-build-fast", "grok-4.6"]
        );
    }
}
