//! Claude's subscription usage, read on the user's request (ADR-67).

use std::path::Path;

use serde_json::Value;

use super::{user_claude_dir, LOGIN};
use crate::driver::probe_dir;
use crate::login_access::{lend_login, sync_lent};

/// Claude's subscription usage, read on the user's request: one tiny Haiku turn without tools (it costs a little
/// quota) whose stream reports `rate_limit_event`. It runs with a scratch `CLAUDE_CONFIG_DIR` holding a copy of the
/// login, like a run, and hands a refreshed login back the same way.
pub async fn rate_limits(
    program: &Path,
    scratch: &Path,
) -> Result<agenium_llm::vendor_usage::VendorUsage, String> {
    use tokio::io::{AsyncBufReadExt, BufReader};
    let user = user_claude_dir().ok_or("cannot find the user's Claude Code folder")?;
    // Deleted with the login copy when this function returns, whichever way.
    let dir = probe_dir(scratch, "claude-home")?;
    let home = dir.path().join("claude-home");
    let cwd = dir.path().join("cwd");
    for d in [&home, &cwd] {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    // Lent without its refresh token, like a run's (ADR-137): the probe lasts two minutes at most.
    lend_login(
        &user.join(LOGIN),
        &home.join(LOGIN),
        super::lend,
        std::time::Duration::from_secs(120),
    )
    .map_err(|e| e.to_string())?;
    let _released = crate::login_access::Released(home.clone());
    let mut cmd = tokio::process::Command::new(program);
    cmd.args([
        "-p",
        "Reply with the single word ok.",
        "--model",
        "haiku",
        "--output-format",
        "stream-json",
        "--verbose",
        "--tools",
        "",
        "--strict-mcp-config",
        "--no-session-persistence",
        "--setting-sources",
        "user",
    ])
    .env("CLAUDE_CONFIG_DIR", &home)
    .current_dir(&cwd)
    .stdin(std::process::Stdio::null())
    .stdout(std::process::Stdio::piped())
    .stderr(std::process::Stdio::null())
    .kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("cannot start claude: {e}"))?;
    let stdout = child.stdout.take().ok_or("claude stdout missing")?;
    let read = async {
        let mut lines = BufReader::new(stdout).lines();
        let mut found = None;
        let mut error = None;
        while let Some(line) = lines.next_line().await.map_err(|e| e.to_string())? {
            let Ok(v) = serde_json::from_str::<Value>(line.trim()) else {
                continue;
            };
            match v["type"].as_str() {
                Some("rate_limit_event") => {
                    found =
                        agenium_llm::vendor_usage::from_claude_event(&v, &agenium_core::now_iso())
                }
                Some("result") if v["is_error"] == true => {
                    error = Some(v["result"].as_str().unwrap_or("claude failed").to_owned())
                }
                _ => {}
            }
        }
        found.ok_or_else(|| error.unwrap_or_else(|| "claude reported no rate limits".into()))
    };
    let out = tokio::time::timeout(std::time::Duration::from_secs(120), read)
        .await
        .unwrap_or_else(|_| Err("claude did not answer in 120 s".into()));
    let _ = child.kill().await;
    if let Err(e) = sync_lent(&user.join(LOGIN), &home.join(LOGIN), super::lend) {
        tracing::warn!(error = %e, "claude login not handed back");
    }
    out
}
