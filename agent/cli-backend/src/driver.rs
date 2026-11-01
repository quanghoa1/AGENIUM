//! What the CliBackend needs from one vendor CLI (ADR-56): how to start it headless with a JSON event stream, how to
//! point it at an Agenium-owned per-run home instead of the user's own configuration (ADR-30), how Agenium keeps
//! control of its permissions, and how to read its events.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use agenium_core::Effort;
use agenium_llm::Usage;
use serde_json::Value;

pub(crate) use crate::tool_paths::named_paths;

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
    /// How long the run may last: a login lent without its refresh token must outlive it (ADR-137).
    pub run_for: std::time::Duration,
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

/// Seconds a CLI waits for one of Agenium's tools over MCP: longer than any of them takes (a command 10 minutes at
/// most, the user's approval of one 30 minutes). A CLI's own default is shorter (Claude Code gave up on an 80-second
/// command, told the model it failed and the model ran it again, ADR-56).
pub(crate) const MCP_CALL_TIMEOUT: u64 = 3600;

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
        let first = first_under(&normalize(p, cx.worktree), &normalize(cx.home, cx.worktree));
        if let Some(f) = first.as_deref().filter(|f| {
            f.starts_with('.') && !HOME_DOT_DIRS.iter().any(|d| d.eq_ignore_ascii_case(f))
        }) {
            return Err(format!(
                "{f} in your scratch folder is a file other programs read as the user's own (the folder is their \
                 home): keep your notes in a folder of it such as notes/"
            ));
        }
        if let Some(f) = first.as_deref().filter(|f| home_config_name(f)) {
            return Err(format!(
                "{f} in your scratch folder is configuration your CLI loads in later runs: keep your notes in a \
                 folder of it such as notes/"
            ));
        }
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

/// Hidden entries of the per-run home a CLI's own tools may use: agy keeps its task notes in `.gemini`. Any other
/// (`.gitconfig`, `.npmrc`, `.ssh`, `.config`) is read by other programs as the user's own, the home being their
/// `HOME`: one a model planted could run commands outside every gate (git's `core.fsmonitor`).
const HOME_DOT_DIRS: [&str; 1] = [".gemini"];

/// Names at the top of a CLI's per-run home (lowercase) that a CLI loads as the user's own configuration or memory —
/// instructions (`CLAUDE.md`), subagents, commands, skills, hooks, plugins, settings. The model's file tools may not
/// write them, and a kept session leaves them out: written there, they were kept with the ticket's session and loaded
/// by every later run of it, on the host and outside the hook (audit M6, 2026-10-31). Agenium writes its own files there
/// again in every run (`Driver::prepare`).
pub(crate) const HOME_CONFIG_NAMES: [&str; 19] = [
    "claude.md",
    "agents.md",
    "gemini.md",
    "agents",
    "commands",
    "skills",
    "hooks",
    "plugins",
    "output-styles",
    "rules",
    "prompts",
    "memory",
    "settings.json",
    "settings.local.json",
    "config.toml",
    "config.json",
    "mcp.json",
    "keybindings.json",
    "statusline.sh",
];

/// Whether `name`, at the top of a CLI's per-run home, is one the CLI loads as configuration ([`HOME_CONFIG_NAMES`]).
pub(crate) fn home_config_name(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    HOME_CONFIG_NAMES.contains(&n.as_str())
}

/// The first component of `p` below `root` (both normalised), compared without case as on Windows.
fn first_under(p: &Path, root: &Path) -> Option<String> {
    let mut pc = p.components();
    for rc in root.components() {
        let a = pc.next()?;
        if !a
            .as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case(&rc.as_os_str().to_string_lossy())
        {
            return None;
        }
    }
    pc.next()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
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

/// What Agenium decides about one tool call of a `Control::Hook` CLI. A CLI's own shell commands are refused, not put
/// to a command gate (ADR-129: commands run through Agenium's `shell`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Check {
    Allow,
    Deny(String),
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
    /// The tokens in the model's context at one request (ADR-155: the context gauge), from a CLI that reports each
    /// request's usage beside its turn's total.
    Context(u64),
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
    /// Called every few seconds while the run goes on and once after it: keeps the run's copy of the login and the
    /// user's the same (`sync_login`).
    fn finish(&self, _setup: &Setup) -> std::io::Result<()> {
        Ok(())
    }
    /// Folders the CLI makes outside its home for the session `id` it ran in `worktree`, innermost first: removed after
    /// the run when they are empty.
    fn session_leftovers(&self, _worktree: &Path, _id: &str) -> Vec<PathBuf> {
        vec![]
    }
    /// The CLI resumes a session of an earlier run from its id (`args(.., resume, ..)`) once that run's home is back
    /// (ADR-134): the same ticket goes on in the CLI's own session, not in a new one shown the conversation so far.
    fn resumes_across_runs(&self) -> bool {
        false
    }
    /// After a login error: a run lent the login without its refresh token takes the whole login when the access token
    /// has run out and no other run holds the refresh token (ADR-137). Whether it did.
    fn promote_login(&self, _setup: &Setup) -> bool {
        false
    }
    /// The login file the per-run home holds (a copy of the user's), never kept with a session.
    fn login_file(&self) -> Option<&'static str> {
        None
    }
    /// Whether an entry at the top of the per-run home is left out of a kept session: one that holds the home's own
    /// path, which the next run's home does not have.
    fn not_kept(&self, _name: &str) -> bool {
        false
    }
    /// Where its per-run homes are made, when not in the temp folder: beside the user's login, on its file system, so the
    /// login can be linked there (`link_login`; a hard link cannot cross file systems, and `/tmp` is often a tmpfs).
    fn home_base(&self) -> Option<PathBuf> {
        None
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

pub use crate::login::{copy_login, link_login, sync_login};
pub(crate) use crate::login::{original, probe_dir};

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
    use serde_json::json;

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

    /// Path review of 2026-10-29: a path under a name no driver listed, in a list, as `FILE:///` or with `%2F..` in a
    /// URI, or as the base of a search pattern, is read and checked; the text a call carries is not.
    #[test]
    fn every_argument_that_names_a_file_is_read() {
        let got = |v: Value| named_paths(&v);
        assert_eq!(
            got(json!({"TargetDirectories": ["D:/elsewhere"], "Query": "a/b"})).unwrap(),
            ["D:/elsewhere"]
        );
        assert_eq!(got(json!({"Uri": "FILE:///C:/x/y"})).unwrap(), ["C:/x/y"]);
        assert_eq!(
            got(json!({"uri": "file:///home/u/..%2F..%2Fsecret"})).unwrap(),
            ["/home/u/../../secret"]
        );
        assert_eq!(
            got(json!({"target": "C:\\outside", "nested": {"more": ["/etc/x"]}})).unwrap(),
            ["/etc/x", "C:\\outside"]
        );
        assert_eq!(
            got(json!({"file_path": "notes.md", "content": "see a/b and C:\\d", "old_string": "x/y"})).unwrap(),
            ["notes.md"]
        );
        assert!(got(json!({"Uri": "agenium://schemas/submit_report"}))
            .unwrap()
            .is_empty());
        for escaping in [
            "C:/Users/me/.ssh/*",
            "/etc/*",
            "../x/*",
            "~/.ssh/*",
            "\\\\server\\share\\*",
        ] {
            assert!(
                got(json!({"pattern": escaping, "path": "notes"})).is_err(),
                "{escaping}"
            );
        }
        assert!(got(json!({"pattern": "fn main", "glob": "**/*.rs", "path": "notes"})).is_ok());
    }

    /// A CLI's own tools keep to its scratch folder, but not to the files there that other programs read as the
    /// user's (the folder is their home): a planted `.gitconfig` ran a command through git (path review of 2026-10-29).
    #[test]
    fn the_scratch_folder_keeps_no_dotfiles_but_the_cli_state() {
        let tmp = tempfile::tempdir().unwrap();
        let (wt, home) = (tmp.path().join("wt"), tmp.path().join("home"));
        let cx = CheckCtx {
            worktree: &wt,
            home: &home,
            mcp_name: "agenium",
            subagents: false,
            read_only: false,
        };
        assert!(file_access(&home.join("notes/plan.md"), &cx, &[]).is_ok());
        assert!(file_access(&home.join(".gemini/antigravity/brain/c1/task.md"), &cx, &[]).is_ok());
        for planted in [
            ".gitconfig",
            ".npmrc",
            ".ssh/config",
            ".config/git/config",
            ".GITCONFIG",
        ] {
            let e = file_access(&home.join(planted), &cx, &[]).unwrap_err();
            assert!(e.contains("other programs read"), "{planted}: {e}");
        }
        let git = crate::run::isolated_home(&home, Default::default());
        assert_eq!(git["GIT_CONFIG_NOSYSTEM"], "1");
        assert!(matches!(
            git["GIT_CONFIG_GLOBAL"].as_str(),
            "NUL" | "/dev/null"
        ));
    }

    /// ADR-136: the run's login is the user's own file (a hard link): what the CLI writes there (a refreshed token) is
    /// in the user's file at once, and the copy-and-sync fallback is not used.
    #[test]
    fn a_runs_login_is_the_users_own_file() {
        let tmp = tempfile::tempdir().unwrap();
        let (user, home) = (tmp.path().join("auth.json"), tmp.path().join("home"));
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(&user, r#"{"refresh":"r1"}"#).unwrap();
        let run = home.join("auth.json");
        std::fs::write(&run, "stale copy of an earlier run").unwrap();
        assert!(link_login(&user, &run).unwrap(), "linked");
        assert_eq!(
            std::fs::read_to_string(&run).unwrap(),
            r#"{"refresh":"r1"}"#
        );
        // Codex writes its login in place: the refresh reaches the user's file, and no original is kept to sync.
        std::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&run)
            .and_then(|mut f| std::io::Write::write_all(&mut f, br#"{"refresh":"r2"}"#))
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(&user).unwrap(),
            r#"{"refresh":"r2"}"#
        );
        assert!(!original(&run).exists());
        sync_login(&user, &run).unwrap();
        // Codex stopped while writing a refresh: the user's file is put back whole.
        crate::login::guard_linked(&user, &run).unwrap();
        std::fs::write(&user, "").unwrap();
        crate::login::guard_linked(&user, &run).unwrap();
        assert_eq!(
            std::fs::read_to_string(&user).unwrap(),
            r#"{"refresh":"r2"}"#
        );
        assert_eq!(
            std::fs::read_to_string(&run).unwrap(),
            r#"{"refresh":"r2"}"#,
            "still one file"
        );
        // Audit M13 (2026-10-31): the model writes the login (the run's home is writable) with something that parses
        // but is not this login: put back too. A refresh of the same shape stays.
        let write = |text: &str| {
            std::fs::OpenOptions::new()
                .write(true)
                .truncate(true)
                .open(&run)
                .and_then(|mut f| std::io::Write::write_all(&mut f, text.as_bytes()))
                .unwrap()
        };
        write("{}");
        crate::login::guard_linked(&user, &run).unwrap();
        assert_eq!(
            std::fs::read_to_string(&user).unwrap(),
            r#"{"refresh":"r2"}"#
        );
        write(r#"{"refresh":"r3"}"#);
        crate::login::guard_linked(&user, &run).unwrap();
        assert_eq!(
            std::fs::read_to_string(&user).unwrap(),
            r#"{"refresh":"r3"}"#,
            "a refresh stays"
        );
        // The run's home goes: the user's login stays.
        std::fs::remove_dir_all(&home).unwrap();
        assert_eq!(
            std::fs::read_to_string(&user).unwrap(),
            r#"{"refresh":"r3"}"#
        );
    }

    /// One CLI account in several runs at once: a run's refreshed login reaches the user's file at once, a login
    /// refreshed elsewhere reaches the other runs' copies, the user's wins a clash, and half a login moves nowhere.
    #[test]
    fn logins_stay_the_same_across_runs_and_the_users_cli() {
        let user = tempfile::tempdir().unwrap();
        let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let u = user.path().join("auth.json");
        let (ha, hb) = (a.path().join("auth.json"), b.path().join("auth.json"));
        let read = |p: &Path| std::fs::read_to_string(p).unwrap();
        std::fs::write(&u, r#"{"refresh": "r1"}"#).unwrap();
        copy_login(&u, &ha).unwrap();
        copy_login(&u, &hb).unwrap();
        sync_login(&u, &ha).unwrap();
        assert_eq!(read(&u), r#"{"refresh": "r1"}"#, "nothing refreshed");
        // Run A refreshes (r1 is used up): the user's file has r2 at once, and run B's copy gets it at its next sync.
        std::fs::write(&ha, r#"{"refresh": "r2"}"#).unwrap();
        sync_login(&u, &ha).unwrap();
        assert_eq!(read(&u), r#"{"refresh": "r2"}"#);
        sync_login(&u, &hb).unwrap();
        assert_eq!(
            read(&hb),
            r#"{"refresh": "r2"}"#,
            "run B no longer holds r1"
        );
        sync_login(&u, &ha).unwrap();
        assert_eq!(read(&ha), r#"{"refresh": "r2"}"#, "run A is settled");
        // The user logged in again while run B refreshed: theirs wins, and B's copy takes it.
        std::fs::write(&u, r#"{"refresh": "user"}"#).unwrap();
        std::fs::write(&hb, r#"{"refresh": "r3"}"#).unwrap();
        sync_login(&u, &hb).unwrap();
        assert_eq!(read(&u), r#"{"refresh": "user"}"#);
        sync_login(&u, &hb).unwrap();
        assert_eq!(read(&hb), r#"{"refresh": "user"}"#);
        // Run A takes the user's login, then half a login (the CLI stopped while writing it) is handed nowhere; whole,
        // it goes.
        sync_login(&u, &ha).unwrap();
        assert_eq!(read(&ha), r#"{"refresh": "user"}"#);
        std::fs::write(&ha, r#"{"refresh": "#).unwrap();
        sync_login(&u, &ha).unwrap();
        assert_eq!(read(&u), r#"{"refresh": "user"}"#);
        std::fs::write(&ha, r#"{"refresh": "r4"}"#).unwrap();
        sync_login(&u, &ha).unwrap();
        assert_eq!(read(&u), r#"{"refresh": "r4"}"#);
        let left: Vec<_> = std::fs::read_dir(user.path())
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert_eq!(left.len(), 1, "no temporary file left: {left:?}");
    }
}
