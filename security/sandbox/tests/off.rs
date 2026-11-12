//! The sandbox turned off by the user (ADR-160): `detect()` reports none and a command runs directly on the host, so
//! it can write outside its worktree, which the sandbox refuses. Its own test binary: `set_off` is process-wide.

use std::time::Duration;

use agenium_sandbox::{
    detect, detect_os, is_off, set_off, CommandSpec, Executor, SandboxPolicy, Shell,
};

#[tokio::test]
async fn a_command_runs_on_the_host_when_the_sandbox_is_off() {
    let os = detect_os();
    set_off(true);
    assert!(is_off());
    let a = detect();
    assert!(a.backend.is_none(), "off reports no backend");
    assert!(a.reason.contains("turned off"), "{}", a.reason);
    assert_eq!(detect_os(), os, "the OS backend is still known");

    let root = tempfile::tempdir().unwrap();
    let wt = root.path().join("wt");
    std::fs::create_dir(&wt).unwrap();
    let outside = root.path().join("outside.txt");
    let ex = Executor::new(Shell::detect());
    let path = outside.display().to_string();
    let command = match ex.shell() {
        Shell::Bash(_) => format!("echo yolo > '{path}'"),
        Shell::Cmd => format!("echo yolo> \"{path}\""),
        Shell::PowerShell(_) => format!("[IO.File]::WriteAllText('{path}', 'yolo')"),
    };
    let out = ex
        .run(
            &CommandSpec {
                command,
                cwd: wt.clone(),
                timeout: Duration::from_secs(60),
                ..Default::default()
            },
            &SandboxPolicy::workspace(&wt),
        )
        .await
        .unwrap();
    set_off(false);
    assert_eq!(out.exit_code, Some(0), "{out:?}");
    assert!(
        std::fs::read_to_string(&outside).unwrap().contains("yolo"),
        "the command ran on the host and wrote outside its worktree"
    );
    assert_eq!(detect(), os, "turned back on, the sandbox is used again");
    agenium_sandbox::remove_run_temp_dir(&wt);
}
