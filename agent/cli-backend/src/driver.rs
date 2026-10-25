//! What the CliBackend needs from one vendor CLI (ADR-56): how to start it headless with a JSON event stream, how to
//! point it at an Agenium-owned per-run home instead of the user's own configuration (ADR-30), how Agenium keeps
//! control of its permissions, and how to read its events.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

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
}

/// Refusal of a subagent tool in team mode.
pub const NO_SUBAGENTS: &str =
    "subagents are not used in team mode: the Lead splits the work into tickets; do this ticket yourself";

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
    /// Arguments of a headless run; `home` = the per-run home, `resume` = the CLI's conversation id for a later
    /// turn (`ProcessPerTurn`), `subagents` = whether the CLI may start its own subagents (Solo only).
    fn args(&self, model: &str, home: &Path, resume: Option<&str>, subagents: bool) -> Vec<String>;
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
    /// Events of one stdout line.
    fn parse(&self, line: &Value) -> Vec<Event>;
    /// The shell command a tool step runs, when it runs one.
    fn command(&self, tool: &str, input: &Value) -> Option<String>;
    /// Whether a tool step is the CLI calling a tool of Agenium's own MCP server (Agenium logs those itself).
    fn is_agenium_call(&self, tool: &str, input: &Value, server: &str) -> bool;
    /// `Control::Hook`: Agenium's decision about the tool call in a hook payload.
    fn check(&self, _payload: &Value, _ctx: &CheckCtx) -> Check {
        Check::Deny("this CLI has no permission hook".into())
    }
    /// `Control::Hook`: the hook's answer for a decision.
    fn hook_answer(&self, allow: bool, reason: &str) -> Value {
        serde_json::json!({"decision": if allow { "allow" } else { "deny" }, "reason": reason})
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

fn original(home_file: &Path) -> PathBuf {
    let mut name = home_file.file_name().unwrap_or_default().to_os_string();
    name.push(".agenium-original");
    home_file.with_file_name(name)
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

/// Whether `p` (relative to `base`) lies inside `root` (case-insensitive on Windows).
pub fn inside(p: &Path, base: &Path, root: &Path) -> bool {
    let (p, root) = (normalize(p, base), normalize(root, base));
    if cfg!(windows) {
        let norm = |x: &Path| x.to_string_lossy().to_lowercase().replace('/', "\\");
        let (a, b) = (norm(&p), norm(&root));
        let b = b.trim_end_matches('\\').to_owned();
        a == b || a.starts_with(&format!("{b}\\"))
    } else {
        p.starts_with(&root)
    }
}

/// Whether `command` is allowed by the project allowlist (exact entries; `prefix *` entries match the prefix when
/// the command has no shell control characters — the same rule as the API backend's command gate).
pub fn permitted(command: &str, allowed: &[String]) -> bool {
    let c = command.trim();
    let plain = !c.contains([';', '&', '|', '`', '$', '>', '<', '\n', '(', ')']);
    allowed.iter().any(|e| match e.strip_suffix('*') {
        Some(prefix) => plain && c.starts_with(prefix),
        None => c == e.trim(),
    })
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

    #[test]
    fn allowlist_rules() {
        let allowed = vec!["cargo test".to_owned(), "npm run *".to_owned()];
        assert!(permitted("cargo test", &allowed));
        assert!(!permitted("cargo test; curl x", &allowed));
        assert!(permitted("npm run build", &allowed));
        assert!(!permitted("npm run build && curl x", &allowed));
        assert!(!permitted("git status", &allowed));
    }
}
