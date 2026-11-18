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

use agenium_core::Effort;
use agenium_llm::Usage;
use serde_json::{json, Value};

use crate::driver::{
    file_access, hook_call_secs, named_paths, own_tool_off, Check, CheckCtx, Control, Driver,
    Event, Setup, Turns, NO_SUBAGENTS,
};
use crate::driver::{HOOK_TIMEOUT, HOOK_TOKEN_ENV, HOOK_URL_ENV};

/// Name fragments of agy 1.4 tools an Agenium run never uses (matched on the lowercase tool name, since the hook and
/// the init event spell some tools differently).
const WITHHELD: [&str; 9] = [
    "schedule",
    "send_message",
    "inbox",
    "marketplace",
    "plugin",
    "generate_image",
    "knowledge",
    "workflow",
    "notebook_execution",
];

pub struct Agy;

fn write_json(path: &Path, v: &Value) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, serde_json::to_vec_pretty(v)?)
}

/// The hook: a script in the per-run config folder (the hook's working directory) that posts the hook payload
/// (stdin) to Agenium and prints Agenium's answer; `--fail-with-body` makes a refused or failed request fail the
/// hook, which blocks the tool, and `--max-time` ends the call before agy's own hook timeout (a hook killed by the
/// CLI must not let the tool through). A script, because agy's own `cmd /c` call mangles quoted arguments. Returns the
/// hook command and the script (file name, content).
fn hook_script() -> (&'static str, &'static str, String) {
    let t = hook_call_secs(HOOK_TIMEOUT);
    if cfg!(windows) {
        (
            "agenium-hook.cmd",
            "agenium-hook.cmd",
            format!(
                "@curl.exe -s -S --fail-with-body --max-time {t} -X POST -H \"Content-Type: application/json\" -H \"Authorization: Bearer %{HOOK_TOKEN_ENV}%\" --data-binary @- \"%{HOOK_URL_ENV}%\"\r\n"
            ),
        )
    } else {
        (
            "sh ./agenium-hook.sh",
            "agenium-hook.sh",
            format!(
                "#!/bin/sh\nexec curl -s -S --fail-with-body --max-time {t} -X POST -H 'Content-Type: application/json' -H \"Authorization: Bearer ${HOOK_TOKEN_ENV}\" --data-binary @- \"${HOOK_URL_ENV}\"\n"
            ),
        )
    }
}

/// Agenium's files in the per-run home, out of reach of the model's file tools (`driver::file_access`): the config
/// folder (the hook script, `hooks.json`, `mcp_config.json`) and the workspace settings.
fn protected(home: &Path) -> Vec<std::path::PathBuf> {
    let gemini = home.join(".gemini");
    vec![
        gemini.join("config"),
        gemini.join("antigravity-cli").join("settings.json"),
    ]
}

/// agy's own command tools (name prefixes, lowercase): off in Agenium runs (ADR-129), commands go through Agenium's
/// `shell`. agy has no flag that withholds a tool, so the hook refuses them.
const COMMAND_TOOLS: [&str; 4] = [
    "run_command",
    "command_status",
    "send_command",
    "read_terminal",
];

/// agy's tools that touch neither files nor commands nor the network, allowed as they are: its task markers and its
/// notes to the user. Any other tool is refused (audit M14, 2026-10-31: anything unknown was allowed, so a tool a new
/// agy version adds, a command-like one too, ran unchecked). Seen in agy 1.4.7.
const OTHER_TOOLS: [&str; 2] = ["task_boundary", "notify_user"];

/// agy's own file and search tools (name prefixes, lowercase): its scratch folder only (the per-run home, its task
/// notes); on the worktree the work goes through Agenium's tools (ADR-129).
const FILE_TOOLS: [&str; 14] = [
    "view_",
    "list_",
    "read_",
    "find_",
    "grep",
    "search",
    "codebase",
    "write_",
    "replace_",
    "multi_replace",
    "edit_",
    "create_file",
    "delete_",
    "move_",
];

impl Driver for Agy {
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
        _home: &Path,
        resume: Option<&str>,
        _subagents: bool,
    ) -> Vec<String> {
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
        // Antigravity's models name their level (`gemini-3.8-flash-low`); `models` lists each once with its levels,
        // so the level picks the variant. A model chosen with its level in the name runs as it is.
        if !model.is_empty() {
            let model = match effort {
                Some(e) if level_of(model).is_none() => format!("{model}-{e}"),
                _ => model.to_owned(),
            };
            a.extend(["--model".to_owned(), model]);
        }
        // A ticket's conversation of an earlier run, its home brought back (ADR-134).
        if let Some(id) = resume {
            a.extend(["--conversation".to_owned(), id.to_owned()]);
        }
        // `-p` takes a value; the prompts come from stdin.
        a.push("-p=".to_owned());
        a
    }

    fn resumes_across_runs(&self) -> bool {
        true
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
                "hooks": [{"type": "command", "command": command, "timeout": HOOK_TIMEOUT}],
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
        let token = s.mcp_token();
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
                    Some("tool") => {
                        let id = s["step_index"].to_string();
                        let info = &s["tool_info"];
                        let error = s["state"] == "ERROR";
                        let text = info["output"]
                            .as_str()
                            .or_else(|| info["error"]["message"].as_str())
                            .unwrap_or("");
                        // A step that ended (done or refused) carries what it returned.
                        let output = (matches!(s["state"].as_str(), Some("DONE" | "ERROR"))
                            && (error || !text.is_empty()))
                        .then(|| Event::ToolOutput {
                            id: id.clone(),
                            text: text.to_owned(),
                            error,
                        });
                        output
                            .into_iter()
                            .chain([Event::Tool {
                                id,
                                name: s["tool_name"].as_str().unwrap_or("tool").to_owned(),
                                input: info["parameters"].clone(),
                                done: s["state"] == "DONE",
                            }])
                            .collect()
                    }
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
                        cache_write_1h: 0,
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
            return Check::Deny(
                "use the `fetch` tool from the \"agenium\" MCP server instead (allowlisted and logged); web and browser tools are off".into(),
            );
        }
        if lower.contains("subagent") && !cx.subagents {
            return Check::Deny(NO_SUBAGENTS.into());
        }
        // agy's own scheduling, messaging, marketplace, image, knowledge and workflow tools, and its interactive
        // questions (nobody answers them in a headless run: Agenium's ask_question is the way).
        if lower.starts_with("ask_") || lower.contains("permission") {
            return Check::Deny(
                "ask the Lead with the ask_question tool of the agenium MCP server instead".into(),
            );
        }
        if WITHHELD.iter().any(|w| lower.contains(w)) {
            return Check::Deny(format!("{name} is not available in Agenium runs"));
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
        // MCP resources (agy reads a server's resources by URI): Agenium's own server only; a `file:` URI is a path
        // like any other (it was let through before the path checks).
        if let Some(s) = args["ServerName"]
            .as_str()
            .filter(|s| lower.contains("resource") && *s != cx.mcp_name)
        {
            return Check::Deny(format!("MCP server {s} is not available in Agenium runs"));
        }
        if COMMAND_TOOLS.iter().any(|p| lower.starts_with(p)) {
            return Check::Deny(own_tool_off(name));
        }
        let paths = match named_paths(args) {
            Ok(p) => p,
            Err(r) => return Check::Deny(r),
        };
        // A resource of Agenium's own MCP server is read by its URI, which names no file.
        if paths.is_empty() && lower.contains("resource") {
            return Check::Allow;
        }
        // Its own file and search tools act on the worktree when no path is named.
        if paths.is_empty() && FILE_TOOLS.iter().any(|p| lower.starts_with(p)) {
            return Check::Deny(own_tool_off(name));
        }
        let known = FILE_TOOLS.iter().any(|p| lower.starts_with(p))
            || lower.contains("resource")
            || (lower.contains("subagent") && cx.subagents)
            || OTHER_TOOLS.contains(&lower.as_str());
        if !known {
            return Check::Deny(format!(
                "{name} is not a tool Agenium knows: use the tools of the \"agenium\" MCP server"
            ));
        }
        for p in paths {
            if let Err(r) = file_access(Path::new(&p), cx, &protected(cx.home)) {
                return Check::Deny(r);
            }
        }
        Check::Allow
    }
}

/// The models this Antigravity login can use (`agy models`: `id<TAB>name` lines; no model turn), one entry per model
/// with the levels its variants name.
pub async fn models(program: &Path) -> Result<Vec<agenium_llm::ModelInfo>, String> {
    let mut cmd = tokio::process::Command::new(program);
    cmd.arg("models")
        .env("AGY_CLI_DISABLE_AUTO_UPDATE", "true")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    let out = crate::probe::output(cmd, std::time::Duration::from_secs(40))
        .await
        .map_err(|e| format!("cannot start agy: {e}"))?
        .ok_or_else(|| "agy models did not answer in 40 s".to_owned())?;
    Ok(group(parse_models(&String::from_utf8_lossy(&out.stdout))))
}

fn parse_models(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| l.split_once('\t'))
        .map(|(id, _)| id.trim().to_owned())
        .filter(|id| !id.is_empty() && !id.contains(' '))
        .collect()
}

/// The level a model id names (`gemini-3.8-flash-low` -> `low`).
fn level_of(id: &str) -> Option<(&str, Effort)> {
    let (base, level) = id.rsplit_once('-')?;
    Effort::ORDER
        .contains(&level)
        .then(|| (base, level.parse().ok()))
        .and_then(|(b, l)| Some((b, l?)))
}

/// One entry per model, with the levels its variants name, lowest first (`gpt-oss-120b-medium` has one).
fn group(ids: Vec<String>) -> Vec<agenium_llm::ModelInfo> {
    let mut out: Vec<agenium_llm::ModelInfo> = Vec::new();
    for id in ids {
        let (base, level) = match level_of(&id) {
            Some((b, l)) => (b.to_owned(), Some(l)),
            None => (id.clone(), None),
        };
        let i = match out.iter().position(|m| m.id == base) {
            Some(i) => i,
            None => {
                out.push(agenium_llm::ModelInfo {
                    id: base,
                    efforts: level.as_ref().map(|_| vec![]),
                    ..Default::default()
                });
                out.len() - 1
            }
        };
        if let (Some(l), Some(levels)) = (level, out[i].efforts.as_mut()) {
            levels.push(l);
            levels.sort_by_key(|e| e.rank());
        }
    }
    out
}

#[cfg(test)]
mod tests;
