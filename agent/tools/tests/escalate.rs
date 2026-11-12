//! Sandbox first, then the user (ADR-161): a command the sandbox blocks goes to the gate to run again outside it; one
//! the agent asks to run outside goes to the gate before it runs. Needs a working sandbox (skipped without one).

mod common;

use std::path::Path;
use std::sync::{Arc, Mutex};

use agenium_tools::*;
use async_trait::async_trait;
use common::*;
use serde_json::json;

/// Allows every command, and answers escalations as told, keeping what it was asked.
struct Escalations {
    allow: bool,
    asked: Mutex<Vec<String>>,
}

#[async_trait]
impl CommandGate for Escalations {
    async fn check(&self, _: &str, _: &Path) -> GateDecision {
        GateDecision::Allow
    }
    async fn escalate(&self, _: &str, _: &Path, why: &str) -> GateDecision {
        self.asked.lock().unwrap().push(why.to_owned());
        if self.allow {
            GateDecision::Allow
        } else {
            GateDecision::Deny("not now".into())
        }
    }
}

/// A command that writes `path` (in a read-only folder of the sandbox).
fn write_line(path: &Path) -> String {
    let p = path.display().to_string();
    match agenium_sandbox::Shell::detect() {
        agenium_sandbox::Shell::Bash(_) => format!("echo out > '{p}'"),
        agenium_sandbox::Shell::Cmd => format!("echo out> \"{p}\""),
        agenium_sandbox::Shell::PowerShell(_) => format!("[IO.File]::WriteAllText('{p}', 'out')"),
    }
}

fn setup_with(allow: bool) -> (Worktree, ToolCtx, Arc<Escalations>, std::path::PathBuf) {
    let (wt, _) = setup();
    let gate = Arc::new(Escalations {
        allow,
        asked: Mutex::default(),
    });
    let mut c = ToolCtx::new(&wt.root, "run1", gate.clone());
    // A folder the sandbox shows read-only (as a reference folder or a toolchain is): writing there is refused
    // in a way every backend reports (EPERM, EROFS, access denied), while outside the sandbox it works.
    let ro = wt.root.parent().unwrap().join("ro");
    std::fs::create_dir_all(&ro).unwrap();
    c.policy.read_roots.push(ro.clone());
    (wt, c, gate, ro.join("out.txt"))
}

#[tokio::test]
async fn a_command_the_sandbox_blocks_runs_again_outside_once_allowed() {
    if agenium_sandbox::detect().backend.is_none() {
        eprintln!("skipped: no sandbox backend here");
        return;
    }
    let (_wt, c, gate, outside) = setup_with(true);
    let out = call(&c, "shell", json!({"command": write_line(&outside)}))
        .await
        .unwrap();
    let asked = gate.asked.lock().unwrap().clone();
    assert_eq!(asked.len(), 1, "{asked:?}\n{}", out.text);
    assert!(
        asked[0].starts_with("the sandbox blocked it: "),
        "{asked:?}"
    );
    assert!(
        out.text.contains("ran again outside the sandbox"),
        "{}",
        out.text
    );
    assert!(out.text.contains("exit code 0"), "{}", out.text);
    assert!(
        outside.exists(),
        "the second run, outside the sandbox, wrote it"
    );
}

#[tokio::test]
async fn refused_it_keeps_the_sandbox_answer_and_says_why() {
    if agenium_sandbox::detect().backend.is_none() {
        eprintln!("skipped: no sandbox backend here");
        return;
    }
    let (_wt, c, gate, outside) = setup_with(false);
    let out = call(&c, "shell", json!({"command": write_line(&outside)}))
        .await
        .unwrap();
    assert_eq!(gate.asked.lock().unwrap().len(), 1);
    assert!(
        out.text
            .contains("running it outside was not allowed: not now"),
        "{}",
        out.text
    );
    assert!(!outside.exists());
}

#[tokio::test]
async fn a_command_the_agent_says_needs_the_host_is_asked_before_it_runs() {
    if agenium_sandbox::detect().backend.is_none() {
        eprintln!("skipped: no sandbox backend here");
        return;
    }
    let (_wt, c, gate, outside) = setup_with(true);
    let out = call(
        &c,
        "shell",
        json!({"command": write_line(&outside), "outside_sandbox": "it writes next to the worktree"}),
    )
    .await
    .unwrap();
    let asked = gate.asked.lock().unwrap().clone();
    assert_eq!(
        asked,
        ["the agent asks to run it outside the sandbox: it writes next to the worktree"]
    );
    assert!(
        out.text.contains("it ran outside the sandbox"),
        "{}",
        out.text
    );
    assert!(outside.exists());

    // Refused up front, it does not run at all.
    let (_wt2, c2, _g2, outside2) = setup_with(false);
    let e = call(
        &c2,
        "shell",
        json!({"command": write_line(&outside2), "outside_sandbox": "needed"}),
    )
    .await
    .unwrap_err();
    assert_eq!(e.code, "command_denied", "{e:?}");
    assert!(!outside2.exists());
}

#[tokio::test]
async fn a_command_that_works_in_the_sandbox_asks_nothing() {
    let (_wt, c, gate, _) = setup_with(true);
    let out = call(&c, "shell", json!({"command": "echo inside"}))
        .await
        .unwrap();
    assert!(out.text.contains("inside"), "{}", out.text);
    assert!(gate.asked.lock().unwrap().is_empty());
}
