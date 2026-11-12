//! A plugin's hooks, run Agenium's way (ADR-157): Claude Code's hook commands, each in Agenium's sandbox (the plugin's
//! folder read-only, a data folder of its own the only place it writes, no network, a time limit), only for the
//! agents that have the plugin on. What a hook says reaches the model in the run's instructions, rebuilt at every
//! run and never stored in a conversation: a plugin switched off or removed leaves nothing behind in a prompt.
//!
//! - `SessionStart` (matcher on `startup`): at the start of every run; its text joins the run's instructions.
//! - `UserPromptSubmit`: on each of the user's messages to the Lead or Solo; its text joins that turn's instructions.
//! - `PreToolUse` (matcher on Claude Code's tool names): before each tool call; it may refuse a call, never allow one
//!   Agenium's rules refuse.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use agenium_sandbox::{CommandSpec, Executor, SandboxMode, SandboxPolicy, Shell};
use agenium_tools::{Tool, ToolCtx, ToolError, ToolOutput};
use async_trait::async_trait;
use serde::Serialize;
use serde_json::{json, Value};

use super::layout;
use crate::app::App;

/// The most text one hook adds to a run's instructions.
const MAX_CONTEXT: usize = 12_000;
/// The longest a hook may take (seconds), whatever it asks for.
const MAX_SECS: u64 = 60;

/// One hook's run, shown in the chat as the plugin's (never sent to a model as such).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct HookNote {
    pub plugin: String,
    pub event: String,
    pub ok: bool,
    /// What it added to the instructions, or why it failed.
    pub text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    Go,
    Refuse(String),
}

#[derive(Debug, Clone)]
pub struct HookRun {
    pub note: HookNote,
    /// Text for the run's instructions.
    pub context: Option<String>,
    pub verdict: Verdict,
}

/// Claude Code's name of an Agenium tool, which a `PreToolUse` matcher names (`Bash`, `Read`, `Edit`…).
pub fn claude_tool_name(name: &str) -> &str {
    match name {
        "shell" => "Bash",
        "read_file" => "Read",
        "write_file" => "Write",
        "edit_file" | "apply_patch" => "Edit",
        "multi_edit" => "MultiEdit",
        "glob" => "Glob",
        "grep" => "Grep",
        "list_dir" => "LS",
        "fetch" => "WebFetch",
        "web_search" => "WebSearch",
        "todo" => "TodoWrite",
        other => other,
    }
}

/// Whether a hook group's matcher takes `on` (absent, empty or `*`: every one).
fn matches(matcher: &Value, on: &str) -> bool {
    match matcher.as_str().map(str::trim) {
        None | Some("") | Some("*") => true,
        Some(m) => regex::Regex::new(&format!("^(?:{m})$"))
            .map(|r| r.is_match(on))
            .unwrap_or(false),
    }
}

fn cut(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_owned()
    } else {
        let mut t: String = s.chars().take(n).collect();
        t.push_str("\n[…cut]");
        t
    }
}

/// The hooks of `event` an installed plugin has (from its `agenium-plugin.json`).
pub(crate) fn hooks_of(dir: &Path, event: &str) -> Vec<Value> {
    let Ok(text) = std::fs::read_to_string(dir.join(layout::MANIFEST)) else {
        return vec![];
    };
    let Ok(m) = serde_json::from_str::<Value>(&text) else {
        return vec![];
    };
    m["hooks"][event].as_array().cloned().unwrap_or_default()
}

/// A hook command for `shell`: its PowerShell form where the sandbox runs PowerShell and it has one, the plugin's
/// folders put in, and the hook's input on its standard input (read from `input`).
fn command_line(
    shell: &Shell,
    hook: &Value,
    vars: &[(&str, String)],
    input: &Path,
) -> Option<String> {
    let ps = matches!(shell, Shell::PowerShell(_));
    let raw = if ps {
        hook["powershell"].as_str().or(hook["command"].as_str())
    } else {
        hook["command"].as_str()
    }?;
    let mut c = raw.to_owned();
    for (k, v) in vars {
        c = c
            .replace(&format!("${{{k}}}"), v)
            .replace(&format!("${k}"), v);
    }
    let f = input.display().to_string();
    Some(match shell {
        Shell::Bash(_) => format!("{{ {c}\n}} < '{}'", f.replace('\'', "'\\''")),
        Shell::Cmd => format!("({c}) < \"{f}\""),
        Shell::PowerShell(_) => format!(
            "Get-Content -Raw -LiteralPath '{}' | {c}",
            f.replace('\'', "''")
        ),
    })
}

/// What a hook's exit and output mean (Claude Code's conventions: exit 2 refuses with its stderr; JSON output may
/// give `hookSpecificOutput.additionalContext` or a `permissionDecision`; plain output is context).
fn read_output(
    event: &str,
    code: Option<i32>,
    stdout: &str,
    stderr: &str,
) -> (bool, Option<String>, Verdict, String) {
    let pre = event == "PreToolUse";
    match code {
        Some(0) => {}
        Some(2) if pre => {
            let why = cut(stderr.trim(), 300);
            return (
                true,
                None,
                Verdict::Refuse(why.clone()),
                format!("refused: {why}"),
            );
        }
        // 0xC0000142: a program the hook starts could not start in Windows' sandbox (Git Bash cannot: S2).
        Some(-1073741502) => {
            let why =
                "a program it starts (Git Bash, most likely) cannot start in Windows' sandbox"
                    .to_owned();
            return (false, None, Verdict::Go, why);
        }
        c => {
            let code = c.map_or("no exit code".to_owned(), |c| format!("exit {c}"));
            let why = format!("{code}: {}", cut(stderr.trim(), 300));
            return (false, None, Verdict::Go, why);
        }
    }
    let out = stdout.trim();
    if let Ok(Value::Object(j)) = serde_json::from_str::<Value>(out) {
        let spec = &j.get("hookSpecificOutput").cloned().unwrap_or(Value::Null);
        let context = spec["additionalContext"]
            .as_str()
            .or(j.get("additionalContext").and_then(Value::as_str))
            .or(j.get("additional_context").and_then(Value::as_str))
            .map(|s| cut(s.trim(), MAX_CONTEXT))
            .filter(|s| !s.is_empty());
        let decision = spec["permissionDecision"]
            .as_str()
            .or(j.get("decision").and_then(Value::as_str))
            .unwrap_or("");
        // shortcut: "ask" refuses too (the agent is told to ask the user with request_permission); route it to the
        // approval queue when a plugin needs it.
        if pre && matches!(decision, "deny" | "block" | "ask") {
            let why = spec["permissionDecisionReason"]
                .as_str()
                .or(j.get("reason").and_then(Value::as_str))
                .unwrap_or("no reason given");
            let why = cut(why.trim(), 300);
            let why = if decision == "ask" {
                format!("{why} (it asks for the user's approval: use request_permission)")
            } else {
                why
            };
            return (
                true,
                None,
                Verdict::Refuse(why.clone()),
                format!("refused: {why}"),
            );
        }
        let shown = context.clone().unwrap_or_default();
        return (true, if pre { None } else { context }, Verdict::Go, shown);
    }
    let context = (!pre && !out.is_empty()).then(|| cut(out, MAX_CONTEXT));
    (
        true,
        context.clone(),
        Verdict::Go,
        context.unwrap_or_default(),
    )
}

impl App {
    /// Whether a plugin the agent has on has hooks of `event`.
    pub fn has_plugin_hooks(&self, skills: &[String], event: &str) -> bool {
        let dir = self.skill_registry().plugins_dir();
        skills
            .iter()
            .filter_map(|e| agenium_skills::plugin_of(e))
            .any(|p| !hooks_of(&dir.join(p), event).is_empty())
    }

    /// Runs the `event` hooks of the plugins the agent has on (`skills`, its binding) whose matcher takes `on`, each
    /// in the sandbox, with Claude Code's input (`input` plus the event, the run, the folder).
    pub async fn run_plugin_hooks(
        &self,
        skills: &[String],
        event: &str,
        on: &str,
        input: Value,
        run: &str,
        project_root: &Path,
    ) -> Vec<HookRun> {
        let dir = self.skill_registry().plugins_dir();
        let mut out = Vec::new();
        for plugin in skills.iter().filter_map(|e| agenium_skills::plugin_of(e)) {
            let root = dir.join(plugin);
            for group in hooks_of(&root, event) {
                if !matches(&group["matcher"], on) {
                    continue;
                }
                for hook in group["hooks"].as_array().into_iter().flatten() {
                    let r = run_hook(&root, hook, event, &input, run, project_root).await;
                    let (ok, context, verdict, text) = match r {
                        Ok((code, so, se)) => read_output(event, code, &so, &se),
                        Err(e) => (false, None, Verdict::Go, e),
                    };
                    if !ok {
                        tracing::warn!(plugin, event, error = %text, "a plugin hook failed");
                    }
                    out.push(HookRun {
                        note: HookNote {
                            plugin: plugin.to_owned(),
                            event: event.to_owned(),
                            ok,
                            text,
                        },
                        context,
                        verdict,
                    });
                }
            }
        }
        out
    }
}

/// One hook command in the sandbox: (exit code, stdout, stderr), or why it could not run.
async fn run_hook(
    plugin_dir: &Path,
    hook: &Value,
    event: &str,
    input: &Value,
    run: &str,
    project_root: &Path,
) -> Result<(Option<i32>, String, String), String> {
    // With the sandbox turned off (ADR-160) a hook runs directly on the host, like every other command.
    if agenium_sandbox::detect().backend.is_none() && !agenium_sandbox::is_off() {
        return Err("not run: this machine has no sandbox for a plugin's hook".into());
    }
    let data = plugin_dir.join("data");
    let tmp = data.join("tmp");
    std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    let shell = Shell::detect();
    let cwd = if project_root.is_dir() {
        project_root.to_path_buf()
    } else {
        data.clone()
    };
    let mut payload = input.clone();
    payload["hook_event_name"] = json!(event);
    payload["session_id"] = json!(run);
    payload["cwd"] = json!(cwd.display().to_string());
    payload["transcript_path"] = json!("");
    let file = tmp.join(format!("hook-{}.json", ulid::Ulid::new()));
    std::fs::write(&file, payload.to_string()).map_err(|e| e.to_string())?;
    let vars = [
        ("CLAUDE_PLUGIN_ROOT", plugin_dir.display().to_string()),
        ("CLAUDE_PLUGIN_DATA", data.display().to_string()),
        ("CLAUDE_PROJECT_DIR", cwd.display().to_string()),
    ];
    let result = async {
        let command = command_line(&shell, hook, &vars, &file).ok_or("the hook has no command")?;
        let spec = CommandSpec {
            command,
            cwd: cwd.clone(),
            timeout: Duration::from_secs(hook["timeout"].as_u64().unwrap_or(30).clamp(1, MAX_SECS)),
            env: vars
                .iter()
                .map(|(k, v)| ((*k).to_owned(), v.clone()))
                .collect(),
            ..Default::default()
        };
        let mut read_roots = agenium_sandbox::toolchain_read_roots();
        read_roots.extend([plugin_dir.to_path_buf(), cwd.clone()]);
        let policy = SandboxPolicy {
            mode: SandboxMode::WorkspaceWrite,
            writable_roots: vec![data.clone()],
            network: false,
            read_roots,
            temp_dir: Some(tmp.clone()),
        };
        let out = Executor::new(shell.clone())
            .run(&spec, &policy)
            .await
            .map_err(|e| e.to_string())?;
        if out.timed_out {
            return Err(format!("stopped after {} s", spec.timeout.as_secs()));
        }
        Ok::<_, String>((out.exit_code, out.stdout, out.stderr))
    }
    .await;
    let _ = std::fs::remove_file(&file);
    result
}

/// The run's tools with the `PreToolUse` hooks of the plugins the agent has on before each call.
pub fn wrap(
    app: Arc<App>,
    skills: Vec<String>,
    root: PathBuf,
    run: String,
    tools: Vec<Arc<dyn Tool>>,
) -> Vec<Arc<dyn Tool>> {
    tools
        .into_iter()
        .map(|inner| {
            Arc::new(Hooked {
                inner,
                app: app.clone(),
                skills: skills.clone(),
                root: root.clone(),
                run: run.clone(),
            }) as Arc<dyn Tool>
        })
        .collect()
}

struct Hooked {
    inner: Arc<dyn Tool>,
    app: Arc<App>,
    skills: Vec<String>,
    root: PathBuf,
    run: String,
}

#[async_trait]
impl Tool for Hooked {
    fn name(&self) -> &str {
        self.inner.name()
    }
    fn description(&self) -> &str {
        self.inner.description()
    }
    fn parameters(&self) -> Value {
        self.inner.parameters()
    }
    fn read_only(&self) -> bool {
        self.inner.read_only()
    }
    async fn call(&self, ctx: &ToolCtx, args: Value) -> Result<ToolOutput, ToolError> {
        let name = claude_tool_name(self.inner.name()).to_owned();
        let mut tool_input = args.clone();
        if let (Some(p), Value::Object(o)) = (args.get("path").cloned(), &mut tool_input) {
            o.entry("file_path").or_insert(p);
        }
        let runs = self
            .app
            .run_plugin_hooks(
                &self.skills,
                "PreToolUse",
                &name,
                json!({ "tool_name": name, "tool_input": tool_input }),
                &self.run,
                &self.root,
            )
            .await;
        if let Some((plugin, why)) = runs.iter().find_map(|r| match &r.verdict {
            Verdict::Refuse(w) => Some((r.note.plugin.clone(), w.clone())),
            Verdict::Go => None,
        }) {
            return Err(ToolError::new(
                "refused_by_plugin",
                format!("the plugin {plugin}'s hook refused this call: {why}"),
            ));
        }
        self.inner.call(ctx, args).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hook_says_what_claude_codes_conventions_say() {
        let (ok, ctx, v, _) = read_output("SessionStart", Some(0), "Use the skills first.\n", "");
        assert_eq!(
            (ok, ctx.as_deref(), v),
            (true, Some("Use the skills first."), Verdict::Go)
        );
        let j = r#"{"hookSpecificOutput":{"hookEventName":"SessionStart","additionalContext":"Ponytail mode on."}}"#;
        assert_eq!(
            read_output("SessionStart", Some(0), j, "").1.as_deref(),
            Some("Ponytail mode on.")
        );
        let (_, _, v, _) = read_output("PreToolUse", Some(2), "", "no rm -rf");
        assert_eq!(v, Verdict::Refuse("no rm -rf".into()));
        let j = r#"{"hookSpecificOutput":{"permissionDecision":"deny","permissionDecisionReason":"secrets"}}"#;
        assert_eq!(
            read_output("PreToolUse", Some(0), j, "").2,
            Verdict::Refuse("secrets".into())
        );
        // A tool's hook adds nothing to the instructions; a failing hook refuses nothing.
        assert_eq!(read_output("PreToolUse", Some(0), "text", "").1, None);
        let (ok, _, v, _) = read_output("SessionStart", Some(1), "", "boom");
        assert_eq!((ok, v), (false, Verdict::Go));
    }

    #[test]
    fn matchers_and_commands_follow_claude_code() {
        assert!(matches(&json!("startup|clear|compact"), "startup"));
        assert!(!matches(&json!("resume"), "startup"));
        assert!(matches(&Value::Null, "Bash") && matches(&json!("*"), "Bash"));
        assert!(matches(
            &json!("Edit|Write"),
            claude_tool_name("write_file")
        ));
        let hook = json!({"command": "\"${CLAUDE_PLUGIN_ROOT}/hooks/run\" start", "powershell": "& \"${CLAUDE_PLUGIN_ROOT}/hooks/run\" start"});
        let vars = [("CLAUDE_PLUGIN_ROOT", "/p".to_owned())];
        let bash = command_line(
            &Shell::Bash("/bin/bash".into()),
            &hook,
            &vars,
            Path::new("/t/in.json"),
        )
        .unwrap();
        assert_eq!(bash, "{ \"/p/hooks/run\" start\n} < '/t/in.json'");
        let ps = command_line(
            &Shell::PowerShell("pwsh".into()),
            &hook,
            &vars,
            Path::new("C:/t/in.json"),
        )
        .unwrap();
        assert_eq!(
            ps,
            "Get-Content -Raw -LiteralPath 'C:/t/in.json' | & \"/p/hooks/run\" start"
        );
    }
}
