//! Short probes of a CLI (its version, login, models, usage, updates): started with no console window, in a process
//! group of their own, and ended with their whole tree. A plain kill ended only the `.cmd` shim of an npm-installed
//! CLI and left the `node` behind it running.

use std::io::Read;
use std::process::{Output, Stdio};
use std::time::{Duration, Instant};

use agenium_sandbox::TreeGuard;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Starts `cmd`; its tree ends when the returned guard is dropped.
pub(crate) fn spawn(
    cmd: &mut tokio::process::Command,
) -> std::io::Result<(tokio::process::Child, TreeGuard)> {
    cmd.kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    #[cfg(unix)]
    cmd.process_group(0);
    let child = cmd.spawn()?;
    let tree = TreeGuard::attach(&child);
    Ok((child, tree))
}

/// `cmd`'s output, its pipes read while it runs. `None`: it did not end within `timeout` (its tree is ended).
pub(crate) async fn output(
    mut cmd: tokio::process::Command,
    timeout: Duration,
) -> std::io::Result<Option<Output>> {
    let (child, _tree) = spawn(&mut cmd)?;
    match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(out) => out.map(Some),
        Err(_) => Ok(None),
    }
}

/// The blocking form of [`output`], for detection that runs off the async workers: stdout and stderr together.
pub(crate) fn output_blocking(mut cmd: std::process::Command, timeout: Duration) -> Option<String> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let mut child = cmd.spawn().ok()?;
    // Read while it runs: a CLI that writes more than a pipe holds waits for a reader.
    let reader = |p: Option<Box<dyn Read + Send>>| {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut s = String::new();
            if let Some(mut p) = p {
                let _ = p.read_to_string(&mut s);
            }
            let _ = tx.send(s);
        });
        rx
    };
    let out = reader(
        child
            .stdout
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );
    let err = reader(
        child
            .stderr
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() < timeout => std::thread::sleep(Duration::from_millis(25)),
            _ => {
                // Still running: its id is still its own.
                kill_tree(child.id());
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    // Something it started may still hold the pipes: not waited for past a moment.
    let left = |rx: std::sync::mpsc::Receiver<String>| {
        rx.recv_timeout(Duration::from_secs(2)).unwrap_or_default()
    };
    let mut s = left(out);
    s.push_str(&left(err));
    Some(s)
}

/// Ends the process `pid` started with everything it started.
fn kill_tree(pid: u32) {
    #[cfg(windows)]
    let mut k = {
        use std::os::windows::process::CommandExt;
        let mut k = std::process::Command::new("taskkill");
        k.args(["/T", "/F", "/PID", &pid.to_string()])
            .creation_flags(CREATE_NO_WINDOW);
        k
    };
    #[cfg(unix)]
    let mut k = {
        let mut k = std::process::Command::new("kill");
        k.args(["-KILL", "--", &format!("-{pid}")]);
        k
    };
    let _ = k.stdout(Stdio::null()).stderr(Stdio::null()).status();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A probe that does not end in time is ended; one that writes a lot is read while it runs.
    #[test]
    fn a_probe_ends_in_time_and_is_read_while_it_runs() {
        #[cfg(windows)]
        let (slow, chatty) = {
            let mut slow = std::process::Command::new("powershell");
            slow.args(["-NoProfile", "-Command", "Start-Sleep 30"]);
            let mut chatty = std::process::Command::new("powershell");
            chatty.args(["-NoProfile", "-Command", "'x' * 200000; 'done'"]);
            (slow, chatty)
        };
        #[cfg(unix)]
        let (slow, chatty) = {
            let mut slow = std::process::Command::new("sh");
            slow.args(["-c", "sleep 30"]);
            let mut chatty = std::process::Command::new("sh");
            chatty.args([
                "-c",
                "head -c 200000 /dev/zero | tr '\\0' x; echo; echo done",
            ]);
            (slow, chatty)
        };
        let t = Instant::now();
        assert!(output_blocking(slow, Duration::from_secs(1)).is_none());
        assert!(t.elapsed() < Duration::from_secs(10));
        let out = output_blocking(chatty, Duration::from_secs(30)).unwrap();
        assert!(out.len() > 200_000 && out.contains("done"), "{}", out.len());
    }
}
