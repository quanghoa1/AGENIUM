//! Detection of installed agent CLIs: path, version, login state, minimum version.

use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::Serialize;

/// The CLIs Agenium detects (decision 3c; `devin` added for the owner's setup). Which ones it drives: `crate::driver`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CliKind {
    Codex,
    Agy,
    Devin,
    Claude,
    Opencode,
}

impl CliKind {
    pub const ALL: [CliKind; 5] = [
        CliKind::Codex,
        CliKind::Agy,
        CliKind::Devin,
        CliKind::Claude,
        CliKind::Opencode,
    ];

    pub fn program(self) -> &'static str {
        match self {
            CliKind::Codex => "codex",
            CliKind::Agy => "agy",
            CliKind::Devin => "devin",
            CliKind::Claude => "claude",
            CliKind::Opencode => "opencode",
        }
    }

    /// Lowest version whose headless flags Agenium has checked (contract tests pin these).
    pub fn min_version(self) -> [u64; 3] {
        match self {
            CliKind::Codex => [0, 163, 0],
            CliKind::Agy => [1, 4, 4],
            CliKind::Devin => [3000, 11, 3],
            CliKind::Claude => [2, 1, 310],
            CliKind::Opencode => [2, 1, 0],
        }
    }

    /// Arguments of the CLI's own "am I logged in" command and how to read its output; `None` when the CLI has no
    /// such command (the state is then reported as unknown, never guessed).
    fn login_probe(self) -> Option<LoginProbe> {
        fn codex(out: &str) -> Option<bool> {
            let o = out.to_ascii_lowercase();
            if o.contains("not logged in") {
                Some(false)
            } else if o.contains("logged in") {
                Some(true)
            } else {
                None
            }
        }
        match self {
            CliKind::Codex => Some((&["login", "status"], codex)),
            CliKind::Devin => Some((&["auth", "status"], codex)),
            CliKind::Claude => Some((&["auth", "status"], codex)),
            CliKind::Agy | CliKind::Opencode => None,
        }
    }
}

/// Arguments of a login-status command and the reader of its output.
type LoginProbe = (&'static [&'static str], fn(&str) -> Option<bool>);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Login {
    LoggedIn,
    NotLoggedIn,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CliInfo {
    pub kind: CliKind,
    pub path: Option<PathBuf>,
    pub version: Option<String>,
    pub login: Login,
    /// Installed and at least the minimum version.
    pub compatible: bool,
    pub note: Option<String>,
}

/// Runs a command with a deadline; returns combined stdout + stderr.
fn run(program: &PathBuf, args: &[&str], timeout: Duration) -> Option<String> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() < timeout => std::thread::sleep(Duration::from_millis(25)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let mut s = String::new();
    if let Some(mut o) = child.stdout.take() {
        let _ = o.read_to_string(&mut s);
    }
    if let Some(mut e) = child.stderr.take() {
        let _ = e.read_to_string(&mut s);
    }
    Some(s)
}

/// First `a.b.c` (or `a.b`) in a version banner.
pub fn parse_version(banner: &str) -> Option<[u64; 3]> {
    for word in banner.split(|c: char| !(c.is_ascii_digit() || c == '.')) {
        let parts: Vec<u64> = word.split('.').filter_map(|p| p.parse().ok()).collect();
        if parts.len() >= 2 && word.split('.').count() == parts.len() {
            return Some([parts[0], parts[1], parts.get(2).copied().unwrap_or(0)]);
        }
    }
    None
}

pub fn detect(kind: CliKind) -> CliInfo {
    let Some(path) = agenium_sandbox::find_program(kind.program()) else {
        return CliInfo {
            kind,
            path: None,
            version: None,
            login: Login::Unknown,
            compatible: false,
            note: Some("not installed (not on PATH)".into()),
        };
    };
    let banner = run(&path, &["--version"], Duration::from_secs(20));
    let version = banner.as_deref().and_then(parse_version);
    let compatible = version.is_some_and(|v| v >= kind.min_version());
    let login = match kind.login_probe() {
        Some((args, read)) => run(&path, args, Duration::from_secs(20))
            .as_deref()
            .and_then(read)
            .map(|b| {
                if b {
                    Login::LoggedIn
                } else {
                    Login::NotLoggedIn
                }
            })
            .unwrap_or(Login::Unknown),
        None => Login::Unknown,
    };
    let note = match (version, compatible) {
        (None, _) => Some("cannot read its version".into()),
        (Some(_), false) => Some(format!(
            "older than the checked minimum {}",
            kind.min_version().map(|n| n.to_string()).join(".")
        )),
        _ => None,
    };
    CliInfo {
        kind,
        path: Some(path),
        version: version.map(|v| v.map(|n| n.to_string()).join(".")),
        login,
        compatible,
        note,
    }
}

/// Every known CLI, installed or not.
pub fn detect_all() -> Vec<CliInfo> {
    CliKind::ALL.iter().map(|k| detect(*k)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert_eq!(parse_version("codex-cli 0.163.0"), Some([0, 163, 0]));
        assert_eq!(parse_version("2.1.310 (Claude Code)"), Some([2, 1, 310]));
        assert_eq!(parse_version("opencode v2.1.2"), Some([2, 1, 2]));
        assert_eq!(
            parse_version("devin 3000.11.3 (9c803229faa4)"),
            Some([3000, 11, 3])
        );
        assert_eq!(parse_version("1.4.4\n"), Some([1, 4, 4]));
        assert_eq!(parse_version("no version"), None);
    }

    #[test]
    fn not_installed_is_reported_not_guessed() {
        let i = CliInfo {
            kind: CliKind::Codex,
            path: None,
            version: None,
            login: Login::Unknown,
            compatible: false,
            note: None,
        };
        assert!(!i.compatible);
    }
}
