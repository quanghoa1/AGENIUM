//! Devin CLI (`devin`) driver over ACP (Agent Client Protocol), checked with devin 3000.11.3 on 2026-11-04 (ADR-156).
//!
//! - Headless: `devin --config <home>/config.json acp --model <m>` speaks JSON-RPC on stdio. Each turn is one process
//!   (`Turns::ProcessPerTurn`): `initialize`, then `session/new` (the first turn) or `session/load` (later turns, the
//!   session id kept by the run), `session/set_mode bypass`, then `session/prompt`. `-p` prints only the last text.
//! - Control: the whole process runs inside Agenium's sandbox (`Control::Sandbox`): its own exec runs contained, as
//!   Codex's does (Devin's hooks do not run under ACP, and it asks before a command only when it judges one risky).
//!   Devin hands file reads and writes to the client (`fs/read_text_file`, `fs/write_text_file`): the bridge asks
//!   Agenium (the hook endpoint, `check`) and does them itself, so a read-only run writes nothing and nothing outside
//!   the worktree is touched.
//! - Isolation: Devin keeps its login, configuration, plugins and sessions under `%APPDATA%\devin` (the Known Folder,
//!   not `APPDATA`, so no per-run home) and loads the user's MCP servers from there and from other tools'
//!   configurations. In the sandbox it sees only its login ([`LOGIN_FILES`], read-only) and the `cli` folder's
//!   entries but `plugins` (logs, sessions; writable): not the user's `config.json`, `mcp_config.json` or plugins,
//!   nor the other tools' files. `--config` gives it Agenium's `config.json`, the worktree's
//!   `.devin/mcp_config.local.json` turns each of the user's own servers off by name (for a run without the
//!   sandbox), and Agenium's MCP server comes in `session/new`.

mod bridge;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use agenium_core::Effort;
use agenium_llm::Usage;
use serde_json::{json, Value};

use crate::driver::{inside, normalize, Check, CheckCtx, Control, Driver, Event, Setup, Turns};
use crate::driver::{HOOK_TOKEN_ENV, HOOK_URL_ENV};

pub(crate) use bridge::bridge;

/// Environment the bridge reads its run's settings from (never passed to Devin).
pub(crate) const MCP_URL_ENV: &str = "AGENIUM_DEVIN_MCP_URL";
pub(crate) const MCP_NAME_ENV: &str = "AGENIUM_DEVIN_MCP_NAME";

/// The file in the worktree that turns the user's own Devin MCP servers off for the run.
pub const LOCAL_MCP: &str = ".devin/mcp_config.local.json";

pub struct Devin;

/// Devin's own folder (`%APPDATA%\devin` on Windows, `~/.config/devin` elsewhere).
fn devin_dir() -> Option<PathBuf> {
    if cfg!(windows) {
        std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("devin"))
    } else {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config").join("devin"))
    }
}

/// The names of the user's own Devin MCP servers (names only: their settings hold tokens).
fn user_servers() -> Vec<String> {
    devin_dir()
        .and_then(|d| std::fs::read_to_string(d.join("mcp_config.json")).ok())
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|v| {
            v["mcpServers"]
                .as_object()
                .map(|o| o.keys().cloned().collect())
        })
        .unwrap_or_default()
}

/// What Devin reads of its own folder in the sandbox: its login and machine id, nothing else (2026-11-04, 3000.11.234).
const LOGIN_FILES: [&str; 3] = ["credentials.toml", "machineid", ".devin-migration-complete"];

/// Its folder's `cli` entries the run may write (logs, sessions): all but the user's plugins.
fn cli_entries() -> Vec<PathBuf> {
    // shortcut: entries that exist when the run starts; a folder a later Devin adds beside them is not writable until
    // it exists (Devin made it in the user's own run first), upgrade to a fixed list if one is missed.
    let Some(cli) = devin_dir().map(|d| d.join("cli")) else {
        return Vec::new();
    };
    std::fs::read_dir(&cli)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_name() != "plugins")
        .map(|e| e.path())
        .collect()
}

/// `.devin/mcp_config.local.json` with each of `servers` off, what the file already held kept.
fn local_mcp(existing: Option<&str>, servers: &[String]) -> Value {
    let mut v = existing
        .and_then(|s| serde_json::from_str::<Value>(s).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}));
    if !v["mcpServers"].is_object() {
        v["mcpServers"] = json!({});
    }
    for s in servers {
        v["mcpServers"][s] = json!({"disabled": true});
    }
    v
}

impl Driver for Devin {
    fn control(&self) -> Control {
        Control::Sandbox
    }

    fn turns(&self) -> Turns {
        Turns::ProcessPerTurn
    }

    /// Devin names a model with its effort (`swe-2-max`): the agent's model and effort are put together here
    /// ([`variant`]); the session id goes to the bridge, which loads it.
    fn args(
        &self,
        model: &str,
        effort: Option<&Effort>,
        home: &Path,
        _resume: Option<&str>,
        _subagents: bool,
    ) -> Vec<String> {
        let mut a = vec![
            "--config".to_owned(),
            home.join("config.json").display().to_string(),
            "--respect-workspace-trust".to_owned(),
            "false".to_owned(),
            "acp".to_owned(),
        ];
        if !model.is_empty() {
            a.extend(["--model".to_owned(), variant(model, effort)]);
        }
        a
    }

    fn prepare(&self, s: &Setup) -> std::io::Result<BTreeMap<String, String>> {
        // No hooks, no permissions of the user's: Devin's own config is replaced for the run. The model comes in
        // `--model` with its effort (the user's `DEVIN_MODEL` does not reach the run: its environment is Agenium's).
        let config = json!({"version": 1});
        std::fs::write(
            s.home.join("config.json"),
            serde_json::to_vec_pretty(&config)?,
        )?;
        let local = s.worktree.join(LOCAL_MCP);
        let existing = std::fs::read_to_string(&local).ok();
        if let Some(dir) = local.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(
            &local,
            serde_json::to_vec_pretty(&local_mcp(existing.as_deref(), &user_servers()))?,
        )?;
        let token = s
            .mcp_authorization
            .strip_prefix("Bearer ")
            .unwrap_or(s.mcp_authorization);
        // Devin finds its config directory from these (without them it does not start); the sandbox still shows it
        // only its login and its `cli` folder but the plugins.
        let known: Vec<(String, String)> = ["APPDATA", "LOCALAPPDATA", "USERPROFILE"]
            .into_iter()
            .filter_map(|k| std::env::var(k).ok().map(|v| (k.to_owned(), v)))
            .collect();
        // Its own `exec` takes `bash` from PATH, on Windows WSL's launcher in System32, which cannot start in the
        // sandbox (0xC0000142): `SHELL` makes it PowerShell, which runs there (checked 2026-11-05, 3000.11.234).
        let shell = cfg!(windows).then(|| ("SHELL".to_owned(), "powershell.exe".to_owned()));
        Ok([
            (HOOK_URL_ENV, s.hook_url),
            (HOOK_TOKEN_ENV, token),
            (MCP_URL_ENV, s.mcp_url),
            (MCP_NAME_ENV, s.mcp_name),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .chain(known)
        .chain(shell)
        .collect())
    }

    fn acp(&self) -> bool {
        true
    }

    /// Devin's binary and its login, read-only: not the user's configuration, MCP servers or plugins.
    fn read_roots(&self, program: &Path) -> Vec<PathBuf> {
        let dir = devin_dir();
        program
            .parent()
            .map(Path::to_path_buf)
            .into_iter()
            .chain(
                LOGIN_FILES
                    .iter()
                    .filter_map(|f| dir.as_ref().map(|d| d.join(f))),
            )
            .collect()
    }

    /// Where Devin writes its logs and sessions (it does not start without them), its plugins left out.
    fn writable_roots(&self) -> Vec<PathBuf> {
        cli_entries()
    }

    fn resumes_across_runs(&self) -> bool {
        true
    }

    fn user_input(&self, text: &str) -> String {
        format!("{}\n", json!({"text": text}))
    }

    /// The bridge's messages: `{"type":"session"}`, `{"type":"update"}` (an ACP `session/update`), `{"type":"turn"}`
    /// (the prompt's answer) and `{"type":"failed"}`.
    fn parse(&self, v: &Value) -> Vec<Event> {
        match v["type"].as_str() {
            Some("session") => v["id"]
                .as_str()
                .map(|s| vec![Event::Session(s.to_owned())])
                .unwrap_or_default(),
            Some("update") => update(&v["update"]),
            Some("turn") => {
                let u = &v["result"]["usage"];
                let n = |k: &str| u[k].as_u64().unwrap_or(0);
                let cached = n("cachedReadTokens");
                vec![Event::TurnEnd {
                    usage: Usage {
                        input: n("inputTokens").saturating_sub(cached),
                        output: n("outputTokens"),
                        cache_read: cached,
                        ..Default::default()
                    },
                    response: String::new(),
                    denied: vec![],
                    error: match v["result"]["stopReason"].as_str() {
                        Some("end_turn") | None => None,
                        Some(other) => Some(format!("devin stopped: {other}")),
                    },
                }]
            }
            Some("failed") => vec![Event::TurnEnd {
                usage: Usage::default(),
                response: String::new(),
                denied: vec![],
                error: Some(v["error"].as_str().unwrap_or("devin failed").to_owned()),
            }],
            _ => vec![],
        }
    }

    fn command(&self, tool: &str, input: &Value) -> Option<String> {
        (tool == "exec")
            .then(|| input["command"].as_str().map(str::to_owned))
            .flatten()
    }

    fn is_agenium_call(&self, tool: &str, _input: &Value, server: &str) -> bool {
        tool.starts_with(&format!("mcp__{server}__"))
    }

    /// The bridge's questions: a file Devin hands to the client to read or write, and a permission Devin asks for.
    fn check(&self, payload: &Value, cx: &CheckCtx) -> Check {
        let (name, input) = crate::driver::hook_call(payload);
        let path = input["path"]
            .as_str()
            .map(|p| normalize(Path::new(p), cx.worktree));
        match name.as_str() {
            "read" => match path {
                Some(p)
                    if inside(&p, cx.worktree, cx.worktree) || inside(&p, cx.worktree, cx.home) =>
                {
                    Check::Allow
                }
                Some(p) => Check::Deny(format!("{} is outside the ticket worktree", p.display())),
                None => Check::Deny("no path".into()),
            },
            "write" => match path {
                _ if cx.read_only => Check::Deny("this run only reads: no file is written".into()),
                Some(p) if p.ends_with(LOCAL_MCP) => {
                    Check::Deny("that file is Agenium's for this run".into())
                }
                Some(p)
                    if inside(&p, cx.worktree, cx.worktree) || inside(&p, cx.worktree, cx.home) =>
                {
                    Check::Allow
                }
                Some(p) => Check::Deny(format!("{} is outside the ticket worktree", p.display())),
                None => Check::Deny("no path".into()),
            },
            n if self.is_agenium_call(n, &input, cx.mcp_name) => Check::Allow,
            n if n.starts_with("mcp__") => {
                Check::Deny(format!("{n} is not available in an Agenium run"))
            }
            // Its own command or edit Devin judged risky: it runs in the sandbox, which bounds it.
            _ if cx.read_only => Check::Deny("this run only reads".into()),
            _ => Check::Allow,
        }
    }
}

/// The models Devin offers (`devin models list --format json`): one per model, its efforts from its variants
/// (`swe-2-high`, `swe-2-max`: `swe-2` at `high`, `max`). The variant of each model and effort is kept for the runs.
pub async fn models(program: &Path) -> Result<Vec<agenium_llm::ModelInfo>, String> {
    let mut cmd = tokio::process::Command::new(program);
    cmd.args(["models", "list", "--format", "json"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    let out = tokio::time::timeout(std::time::Duration::from_secs(40), cmd.output())
        .await
        .map_err(|_| "devin models list did not answer in 40 s".to_owned())?
        .map_err(|e| format!("cannot start devin: {e}"))?;
    let (models, variants) = parse_models(&String::from_utf8_lossy(&out.stdout))?;
    *VARIANTS.write().unwrap_or_else(|p| p.into_inner()) = Some(variants);
    Ok(models)
}

/// Words a Devin variant may end with after its effort: another line of the same model (`swe-2-high-lightning`).
/// shortcut: the list seen in devin 3000.11.3's models; a new kind of line reads as a model of its own until added.
const LINES: [&str; 4] = ["lightning", "fast", "priority", "1m"];

/// Each model and effort's own variant, as the last model list named them (`swe-1-7` at `max` is `swe-1-7`, the
/// variant with no effort in its name, labelled "Max"). Filled whenever the list is read (every start of the app).
static VARIANTS: std::sync::RwLock<Option<Variants>> = std::sync::RwLock::new(None);

type Variants = std::collections::HashMap<(String, String), String>;

/// Whether this process read Devin's model list yet (its runs name variants from it).
pub fn variants_known() -> bool {
    VARIANTS.read().unwrap_or_else(|p| p.into_inner()).is_some()
}

/// A Devin variant's model and effort: `swe-2-max` is `swe-2` at `max`, `swe-2-high-lightning` is `swe-2-lightning` at
/// `high`; `adaptive` has no effort.
fn split(uid: &str) -> (String, Option<Effort>) {
    let words: Vec<&str> = uid.split('-').collect();
    let level = words.iter().rposition(|w| Effort::ORDER.contains(w));
    match level {
        Some(i) if i > 0 && words[i + 1..].iter().all(|w| LINES.contains(w)) => {
            let mut id = words[..i].to_vec();
            id.extend(&words[i + 1..]);
            (id.join("-"), words[i].parse().ok())
        }
        _ => (uid.to_owned(), None),
    }
}

/// The variant Devin runs for `model` at `effort` (`swe-2` at `max` is `swe-2-max`): the one the model list named, else
/// the effort put in the name. A model that already names its effort (an agent saved with `swe-2-max`) runs as it is.
fn variant(model: &str, effort: Option<&Effort>) -> String {
    let Some(e) = effort else {
        return model.to_owned();
    };
    let known = VARIANTS
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .as_ref()
        .and_then(|m| m.get(&(model.to_owned(), e.as_str().to_owned())).cloned());
    if let Some(uid) = known {
        return uid;
    }
    if split(model).1.is_some() {
        return model.to_owned();
    }
    let mut words: Vec<&str> = model.split('-').collect();
    let at = words
        .iter()
        .position(|w| LINES.contains(w))
        .unwrap_or(words.len());
    words.insert(at, e.as_str());
    words.join("-")
}

fn parse_models(text: &str) -> Result<(Vec<agenium_llm::ModelInfo>, Variants), String> {
    let v: Value = serde_json::from_str(text.trim_start_matches('\u{feff}').trim())
        .map_err(|e| format!("devin models list: {e}"))?;
    let mut out: Vec<agenium_llm::ModelInfo> = Vec::new();
    let mut variants = Variants::new();
    for f in v["families"].as_array().into_iter().flatten() {
        // shortcut: Devin 3000.11.234's "fusion" pairs (a model + a SWE-2 sidekick, 655 variants) are left out of the
        // list; add them when an agent should pick a pair.
        if f["family_uid"].as_str() == Some("fusion") {
            continue;
        }
        let all = f["variants"].as_array().cloned().unwrap_or_default();
        for m in &all {
            let Some(uid) = m["model_uid"].as_str() else {
                continue;
            };
            let label = m["label"].as_str().unwrap_or(uid);
            let (id, mut level) = split(uid);
            // `swe-1-7` beside `swe-1-7-medium` is the model at the level its label ends with ("SWE-1.7 Max").
            let has_siblings = all.iter().any(|o| {
                o["model_uid"]
                    .as_str()
                    .is_some_and(|u| u != uid && split(u).0 == id)
            });
            if level.is_none() && has_siblings {
                level = label
                    .rsplit(' ')
                    .next()
                    .and_then(|w| w.to_ascii_lowercase().parse::<Effort>().ok())
                    .filter(|e| e.rank().is_some());
            }
            if let Some(l) = &level {
                variants.insert((id.clone(), l.as_str().to_owned()), uid.to_owned());
            }
            let i = match out.iter().position(|x| x.id == id) {
                Some(i) => i,
                None => {
                    out.push(agenium_llm::ModelInfo {
                        id: id.clone(),
                        display_name: Some(without_level(label, level.as_ref())),
                        context_window: m["max_context_tokens"].as_u64(),
                        max_output: m["max_output_tokens"].as_u64(),
                        efforts: Some(vec![]),
                        // The level Devin names first for a model is its default (`swe-2-high` before `-medium`).
                        default_effort: level.clone(),
                        ..Default::default()
                    });
                    out.len() - 1
                }
            };
            if let (Some(l), Some(levels)) = (level, out[i].efforts.as_mut()) {
                if !levels.contains(&l) {
                    levels.push(l);
                    levels.sort_by_key(|e| e.rank());
                }
            }
        }
    }
    Ok((out, variants))
}

/// A variant's label without its effort word ("SWE-2 Medium Lightning" names "SWE-2 Lightning").
fn without_level(label: &str, level: Option<&Effort>) -> String {
    match level {
        Some(e) => label
            .split(' ')
            .filter(|w| !w.eq_ignore_ascii_case(e.as_str()))
            .collect::<Vec<_>>()
            .join(" "),
        None => label.to_owned(),
    }
}

/// One ACP `session/update` as the run's events (the bridge has put a tool's name and input on its updates).
fn update(u: &Value) -> Vec<Event> {
    match u["sessionUpdate"].as_str() {
        Some("agent_message_chunk") => u["content"]["text"]
            .as_str()
            .filter(|t| !t.is_empty())
            .map(|t| vec![Event::Text(t.to_owned())])
            .unwrap_or_default(),
        Some(kind @ ("tool_call" | "tool_call_update")) => {
            let done = matches!(u["status"].as_str(), Some("completed" | "failed"));
            let id = u["toolCallId"].as_str().unwrap_or("").to_owned();
            // A command's output comes while it runs (`terminalPreview`, then `terminal_exit` with its code); the
            // update that completes it carries none (captured 2026-11-05): each one with text replaces the last.
            let exit = u["_meta"]["terminal_exit"]["exit_code"].as_i64();
            let error = u["status"] == "failed" || exit.is_some_and(|c| c != 0);
            let text = u["content"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|c| c["content"]["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n");
            if kind == "tool_call_update" && !done {
                return if text.is_empty() {
                    vec![]
                } else {
                    vec![Event::ToolOutput { id, text, error }]
                };
            }
            let output = (done && (error || !text.is_empty())).then(|| Event::ToolOutput {
                id: id.clone(),
                text,
                error,
            });
            output
                .into_iter()
                .chain([Event::Tool {
                    id,
                    name: tool_name(u),
                    input: u["rawInput"].clone(),
                    done,
                }])
                .collect()
        }
        Some("usage_update") => u["used"]
            .as_u64()
            .or_else(|| u["usage"]["inputTokens"].as_u64())
            .map(|n| vec![Event::Context(n)])
            .unwrap_or_default(),
        _ => vec![],
    }
}

/// A tool call's own name: Devin's (`write`, `exec`, `mcp__agenium__submit_report`), else its kind.
fn tool_name(u: &Value) -> String {
    let m = &u["_meta"];
    m["cognition.ai/inferenceToolName"]
        .as_str()
        .or_else(|| m["cognition.ai/toolName"].as_str())
        .or_else(|| u["kind"].as_str())
        .unwrap_or("tool")
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Updates captured from devin 3000.11.3 (`swe-2-max`, 2026-11-04), shortened.
    #[test]
    fn reads_the_acp_updates() {
        let ev = |s: &str| Devin.parse(&serde_json::from_str(s).unwrap());
        assert_eq!(
            ev(r#"{"type":"session","id":"flax-poppyseed"}"#),
            [Event::Session("flax-poppyseed".into())]
        );
        assert_eq!(
            ev(
                r#"{"type":"update","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"DONE"}}}"#
            ),
            [Event::Text("DONE".into())]
        );
        let call = ev(
            r#"{"type":"update","update":{"sessionUpdate":"tool_call","toolCallId":"exec:0#1","title":"Ran echo","kind":"execute","rawInput":{"command":"echo hi > cmd.txt","workdir":"w"},"_meta":{"cognition.ai/inferenceToolName":"exec"}}}"#,
        );
        match &call[0] {
            Event::Tool {
                name, input, done, ..
            } => {
                assert_eq!(name, "exec");
                assert!(!done);
                assert_eq!(
                    Devin.command(name, input).as_deref(),
                    Some("echo hi > cmd.txt")
                );
            }
            e => panic!("{e:?}"),
        }
        assert!(ev(r#"{"type":"update","update":{"sessionUpdate":"tool_call_update","toolCallId":"exec:0#1","status":"in_progress"}}"#).is_empty());
        // A command's output while it runs, its exit code with it (captured from devin 3000.11.234 in the sandbox).
        assert_eq!(
            ev(
                r#"{"type":"update","update":{"_meta":{"cognition.ai/inferenceToolName":"exec","terminal_exit":{"exit_code":1}},"content":[{"content":{"text":"boom","type":"text"},"type":"content"}],"sessionUpdate":"tool_call_update","status":"in_progress","toolCallId":"exec:0#2"}}"#
            ),
            [Event::ToolOutput {
                id: "exec:0#2".into(),
                text: "boom".into(),
                error: true
            }]
        );
        // What a call returned (captured from devin 3000.11.234): its content's text, before the end of the step.
        assert_eq!(
            ev(
                r#"{"type":"update","update":{"sessionUpdate":"tool_call_update","toolCallId":"m:1","status":"completed","content":[{"type":"content","content":{"type":"text","text":"a\nb"}}],"_meta":{"cognition.ai/inferenceToolName":"mcp__agenium__list_dir"}}}"#
            )[0],
            Event::ToolOutput {
                id: "m:1".into(),
                text: "a\nb".into(),
                error: false
            }
        );
        assert!(matches!(
            &ev(
                r#"{"type":"update","update":{"sessionUpdate":"tool_call_update","toolCallId":"exec:0#1","status":"completed","_meta":{"cognition.ai/inferenceToolName":"exec"}}}"#
            )[0],
            Event::Tool { done: true, .. }
        ));
        match &ev(
            r#"{"type":"turn","result":{"stopReason":"end_turn","usage":{"inputTokens":18000,"outputTokens":30,"cachedReadTokens":17900}}}"#,
        )[0]
        {
            Event::TurnEnd { usage, error, .. } => {
                assert_eq!(
                    (usage.input, usage.cache_read, usage.output),
                    (100, 17_900, 30)
                );
                assert!(error.is_none());
            }
            e => panic!("{e:?}"),
        }
        assert!(Devin.is_agenium_call("mcp__agenium__submit_report", &Value::Null, "agenium"));
        assert!(!Devin.is_agenium_call("mcp__github__search", &Value::Null, "agenium"));
    }

    #[test]
    fn files_are_read_and_written_in_the_worktree_only() {
        let w = std::env::temp_dir().join("agenium-devin-check").join("wt");
        let h = std::env::temp_dir()
            .join("agenium-devin-check")
            .join("home");
        let cx = |read_only| CheckCtx {
            worktree: &w,
            home: &h,
            mcp_name: "agenium",
            subagents: false,
            read_only,
        };
        let ask =
            |tool: &str, path: &Path| json!({"tool_name": tool, "tool_input": {"path": path}});
        let inside_file = w.join("src").join("a.rs");
        let outside = std::env::temp_dir().join("elsewhere.txt");
        assert_eq!(
            Devin.check(&ask("read", &inside_file), &cx(true)),
            Check::Allow
        );
        assert_eq!(
            Devin.check(&ask("write", &inside_file), &cx(false)),
            Check::Allow
        );
        assert!(matches!(
            Devin.check(&ask("write", &inside_file), &cx(true)),
            Check::Deny(_)
        ));
        assert!(matches!(
            Devin.check(&ask("write", &outside), &cx(false)),
            Check::Deny(_)
        ));
        assert!(matches!(
            Devin.check(&ask("read", &outside), &cx(false)),
            Check::Deny(_)
        ));
        assert!(matches!(
            Devin.check(&ask("write", &w.join(LOCAL_MCP)), &cx(false)),
            Check::Deny(_)
        ));
        let mcp = |n: &str| json!({"tool_name": n, "tool_input": {}});
        assert_eq!(
            Devin.check(&mcp("mcp__agenium__submit_report"), &cx(false)),
            Check::Allow
        );
        assert!(matches!(
            Devin.check(&mcp("mcp__github__search"), &cx(false)),
            Check::Deny(_)
        ));
    }

    #[test]
    fn a_model_is_its_name_and_its_effort_apart() {
        let m = parse_models(
            r#"{"families":[{"family_uid":"swe-2","variants":[
                {"model_uid":"swe-2-high","label":"SWE-2 High","max_context_tokens":262000},
                {"model_uid":"swe-2-medium","label":"SWE-2 Medium","max_context_tokens":262000},
                {"model_uid":"swe-2-max","label":"SWE-2 Max","max_context_tokens":262000},
                {"model_uid":"swe-2-medium-lightning","label":"SWE-2 Medium Lightning"},
                {"model_uid":"swe-2-max-lightning","label":"SWE-2 Max Lightning"}]},
              {"family_uid":"adaptive","variants":[{"model_uid":"adaptive","label":"Adaptive"}]},
              {"family_uid":"glm-5.2","variants":[{"model_uid":"glm-5-2"},{"model_uid":"glm-5-2-max"},{"model_uid":"glm-5-2-max-1m"}]},
              {"family_uid":"swe-1.7","variants":[{"model_uid":"swe-1-7","label":"SWE-1.7 Max"},{"model_uid":"swe-1-7-medium","label":"SWE-1.7 Medium"}]},
              {"family_uid":"fusion","variants":[{"model_uid":"fusion-claude-opus-5-5-medium-sidekick-swe-2-medium"}]}]}"#,
        )
        .unwrap();
        let (m, variants) = m;
        let ids: Vec<&str> = m.iter().map(|x| x.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "swe-2",
                "swe-2-lightning",
                "adaptive",
                "glm-5-2",
                "glm-5-2-1m",
                "swe-1-7"
            ]
        );
        let names = |i: usize| {
            m[i].efforts
                .clone()
                .unwrap()
                .iter()
                .map(|e| e.as_str().to_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(names(0), ["medium", "high", "max"]);
        assert_eq!(m[0].display_name.as_deref(), Some("SWE-2"));
        assert_eq!(
            m[0].default_effort.as_ref().map(|e| e.as_str()),
            Some("high")
        );
        assert_eq!(m[0].context_window, Some(262_000));
        assert_eq!(names(1), ["medium", "max"]);
        assert!(names(2).is_empty());
        // The variant with no effort in its name, at the level its label names.
        assert_eq!(names(5), ["medium", "max"]);
        assert_eq!(
            variants[&("swe-1-7".to_owned(), "max".to_owned())],
            "swe-1-7"
        );
        assert_eq!(m[1].display_name.as_deref(), Some("SWE-2 Lightning"));
        assert_eq!(m[5].display_name.as_deref(), Some("SWE-1.7"));
        // Run as Devin names it.
        assert_eq!(variant("swe-2", Some(&Effort::MAX)), "swe-2-max");
        assert_eq!(
            variant("swe-2-lightning", Some(&Effort::HIGH)),
            "swe-2-high-lightning"
        );
        assert_eq!(variant("adaptive", None), "adaptive");
        assert_eq!(
            variant("swe-2-max", Some(&Effort::MEDIUM)),
            "swe-2-max",
            "an agent saved with the old name"
        );
        *VARIANTS.write().unwrap() = Some(variants);
        assert_eq!(variant("swe-1-7", Some(&Effort::MAX)), "swe-1-7");
        assert_eq!(variant("swe-1-7", Some(&Effort::MEDIUM)), "swe-1-7-medium");
        let a = Devin.args("swe-2", Some(&Effort::MAX), Path::new("/h"), None, false);
        assert_eq!(
            a[a.len() - 2..],
            ["--model".to_owned(), "swe-2-max".to_owned()]
        );
    }

    #[test]
    fn the_run_sees_devins_login_but_not_the_users_plugins_or_config() {
        let read = Devin.read_roots(Path::new("/bin/devin"));
        let names: Vec<String> = read
            .iter()
            .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .collect();
        for f in ["config.json", "mcp_config.json", "cli", "devin", "plugins"] {
            assert!(!names.iter().any(|n| n == f), "{f} must not be readable");
        }
        assert!(Devin
            .writable_roots()
            .iter()
            .all(|p| p.file_name().is_some_and(|n| n != "plugins")));
    }

    #[test]
    fn the_users_servers_are_turned_off_and_the_rest_kept() {
        let v = local_mcp(
            Some(r#"{"mcpServers":{"mine":{"command":"x"}},"other":1}"#),
            &["github".into(), "memory".into()],
        );
        assert_eq!(v["mcpServers"]["github"], json!({"disabled": true}));
        assert_eq!(v["mcpServers"]["memory"], json!({"disabled": true}));
        assert_eq!(v["mcpServers"]["mine"]["command"], "x");
        assert_eq!(v["other"], 1);
        assert_eq!(
            local_mcp(Some("not json"), &["a".into()])["mcpServers"]["a"]["disabled"],
            true
        );
    }
}
