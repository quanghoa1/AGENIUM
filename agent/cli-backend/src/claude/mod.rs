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
use serde_json::{json, Value};

use crate::driver::{
    file_access, hook_call_secs, named_paths, original, own_tool_off, Check, CheckCtx, Control,
    Driver, Event, Setup, Turns, NO_SUBAGENTS,
};
use crate::driver::{HOOK_TIMEOUT, HOOK_TOKEN_ENV, HOOK_URL_ENV, MCP_CALL_TIMEOUT};
use crate::login_access::{lend_login, promote, sync_lent};

mod stream;
mod usage;
pub use usage::rate_limits;

/// Claude Code's login lent to a run (ADR-137): its `claudeAiOauth` without the refresh token (the tokens of the user's
/// own MCP connectors, `mcpOAuth`, stay out too), and when its access token expires (`expiresAt`, milliseconds).
pub(super) fn lend(login: &Value) -> Option<(Value, std::time::SystemTime)> {
    let mut oauth = login.get("claudeAiOauth")?.clone();
    let ms = oauth.get("expiresAt")?.as_u64()?;
    oauth.as_object_mut()?.remove("refreshToken");
    Some((
        json!({ "claudeAiOauth": oauth }),
        std::time::UNIX_EPOCH + std::time::Duration::from_millis(ms),
    ))
}

/// Claude Code tools an Agenium run may use (ADR-129): its own file and search tools for its scratch folder only (the per-run home, where it saves a long tool
/// result and reads it back); the hook refuses them on the worktree. Its shells are not offered: commands go through
/// Agenium's `shell`. `WebSearch` runs on Anthropic's servers and touches nothing here (ADR-163); its `WebFetch` would
/// read pages from this machine, so pages go through Agenium's `fetch`. Everything else (scheduling, notifications,
/// plugins, worktrees, workflows...) is withheld. Not its `ToolSearch`: the Agenium tools are listed, as for every
/// other CLI, and other servers' tools are found with Agenium's `tool_search` (ADR-139; `ENABLE_TOOL_SEARCH=false`,
/// 2026-11-04).
const BASE_TOOLS: [&str; 6] = ["Read", "Write", "Edit", "Glob", "Grep", "WebSearch"];
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
        resume: Option<&str>,
        subagents: bool,
    ) -> Vec<String> {
        // The session is written to the per-run home, which Agenium keeps for the ticket's next run (ADR-134).
        let mut a: Vec<String> = [
            "-p",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--verbose",
            // The answer as it is written (`stream_event` lines): chats and the helpers show it live.
            "--include-partial-messages",
            "--dangerously-skip-permissions",
            "--setting-sources",
            "user",
            "--strict-mcp-config",
            // Skills and slash commands: the ones of the user's claude.ai account would come with the login.
            // Agenium's skills reach the agent through `load_skill`.
            "--disable-slash-commands",
        ]
        .map(str::to_owned)
        .to_vec();
        if let Some(id) = resume {
            a.extend(["--resume".to_owned(), id.to_owned()]);
        }
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
        // Lent without its refresh token (ADR-137), not linked like Codex's (ADR-136): Claude Code writes its login to
        // a temporary file renamed over it, which would part a hard link at the first refresh, and refuses a symbolic one.
        lend_login(&user.join(LOGIN), &s.home.join(LOGIN), lend, s.run_for)?;
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
            // The Agenium tools listed, not deferred behind Claude Code's own search (one search for all: Agenium's).
            ("ENABLE_TOOL_SEARCH".to_owned(), "false".to_owned()),
            (HOOK_URL_ENV.to_owned(), s.hook_url.to_owned()),
            (HOOK_TOKEN_ENV.to_owned(), token.to_owned()),
        ]
        .into())
    }

    fn resumes_across_runs(&self) -> bool {
        true
    }

    fn login_file(&self) -> Option<&'static str> {
        Some(LOGIN)
    }

    fn finish(&self, s: &Setup) -> std::io::Result<()> {
        match user_claude_dir() {
            Some(user) => sync_lent(&user.join(LOGIN), &s.home.join(LOGIN), lend),
            None => Ok(()),
        }
    }

    fn promote_login(&self, s: &Setup) -> bool {
        user_claude_dir().is_some_and(|u| promote(&u.join(LOGIN), &s.home.join(LOGIN), lend))
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

    fn own_web_search(&self) -> bool {
        true
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
        if name == "WebFetch" {
            return Check::Deny(
                "use the `fetch` tool from the \"agenium\" MCP server instead (it reads PDFs too and is logged); WebFetch is off".into(),
            );
        }
        if name == "WebSearch" {
            return Check::Allow;
        }
        if SUBAGENT_TOOLS.contains(&name) && !cx.subagents {
            return Check::Deny(NO_SUBAGENTS.into());
        }
        if !allowed_tools(cx.subagents).contains(&name) {
            return Check::Deny(own_tool_off(name));
        }
        if SUBAGENT_TOOLS.contains(&name) {
            return Check::Allow;
        }
        // Its own file and search tools: its scratch folder only. A search pattern may not start elsewhere (Glob's
        // `pattern` was not read: `C:/Users/me/.ssh/*` searched from its own base).
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

/// The models offered for Claude Code's `--model`, by their full ids: its aliases (`haiku`, `sonnet`, `opus`) are left
/// out, since what they name changes with each release (an agent set to one still runs: `effort::alias_of`). The CLI
/// has no command that lists them, so this follows Anthropic's model list: the current ones first, then the earlier
/// ones still served (Claude Code's own picker offers them too; the plan decides which answer). Retired ones are out.
// shortcut: a fixed list, checked against Anthropic's models on 2026-11-18; update it when a model ships or retires.
pub const MODELS: [&str; 13] = [
    "claude-haiku-5-5",
    "claude-sonnet-5-5",
    "claude-opus-5-5",
    "claude-fable-5-2",
    "claude-fable-5-1",
    "claude-fable-5",
    "claude-opus-5",
    "claude-opus-4-8",
    "claude-opus-4-7",
    "claude-opus-4-6",
    "claude-sonnet-5",
    "claude-sonnet-4-6",
    "claude-haiku-4-5",
];

#[cfg(test)]
mod tests;
