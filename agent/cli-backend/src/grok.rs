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
use std::path::{Component, Path, PathBuf};

use agenium_core::Effort;
use serde_json::Value;

use crate::driver::{
    file_access, hook_call_secs, named_paths, original, own_tool_off, Check, CheckCtx, Control,
    Driver, Event, Setup, Turns, NO_SUBAGENTS,
};
use crate::driver::{toml_str, HOOK_TOKEN_ENV, HOOK_URL_ENV, MCP_CALL_TIMEOUT};
use crate::login_access::{lend_login, promote, sync_lent};

pub struct Grok;

/// Grok tools an Agenium run may use (ADR-129): the MCP dispatchers, which reach the Agenium tools that do the work,
/// and its own file and search tools for its scratch folder only (the per-run home: its session notes); the hook
/// refuses them on the worktree. Its shell and command tools and its to-do list are not offered. Scheduling, monitors,
/// workflows, plan mode, questions to a user who is not there, feedback, image and video generation and web search
/// are withheld too.
const BASE_TOOLS: [&str; 7] = [
    "search_tool",
    "use_tool",
    "read_file",
    "write",
    "search_replace",
    "list_dir",
    "grep",
];
/// Subagents (Solo only): starting one, reading what it returned, ending it (its own commands are off, so the last two
/// reach only subagents). Its `workflow` script, which grok 1.0.41 reaches for to run several in parallel, stays
/// withheld: it runs on after the turn, and the process of a turn ends with it (a Solo agent that ran one on 2026-10-28
/// answered "I will give you both answers when they finish" and nothing came).
const SUBAGENT_TOOLS: [&str; 3] = [
    "spawn_subagent",
    "get_command_or_subagent_output",
    "kill_command_or_subagent",
];

fn allowed_tools(subagents: bool) -> Vec<&'static str> {
    let mut v = BASE_TOOLS.to_vec();
    if subagents {
        v.extend(SUBAGENT_TOOLS);
    }
    v
}

const LOGIN: &str = "auth.json";

/// Grok's login lent to a run (ADR-137): each of its sign-ins (keyed by issuer and account) without its
/// `refresh_token`, and when the first of their access tokens expires (`expires_at`, RFC 3339).
fn lend(login: &Value) -> Option<(Value, std::time::SystemTime)> {
    let mut out = login.as_object()?.clone();
    let mut first: Option<std::time::SystemTime> = None;
    for entry in out.values_mut() {
        let e = entry.as_object_mut()?;
        let at = chrono::DateTime::parse_from_rfc3339(e.get("expires_at")?.as_str()?).ok()?;
        let at = std::time::SystemTime::from(at);
        first = Some(first.map_or(at, |f| f.min(at)));
        e.remove("refresh_token");
    }
    Some((Value::Object(out), first?))
}
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

/// Agenium's files in the per-run home, out of reach of the model's file tools (`driver::file_access`): the hook
/// script, `config.toml` (read again by every turn's process) and the login copy.
fn protected(home: &Path) -> Vec<PathBuf> {
    let login = home.join(LOGIN);
    vec![
        hook(home).0,
        home.join("config.toml"),
        original(&login),
        login,
    ]
}

/// The hook script and the command that runs it (PowerShell on Windows, where Grok runs hooks through it; `sh`
/// elsewhere).
fn hook(home: &Path) -> (PathBuf, String, String) {
    let t = hook_call_secs(HOOK_TIMEOUT);
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

/// The models this Grok login can use (`grok models`; no model turn). A CLI that hangs is given up after 40 s.
pub async fn models(program: &Path) -> Result<Vec<String>, String> {
    let mut cmd = tokio::process::Command::new(program);
    cmd.arg("models")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    let out = tokio::time::timeout(std::time::Duration::from_secs(40), cmd.output())
        .await
        .map_err(|_| "grok models did not answer in 40 s".to_owned())?
        .map_err(|e| format!("cannot start grok: {e}"))?;
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

    fn hook_timeout(&self) -> u64 {
        HOOK_TIMEOUT
    }

    fn prompt_file(&self, home: &Path) -> Option<PathBuf> {
        Some(home.join(TURN_FILE))
    }

    fn args(
        &self,
        model: &str,
        effort: Option<&Effort>,
        home: &Path,
        resume: Option<&str>,
        subagents: bool,
    ) -> Vec<String> {
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
        // The CLI runs xAI's models: their own level (ADR-114). Its `--reasoning-effort` names no levels, so a model
        // the table does not know keeps the CLI's setting.
        if let Some(l) = effort.and_then(|e| agenium_llm::effort::level_for(model, e)) {
            a.extend(["--reasoning-effort".into(), l.as_str().into()]);
        }
        if let Some(id) = resume {
            a.extend(["--resume".into(), id.into()]);
        }
        a
    }

    fn prepare(&self, s: &Setup) -> std::io::Result<BTreeMap<String, String>> {
        let user = user_grok_dir()
            .ok_or_else(|| std::io::Error::other("cannot find the user's Grok folder"))?;
        // Lent without its refresh token (ADR-137), not linked like Codex's (ADR-136): Grok writes `auth.json.tmp` and
        // renames it over its login, which would part a hard link at the first refresh.
        lend_login(&user.join(LOGIN), &s.home.join(LOGIN), lend, s.run_for)?;
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
            "[mcp_servers.{name}]\nurl = {url}\nheaders = {{ Authorization = {auth} }}\ntool_timeout_sec = {MCP_CALL_TIMEOUT}\n\n",
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

    /// Grok makes an empty `/tmp/sessions/<id>` for every session; on Windows `/tmp` is on the worktree's drive, so
    /// runs left `D:\tmp\sessions` with 104 of them (2026-10-30).
    fn session_leftovers(&self, worktree: &Path, id: &str) -> Vec<PathBuf> {
        let root: PathBuf = worktree
            .components()
            .take_while(|c| matches!(c, Component::Prefix(_) | Component::RootDir))
            .collect();
        let sessions = root.join("tmp").join("sessions");
        vec![sessions.join(id), sessions]
    }

    fn resumes_across_runs(&self) -> bool {
        true
    }

    fn login_file(&self) -> Option<&'static str> {
        Some(LOGIN)
    }

    fn finish(&self, s: &Setup) -> std::io::Result<()> {
        match user_grok_dir() {
            Some(user) => sync_lent(&user.join(LOGIN), &s.home.join(LOGIN), lend),
            None => Ok(()),
        }
    }

    fn promote_login(&self, s: &Setup) -> bool {
        user_grok_dir().is_some_and(|u| promote(&u.join(LOGIN), &s.home.join(LOGIN), lend))
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
                && ([
                    &input["server"],
                    &input["server_name"],
                    &input["serverName"],
                ]
                .iter()
                .any(|v| v.as_str() == Some(server))
                    // grok 1.0.41's stream names only the qualified tool (`agenium__grep`): without this, every
                    // Agenium call it made was logged twice (as the call and as `cli:use_tool`).
                    || [&input["tool_name"], &input["toolName"]]
                        .iter()
                        .filter_map(|v| v.as_str())
                        .any(|t| t.starts_with(&format!("{server}__")))))
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
            return Check::Deny(own_tool_off(name));
        }
        if name == "search_tool" || SUBAGENT_TOOLS.contains(&name) {
            return Check::Allow;
        }
        // Its own file and search tools: its scratch folder only, whatever argument names a path (only eight known
        // names were read).
        let paths = match named_paths(input) {
            Ok(p) => p,
            Err(r) => return Check::Deny(r),
        };
        if paths.is_empty() {
            return Check::Deny(own_tool_off(name));
        }
        for p in paths {
            if let Err(r) = file_access(Path::new(&p), cx, &protected(cx.home)) {
                return Check::Deny(r);
            }
        }
        Check::Allow
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
            read_only: false,
        };
        let call = |name: &str, input: Value| json!({"hookEventName": "pre_tool_use", "toolName": name, "toolInput": input});
        // ADR-129: its own shell, file, search and to-do tools are off, even inside the worktree.
        let inside_file = wt.path().join("src").join("a.rs");
        for name in ["run_terminal_command", "todo_write"] {
            assert_eq!(
                Grok.check(&call(name, json!({"command": "cargo test"})), &cx),
                Check::Deny(own_tool_off(name))
            );
        }
        for name in ["search_replace", "read_file", "write", "list_dir", "grep"] {
            let c = Grok.check(
                &call(
                    name,
                    json!({"file_path": inside_file.display().to_string()}),
                ),
                &cx,
            );
            assert!(
                matches!(&c, Check::Deny(r) if r.contains("read_file")),
                "{name}: {c:?}"
            );
            assert_eq!(
                Grok.check(&call(name, json!({"pattern": "x"})), &cx),
                Check::Deny(own_tool_off(name))
            );
        }
        assert!(matches!(
            Grok.check(&call("write", json!({"path": "../../escape.rs"})), &cx),
            Check::Deny(_)
        ));
        assert_eq!(
            Grok.check(&call("agenium__submit_report", json!({})), &cx),
            Check::Allow
        );
        // Its dispatcher's call of an Agenium tool, as the stream names it, is Agenium's (logged once).
        assert!(Grok.is_agenium_call(
            "use_tool",
            &json!({"tool_name": "agenium__grep", "tool_input": {"pattern": "x"}}),
            "agenium"
        ));
        assert!(!Grok.is_agenium_call(
            "use_tool",
            &json!({"tool_name": "linear__save_issue"}),
            "agenium"
        ));
        assert!(matches!(
            Grok.check(&call("linear__save_issue", json!({})), &cx),
            Check::Deny(_)
        ));
        assert_eq!(
            Grok.check(&call("spawn_subagent", json!({})), &cx),
            Check::Deny(NO_SUBAGENTS.into())
        );
        // A Solo agent may run them; the workflow script stays withheld (it outlives the turn).
        let solo = CheckCtx {
            subagents: true,
            ..cx
        };
        assert_eq!(
            Grok.check(&call("spawn_subagent", json!({})), &solo),
            Check::Allow
        );
        assert!(matches!(
            Grok.check(&call("workflow", json!({})), &solo),
            Check::Deny(_)
        ));
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
        let a = Grok.args("grok-4.7", Some(&Effort::MAX), Path::new("/h"), None, false);
        assert!(
            a.windows(2).any(|w| w == ["--reasoning-effort", "xhigh"]),
            "grok-4.7 ends at xhigh: {a:?}"
        );
        assert!(!Grok
            .args(
                "grok-4.7-build-fast",
                Some(&Effort::MAX),
                Path::new("/h"),
                None,
                false
            )
            .contains(&"--reasoning-effort".to_owned()));
        assert!(a
            .windows(2)
            .any(|w| w == ["--permission-mode", "bypassPermissions"]));
        assert!(
            a.contains(&"--no-subagents".to_owned())
                && a.contains(&"--disable-web-search".to_owned())
        );
        assert!(!a.iter().any(|x| x.contains("spawn_subagent")));
        let r = Grok.args("", None, Path::new("/h"), Some("s1"), true);
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
                run_for: std::time::Duration::from_secs(2700),
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

    /// Review of 2026-10-27: `config.toml` (read again by every turn's process) and the hook script in the per-run
    /// home, and the worktree's `.git`, were writable by the model's file tools.
    #[test]
    fn agenium_files_and_dot_git_are_out_of_reach() {
        let wt = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let cx = CheckCtx {
            worktree: wt.path(),
            home: home.path(),
            mcp_name: "agenium",
            subagents: false,
            read_only: false,
        };
        let call = |name: &str, p: PathBuf| {
            Grok.check(
                &json!({"toolName": name, "toolInput": {"file_path": p}}),
                &cx,
            )
        };
        for (name, p) in [
            ("write", home.path().join("config.toml")),
            ("search_replace", hook(home.path()).0),
            ("read_file", home.path().join(LOGIN)),
            ("write", wt.path().join(".git/hooks/pre-push")),
        ] {
            assert!(
                matches!(call(name, p.clone()), Check::Deny(_)),
                "{name} {}",
                p.display()
            );
        }
        assert!(matches!(
            call("read_file", wt.path().join(".git/HEAD")),
            Check::Deny(_)
        ));
        assert_eq!(
            call("write", home.path().join("sessions/notes.md")),
            Check::Allow
        );
        assert!(crate::driver::hook_deadline(&Grok) < std::time::Duration::from_secs(3500));
    }

    #[test]
    fn models_listing() {
        let out = "You are not authenticated.\n\nDefault model: grok-4.7\n\nAvailable models:\n  * grok-4.7 (default)\n  - grok-4.7-build-fast\n  - grok-4.6\n";
        assert_eq!(
            parse_models(out),
            ["grok-4.7", "grok-4.7-build-fast", "grok-4.6"]
        );
    }

    #[test]
    fn the_session_folder_at_the_drive_root_is_named() {
        #[cfg(windows)]
        let (wt, root) = (r"D:\Work\proj\.agenium\wt\t1", r"D:\");
        #[cfg(not(windows))]
        let (wt, root) = ("/home/u/proj/.agenium/wt/t1", "/");
        let sessions = Path::new(root).join("tmp").join("sessions");
        assert_eq!(
            Grok.session_leftovers(Path::new(wt), "01a1-s"),
            [sessions.join("01a1-s"), sessions]
        );
    }
}
