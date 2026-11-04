//! Updating an agent CLI on the user's request (Providers & CLIs): the version installed, the newest one, the one
//! Agenium last verified end to end, and the CLI's own updater run (never Agenium installing over it: the CLI is the
//! user's, set up their way). A version newer than the verified one may change the headless flags or the server API
//! Agenium drives (opencode changed how it loads its model list), so it is said, and the run after an update is checked.

use std::path::Path;
use std::time::Duration;

use serde::Serialize;

use crate::detect::{parse_version, CliKind};

/// An update lasts at most this long.
const UPDATE_TIMEOUT: Duration = Duration::from_secs(10 * 60);

impl CliKind {
    /// The version Agenium last checked end to end (the stress run of 2026-11-01: chat turns one after the other and
    /// at once, each in a fresh per-run home).
    pub fn verified_version(self) -> Option<&'static str> {
        match self {
            CliKind::Codex => Some("0.163.0"),
            CliKind::Agy => Some("1.4.12"),
            CliKind::Claude => Some("2.1.316"),
            CliKind::Opencode => Some("2.1.2"),
            CliKind::Grok => Some("1.0.41"),
            CliKind::Devin => Some("3000.11.3"),
        }
    }

    /// The npm package whose `latest` is the CLI's newest release (its native installers follow the same versions).
    fn npm_package(self) -> Option<&'static str> {
        match self {
            CliKind::Claude => Some("@anthropic-ai/claude-code"),
            CliKind::Codex => Some("@openai/codex"),
            CliKind::Opencode => Some("@opencode/cli"),
            _ => None,
        }
    }

    /// The CLI's own updater: to the newest version, or to `version` where it can install a given one.
    pub fn update_args(self, version: Option<&str>) -> Option<Vec<String>> {
        let v = |a: &[&str]| Some(a.iter().map(|s| (*s).to_owned()).collect());
        match (self, version) {
            (CliKind::Claude, None) => v(&["update"]),
            (CliKind::Claude, Some(x)) => v(&["install", x]),
            (CliKind::Codex, None) => v(&["update"]),
            (CliKind::Agy, None) => v(&["update"]),
            (CliKind::Opencode, None) => v(&["upgrade"]),
            (CliKind::Opencode, Some(x)) => v(&["upgrade", x]),
            (CliKind::Grok, None) => v(&["update"]),
            (CliKind::Grok, Some(x)) => v(&["update", "--version", x]),
            (CliKind::Devin, None) => v(&["update"]),
            _ => None,
        }
    }
}

/// Whether version `a` is newer than `b` (both as the CLIs print them).
pub fn newer(a: &str, b: &str) -> bool {
    match (parse_version(a), parse_version(b)) {
        (Some(x), Some(y)) => x > y,
        _ => false,
    }
}

/// A version as the updaters take it (it goes on their command line).
pub fn valid_version(v: &str) -> bool {
    !v.is_empty()
        && v.len() <= 40
        && v.starts_with(|c: char| c.is_ascii_digit())
        && v.chars()
            .all(|c| c.is_ascii_alphanumeric() || ".-+".contains(c))
}

/// The newest release of a CLI, when its source says (`None`: the CLI has no way to tell).
pub async fn latest(kind: CliKind, program: Option<&Path>) -> Result<Option<String>, String> {
    if let Some(pkg) = kind.npm_package() {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
            .map_err(|e| e.to_string())?;
        let v: serde_json::Value = http
            .get(format!("https://registry.npmjs.org/{pkg}/latest"))
            .header(reqwest::header::USER_AGENT, "agenium")
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(|e| format!("npm: {e}"))?
            .json()
            .await
            .map_err(|e| format!("npm: {e}"))?;
        return Ok(v["version"]
            .as_str()
            .filter(|s| valid_version(s))
            .map(str::to_owned));
    }
    if kind == CliKind::Grok {
        let Some(p) = program else { return Ok(None) };
        let out = run(p, &["update", "--check", "--json"], Duration::from_secs(60)).await?;
        let v: serde_json::Value = serde_json::from_str(out.trim()).map_err(|e| e.to_string())?;
        if let Some(e) = v["error"].as_str() {
            return Err(format!("grok update --check: {e}"));
        }
        return Ok(v["latestVersion"].as_str().map(str::to_owned));
    }
    Ok(None)
}

/// What an update did: the CLI's last lines.
#[derive(Debug, Clone, Serialize)]
pub struct Updated {
    pub output: String,
}

/// Runs the CLI's own updater (to the newest version, or to `version`). It runs as the user would run it, on the host:
/// the CLI's install is the user's.
pub async fn update(
    kind: CliKind,
    program: &Path,
    version: Option<&str>,
) -> Result<Updated, String> {
    if let Some(v) = version {
        if !valid_version(v) {
            return Err(format!("not a version: {v}"));
        }
    }
    let args = kind.update_args(version).ok_or_else(|| match version {
        Some(v) => format!(
            "{} cannot install a given version ({v}) itself",
            kind.program()
        ),
        None => format!("{} has no updater Agenium can run", kind.program()),
    })?;
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let out = run(program, &args, UPDATE_TIMEOUT).await?;
    let tail: Vec<&str> = out.lines().filter(|l| !l.trim().is_empty()).collect();
    Ok(Updated {
        output: tail[tail.len().saturating_sub(12)..].join("\n"),
    })
}

/// Runs `program args` without a console window or input; `Err` with its last lines when it fails.
async fn run(program: &Path, args: &[&str], timeout: Duration) -> Result<String, String> {
    let mut cmd = tokio::process::Command::new(program);
    cmd.args(args)
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true)
        // Updaters that would ask are told there is no one to answer.
        .env("CI", "1")
        .env("NO_COLOR", "1");
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW: the desktop app has no console
    let out = tokio::time::timeout(timeout, cmd.output())
        .await
        .map_err(|_| {
            format!(
                "{} {} took longer than {} s",
                program.display(),
                args.join(" "),
                timeout.as_secs()
            )
        })?
        .map_err(|e| format!("{}: {e}", program.display()))?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    if out.status.success() {
        Ok(text)
    } else {
        let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
        Err(format!(
            "{} {} failed ({}): {}",
            program.display(),
            args.join(" "),
            out.status,
            lines[lines.len().saturating_sub(8)..].join("\n")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_cli_updates_with_its_own_updater_and_says_what_it_cannot() {
        assert_eq!(CliKind::Claude.update_args(None).unwrap(), ["update"]);
        assert_eq!(
            CliKind::Claude.update_args(Some("2.1.316")).unwrap(),
            ["install", "2.1.316"]
        );
        assert_eq!(
            CliKind::Opencode.update_args(Some("2.1.2")).unwrap(),
            ["upgrade", "2.1.2"]
        );
        assert_eq!(
            CliKind::Grok.update_args(Some("1.0.41")).unwrap(),
            ["update", "--version", "1.0.41"]
        );
        assert_eq!(CliKind::Codex.update_args(None).unwrap(), ["update"]);
        assert!(CliKind::Codex.update_args(Some("0.163.0")).is_none());
        assert!(CliKind::Agy.update_args(Some("1.4.12")).is_none());
        assert_eq!(CliKind::Devin.update_args(None).unwrap(), ["update"]);
        assert!(CliKind::Devin.update_args(Some("3000.11.3")).is_none());
        assert!(
            newer("0.166.0", "0.163.0")
                && !newer("2.1.2", "2.1.2")
                && newer("codex-cli 1.0.0", "0.9.9")
        );
        assert!(valid_version("2.1.316") && valid_version("0.1.151-alpha.2"));
        assert!(!valid_version("2.1; rm -rf") && !valid_version("") && !valid_version("latest"));
        for k in CliKind::ALL {
            assert!(
                k.verified_version()
                    .is_some_and(|v| parse_version(v).unwrap() >= k.min_version()),
                "{k:?}"
            );
        }
    }
}
