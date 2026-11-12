//! Telling that the sandbox stopped a command (ADR-161): the line of its output that says so, for the user to decide
//! whether it runs again outside the sandbox. Only a command that failed counts: npm and others print a refused
//! write as a warning and go on, and asking the user about a command that worked would only be noise.

use super::*;

/// What the OS answers a contained command it refuses: macOS Seatbelt (`EPERM`, and a nested `sandbox-exec`),
/// Linux bubblewrap (the read-only binds), Windows MXC.
const MARKERS: &[&str] = &[
    "Operation not permitted",
    "operation not permitted",
    "Read-only file system",
    "read-only file system",
    "sandbox_apply",
    "Access is denied",
    // .NET's refused write ("Access to the path 'C:\x' is denied."), PowerShell's own cmdlets.
    "Access to the path",
    "EPERM",
    "EROFS",
];

/// The line saying the sandbox refused something, when `out` is a command that failed; `None` otherwise.
pub fn blocked_by_sandbox(out: &ExecOutput) -> Option<String> {
    if out.timed_out || out.exit_code == Some(0) {
        return None;
    }
    out.stderr
        .lines()
        .chain(out.stdout.lines())
        .find(|l| MARKERS.iter().any(|m| l.contains(m)))
        .map(|l| {
            let l = l.trim();
            match l.char_indices().nth(300) {
                Some((i, _)) => format!("{}…", &l[..i]),
                None => l.to_owned(),
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn out(code: i32, stderr: &str) -> ExecOutput {
        ExecOutput {
            exit_code: Some(code),
            stdout: String::new(),
            stderr: stderr.into(),
            timed_out: false,
            elapsed_ms: 1,
        }
    }

    /// The refusals the macOS and Linux tool probes saw (kits v8-v10, the Ubuntu VM).
    #[test]
    fn a_failed_command_refused_by_the_sandbox_is_told_apart() {
        for (code, err) in [
            (126, "bash: line 1: /bin/ps: Operation not permitted"),
            (
                128,
                "fatal: Unable to create '/r/.git/worktrees/wt/index.lock': Read-only file system",
            ),
            (
                1,
                "error: x\nsandbox-exec: sandbox_apply: Operation not permitted",
            ),
            (1, "Remove-Item : Access is denied"),
        ] {
            let line = blocked_by_sandbox(&out(code, err)).expect(err);
            assert!(err.contains(&line), "{line}");
        }
    }

    #[test]
    fn a_command_that_worked_or_failed_for_another_reason_is_not() {
        assert_eq!(
            blocked_by_sandbox(&out(0, "npm warn: EPERM writing the log")),
            None
        );
        assert_eq!(
            blocked_by_sandbox(&out(1, "error[E0425]: cannot find value `x`")),
            None
        );
        let mut t = out(1, "Operation not permitted");
        t.timed_out = true;
        assert_eq!(blocked_by_sandbox(&t), None);
    }
}
