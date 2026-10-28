//! What the CliBackend needs from one vendor CLI (ADR-56): how to start it headless with a JSON event stream, how to
//! point it at an Agenium-owned per-run home instead of the user's own configuration (ADR-30), how Agenium keeps
//! control of its permissions, and how to read its events.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use agenium_core::Effort;
use agenium_llm::Usage;
use serde_json::Value;

/// Everything a driver writes into the per-run home.
pub struct Setup<'a> {
    /// Fresh, empty directory owned by this run (deleted afterwards).
    pub home: &'a Path,
    /// Ticket worktree: the CLI's working directory and its only workspace.
    pub worktree: &'a Path,
    /// Agenium's MCP server for this run.
    pub mcp_name: &'a str,
    pub mcp_url: &'a str,
    pub mcp_authorization: &'a str,
    /// Agenium's permission endpoint for the CLI's pre-tool hook (same bearer token as the MCP server).
    pub hook_url: &'a str,
    /// Solo agents may start the CLI's own subagents; team members never do (the team is the delegation).
    pub subagents: bool,
    /// The CLI's model (empty = its default), for CLIs that take it in their configuration rather than as an
    /// argument.
    pub model: &'a str,
}

/// Context of a permission check.
pub struct CheckCtx<'a> {
    pub worktree: &'a Path,
    pub home: &'a Path,
    pub mcp_name: &'a str,
    /// See [`Setup::subagents`].
    pub subagents: bool,
    /// The run only reads (chat, review, inbox, plan): no file in the worktree may be written. A Lead's chat runs in
    /// the project's main checkout, which only the merge queue changes.
    pub read_only: bool,
}

/// Env names under which a hooked CLI's permission hook finds Agenium's endpoint and its per-run token.
pub(crate) const HOOK_URL_ENV: &str = "AGENIUM_HOOK_URL";
pub(crate) const HOOK_TOKEN_ENV: &str = "AGENIUM_HOOK_TOKEN";

/// Seconds Claude Code and agy wait for Agenium's permission hook ([`Driver::hook_timeout`]).
pub(crate) const HOOK_TIMEOUT: u64 = 900;

/// The hook script's own deadline for its call to Agenium: 100 s before the CLI's. A CLI that kills a slow hook goes
/// on with the tool (Claude Code and Grok fail open on a hook timeout), so the hook must end first, with a deny.
pub(crate) fn hook_call_secs(timeout: u64) -> u64 {
    timeout.saturating_sub(100)
}

/// When Agenium answers a hook at the latest (a deny when the decision, e.g. the user's approval, is not there): 100 s
/// before the hook's own call gives up, so the CLI gets Agenium's answer rather than a failed hook.
pub fn hook_deadline(d: &dyn Driver) -> std::time::Duration {
    std::time::Duration::from_secs(hook_call_secs(d.hook_timeout()).saturating_sub(100))
}

/// Whether a hooked CLI's own file tool may touch `p` (relative paths from the worktree). Only its scratch: the per-run
/// home, where the CLI keeps what it needs for itself (a long tool result saved to read back, its task notes), but
/// never Agenium's own files there (`protected`: the hook script, the configuration that names it, the login copy — a
/// model that rewrote the hook script to `exit 0` would skip every later check), nor a folder holding them (a search
/// of the whole home would read the login). The worktree and everything else are worked on through Agenium's tools
/// (ADR-129). `Err` is the reason given to the model.
pub(crate) fn file_access(p: &Path, cx: &CheckCtx, protected: &[PathBuf]) -> Result<(), String> {
    if protected
        .iter()
        .any(|f| inside(p, cx.worktree, f) || inside(f, cx.worktree, &normalize(p, cx.worktree)))
    {
        return Err(format!(
            "{} is Agenium's own file for this run (its permission hook, configuration or login)",
            p.display()
        ));
    }
    if inside(p, cx.worktree, cx.home) {
        return Ok(());
    }
    if inside(p, cx.worktree, cx.worktree) {
        return Err(format!(
            "{} is in the ticket worktree: work on it through the tools of the \"agenium\" MCP server ({AGENIUM_TOOLS}); \
             your own file tools reach only your scratch folder",
            p.display()
        ));
    }
    Err(format!("{} is outside the ticket worktree", p.display()))
}

/// A TOML basic string (JSON string escaping is valid TOML).
pub(crate) fn toml_str(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_default()
}

/// Refusal of a subagent tool in team mode.
pub const NO_SUBAGENTS: &str =
    "subagents are not used in team mode: the Lead splits the work into tickets; do this ticket yourself";

/// Agenium's tools a CLI works through instead of its own (ADR-129), as the refusal of one of its own names them.
pub const AGENIUM_TOOLS: &str =
    "read_file, list_dir, glob, grep and find_code to read; edit_file, multi_edit, \
apply_patch and write_file to change files; shell for every command; todo for your task list";

/// Refusal of a CLI's own file, search or shell tool (ADR-129): the work goes through Agenium's tools.
pub fn own_tool_off(name: &str) -> String {
    format!(
        "{name} is turned off in Agenium runs: use the tools of the \"agenium\" MCP server instead ({AGENIUM_TOOLS})"
    )
}

/// The tool and its input in a permission-hook payload, whichever CLI sent it: Claude Code and Grok
/// (`tool_name`/`tool_input`, Grok also `toolName`/`toolInput`), agy (`toolCall.name`/`args`), opencode
/// (`action`/`resources`).
pub(crate) fn hook_call(payload: &Value) -> (String, Value) {
    let pick = |names: &[&str]| -> Value {
        names
            .iter()
            .map(|n| n.split('.').fold(payload, |v, k| &v[k]))
            .find(|v| !v.is_null())
            .cloned()
            .unwrap_or(Value::Null)
    };
    let name = pick(&["tool_name", "toolName", "toolCall.name", "action"]);
    let input = pick(&["tool_input", "toolInput", "toolCall.args", "resources"]);
    (name.as_str().unwrap_or("").to_owned(), input)
}

/// What Agenium decides about one tool call of a `Control::Hook` CLI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Check {
    Allow,
    Deny(String),
    /// A shell command: the run's command gate decides.
    Command(String),
}

/// How Agenium keeps control of what the CLI does while the CLI runs in its own YOLO mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control {
    /// The CLI process runs inside Agenium's sandbox (worktree and per-run home writable, network for the vendor
    /// API). Requires a working sandbox backend (fail-closed).
    Sandbox,
    /// The CLI runs on the host (its login cannot be reached from a container); its pre-tool hook, written into
    /// the per-run home, asks Agenium before every tool call and fails closed when Agenium cannot be reached.
    Hook,
}

/// How turns reach the CLI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Turns {
    /// One process for the run; each stdin line is a user message.
    Stdin,
    /// One process per turn: the message is written to stdin, then stdin is closed; later turns resume the CLI's
    /// conversation of this run.
    ProcessPerTurn,
}

/// One thing the CLI reported.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// The CLI's own conversation id (used only to continue the same run's conversation).
    Session(String),
    /// Assistant text (a delta or a whole message).
    Text(String),
    /// A tool step of the CLI; `done` = finished.
    Tool {
        id: String,
        name: String,
        input: Value,
        done: bool,
    },
    /// Steps of a subagent the CLI started (Claude Code's `parent_tool_use_id`): `parent` is the id of the tool step
    /// that started it (ADR-129: the run's log shows them as the subagent's).
    Subagent { parent: String, events: Vec<Event> },
    /// Tokens of one model step (CLIs that report usage per step rather than per turn).
    Usage(Usage),
    /// The vendor's subscription usage, as the CLI reported it during the run.
    Limits(agenium_llm::vendor_usage::VendorUsage),
    /// The turn ended. `denied` lists the permission kinds the CLI refused itself.
    TurnEnd {
        usage: Usage,
        response: String,
        denied: Vec<String>,
        error: Option<String>,
    },
}

pub trait Driver: Send + Sync {
    fn control(&self) -> Control;
    fn turns(&self) -> Turns;
    /// Arguments of a headless run; `effort` = the agent's reasoning effort (`None`: the CLI's own setting), `home` =
    /// the per-run home, `resume` = the CLI's conversation id for a later turn (`ProcessPerTurn`), `subagents` =
    /// whether the CLI may start its own subagents (Solo only).
    fn args(
        &self,
        model: &str,
        effort: Option<&Effort>,
        home: &Path,
        resume: Option<&str>,
        subagents: bool,
    ) -> Vec<String>;
    /// Writes the per-run configuration into `setup.home` and returns the environment that makes the CLI use it.
    fn prepare(&self, setup: &Setup) -> std::io::Result<BTreeMap<String, String>>;
    /// The binary to start, given what was found on PATH (e.g. the real binary behind an npm shim).
    fn program(&self, found: &Path) -> PathBuf {
        found.to_path_buf()
    }
    /// The CLI runs as a local server driven through its HTTP API by an in-process bridge (opencode) instead of
    /// a JSON stream on stdio.
    fn served(&self) -> bool {
        false
    }
    /// Called after the run (the process has ended): e.g. hand a refreshed login back to the user's CLI.
    fn finish(&self, _setup: &Setup) -> std::io::Result<()> {
        Ok(())
    }
    /// Folders the CLI must read inside the sandbox besides the toolchains (its installation).
    fn read_roots(&self, _program: &Path) -> Vec<PathBuf> {
        vec![]
    }
    /// What goes to stdin for one user message.
    fn user_input(&self, text: &str) -> String;
    /// `Turns::ProcessPerTurn` CLIs that take the turn's message from a file instead of stdin (grok): the file the
    /// message is written to before the process starts (its path is in `args`).
    fn prompt_file(&self, _home: &Path) -> Option<PathBuf> {
        None
    }
    /// Events of one stdout line.
    fn parse(&self, line: &Value) -> Vec<Event>;
    /// The shell command a tool step runs, when it runs one.
    fn command(&self, tool: &str, input: &Value) -> Option<String>;
    /// Whether a tool step is the CLI calling a tool of Agenium's own MCP server (Agenium logs those itself).
    fn is_agenium_call(&self, tool: &str, input: &Value, server: &str) -> bool;
    /// Whether a tool step starts one of the CLI's own subagents (Solo agents only).
    fn starts_subagent(&self, _tool: &str) -> bool {
        false
    }
    /// Whether a tool step only runs other tools (opencode's code-mode `execute`): it is not checked itself, the
    /// tools it calls are (Agenium's over MCP, the CLI's own through its hook), so it is logged as run.
    fn runs_other_tools(&self, _tool: &str) -> bool {
        false
    }
    /// `Control::Hook`: seconds the CLI waits for its permission hook (see [`hook_deadline`]).
    fn hook_timeout(&self) -> u64 {
        HOOK_TIMEOUT
    }
    /// `Control::Hook`: Agenium's decision about the tool call in a hook payload.
    fn check(&self, _payload: &Value, _ctx: &CheckCtx) -> Check {
        Check::Deny("this CLI has no permission hook".into())
    }
    /// `Control::Hook`: the hook's answer for a decision.
    fn hook_answer(&self, allow: bool, reason: &str) -> Value {
        serde_json::json!({"decision": if allow { "allow" } else { "deny" }, "reason": reason})
    }
    /// `Control::Hook`: `answer` (an allow) carrying `text` to the model as added context, when the CLI's hook can
    /// do that (`None`: it cannot; the text waits for the next Agenium tool or turn).
    fn hook_with_context(&self, _answer: Value, _text: &str) -> Option<Value> {
        None
    }
}

/// Copies the user's login file into the per-run home, with a pristine copy to compare against afterwards.
pub fn copy_login(user_file: &Path, home_file: &Path) -> std::io::Result<()> {
    let login = std::fs::read(user_file).map_err(|e| {
        std::io::Error::other(format!("not logged in ({}): {e}", user_file.display()))
    })?;
    std::fs::write(home_file, &login)?;
    std::fs::write(original(home_file), &login)
}

/// The pristine copy of a login file copied into a per-run home.
pub(crate) fn original(home_file: &Path) -> PathBuf {
    let mut name = home_file.file_name().unwrap_or_default().to_os_string();
    name.push(".agenium-original");
    home_file.with_file_name(name)
}

/// A fresh folder inside `scratch` for one usage or model-list probe of a CLI, deleted when dropped — on every path,
/// errors included — with the copy of the user's login it holds. Probes used to share `scratch/<legacy>` and leave the
/// login (OAuth refresh tokens) there in clear; that folder is removed too.
pub(crate) fn probe_dir(scratch: &Path, legacy: &str) -> Result<tempfile::TempDir, String> {
    let _ = std::fs::remove_dir_all(scratch.join(legacy));
    std::fs::create_dir_all(scratch).map_err(|e| e.to_string())?;
    tempfile::Builder::new()
        .prefix("probe-")
        .tempdir_in(scratch)
        .map_err(|e| e.to_string())
}

/// After the run: when the CLI refreshed its login inside the per-run home, the new login goes back to the user's
/// file so their own CLI stays logged in (refresh tokens are single-use) — only when the user's file did not change
/// meanwhile (then theirs wins).
pub fn hand_back_login(user_file: &Path, home_file: &Path) -> std::io::Result<()> {
    let (Ok(now), Ok(before)) = (std::fs::read(home_file), std::fs::read(original(home_file)))
    else {
        return Ok(());
    };
    if now == before || now.is_empty() {
        return Ok(());
    }
    if std::fs::read(user_file)? != before {
        tracing::warn!(
            file = %user_file.display(),
            "login changed both inside and outside the run; keeping the user's"
        );
        return Ok(());
    }
    let tmp = original(user_file).with_extension("agenium-tmp");
    std::fs::write(&tmp, &now)?;
    std::fs::rename(&tmp, user_file)
}

/// Lexically normalised absolute path (no `.`/`..`), relative paths taken from `base`.
pub fn normalize(p: &Path, base: &Path) -> PathBuf {
    let joined = if p.is_absolute() {
        p.to_path_buf()
    } else {
        base.join(p)
    };
    let mut out = PathBuf::new();
    for c in joined.components() {
        match c {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Whether `p` (relative to `base`) lies inside `root` (case-insensitive on Windows), under either name of the root
/// and with links resolved: the confinement rule of the API tools (`agenium_tools::paths::confined`).
pub fn inside(p: &Path, base: &Path, root: &Path) -> bool {
    let (p, root) = (normalize(p, base), normalize(root, base));
    agenium_tools::paths::confined(&p, &root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_inside_a_root() {
        let root = if cfg!(windows) {
            Path::new(r"C:\wt")
        } else {
            Path::new("/wt")
        };
        assert!(inside(Path::new("a/b.txt"), root, root));
        assert!(inside(&root.join("x"), root, root));
        assert!(!inside(Path::new("../wt2/x"), root, root));
        assert!(!inside(Path::new("../../etc/passwd"), root, root));
        if cfg!(windows) {
            assert!(inside(Path::new(r"c:\WT\sub\f"), root, root));
            assert!(!inside(Path::new(r"C:\wt2\f"), root, root));
        }
    }

    /// macOS runs of 2026-10-25/26 (stream_run): the worktree was /var/folders/…/wt and the CLI wrote under its
    /// getcwd(), /private/var/folders/…/wt; the hook refused it. On Windows the canonical name is `\\?\D:\…`.
    #[test]
    fn the_canonical_name_of_the_worktree_is_inside_it() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("wt");
        std::fs::create_dir(&root).unwrap();
        let canon = std::fs::canonicalize(&root).unwrap();
        assert!(inside(&canon.join("a.txt"), &root, &root));
        assert!(!inside(&canon.join("..").join("escape.txt"), &root, &root));
    }

    #[test]
    fn refreshed_login_goes_back_only_when_the_user_file_is_unchanged() {
        let home = tempfile::tempdir().unwrap();
        let user = tempfile::tempdir().unwrap();
        let (u, h) = (user.path().join("auth.json"), home.path().join("auth.json"));
        std::fs::write(&u, "old").unwrap();
        copy_login(&u, &h).unwrap();
        hand_back_login(&u, &h).unwrap();
        assert_eq!(
            std::fs::read_to_string(&u).unwrap(),
            "old",
            "nothing refreshed"
        );
        std::fs::write(&h, "new").unwrap();
        hand_back_login(&u, &h).unwrap();
        assert_eq!(std::fs::read_to_string(&u).unwrap(), "new");
        // The user logged in again meanwhile: theirs wins.
        copy_login(&u, &h).unwrap();
        std::fs::write(&u, "other").unwrap();
        std::fs::write(&h, "newer").unwrap();
        hand_back_login(&u, &h).unwrap();
        assert_eq!(std::fs::read_to_string(&u).unwrap(), "other");
    }
}
