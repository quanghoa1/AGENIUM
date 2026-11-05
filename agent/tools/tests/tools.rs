mod common;

use std::path::Path;
use std::sync::Arc;

use agenium_tools::*;
use common::*;
use serde_json::json;

#[test]
fn catalogue_is_sorted_and_described() {
    let tools = work_tools();
    let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted);
    assert_eq!(
        names,
        [
            "apply_patch",
            "browser",
            "edit_file",
            "fetch",
            "glob",
            "grep",
            "list_dir",
            "multi_edit",
            "read_file",
            "shell",
            "shell_output",
            "shell_stop",
            "todo",
            "view_image",
            "write_file"
        ]
    );
    for t in work_tools() {
        assert_eq!(t.parameters()["type"], "object", "{}", t.name());
        assert!(t.description().is_ascii());
    }
}

#[tokio::test]
async fn read_requires_paths_inside_and_numbers_lines() {
    let (_t, c) = setup();
    let out = call(&c, "read_file", json!({"path": "src/lib.rs"}))
        .await
        .unwrap();
    assert!(out.text.contains("     2\t    a - b"), "{}", out.text);
    let out = call(
        &c,
        "read_file",
        json!({"path": "src/lib.rs", "offset": 2, "limit": 1}),
    )
    .await
    .unwrap();
    assert!(out.text.starts_with("     2\t") && out.text.contains("showing lines 2-2 of 4"));
    assert_eq!(
        call(&c, "read_file", json!({"path": "../secret"}))
            .await
            .unwrap_err()
            .code,
        "out_of_scope"
    );
    assert_eq!(
        call(&c, "read_file", json!({"path": "nope.txt"}))
            .await
            .unwrap_err()
            .code,
        "not_found"
    );
    assert_eq!(
        call(&c, "read_file", json!({"path": "src"}))
            .await
            .unwrap_err()
            .code,
        "is_directory"
    );
    assert_eq!(
        call(&c, "read_file", json!({"pathh": "x"}))
            .await
            .unwrap_err()
            .code,
        "invalid_arguments"
    );
    std::fs::write(c.root.join("bin.dat"), [0u8, 1, 2]).unwrap();
    assert_eq!(
        call(&c, "read_file", json!({"path": "bin.dat"}))
            .await
            .unwrap_err()
            .code,
        "binary_file"
    );
}

#[tokio::test]
async fn edit_rules_read_first_unique_and_fresh() {
    let (_t, c) = setup();
    let e =
        |old: &str, new: &str| json!({"path": "src/lib.rs", "old_string": old, "new_string": new});
    assert_eq!(
        call(&c, "edit_file", e("a - b", "a + b"))
            .await
            .unwrap_err()
            .code,
        "read_first"
    );
    call(&c, "read_file", json!({"path": "src/lib.rs"}))
        .await
        .unwrap();
    assert_eq!(
        call(&c, "edit_file", e("add", "plus"))
            .await
            .unwrap_err()
            .code,
        "not_unique"
    );
    assert_eq!(
        call(&c, "edit_file", e("zzz", "y")).await.unwrap_err().code,
        "not_found"
    );
    call(&c, "edit_file", e("a - b", "a + b")).await.unwrap();
    // The agent's own edits keep the ledger fresh; an outside change makes it stale.
    call(&c, "edit_file", json!({"path": "src/lib.rs", "old_string": "add", "new_string": "sum", "replace_all": true})).await.unwrap();
    assert_eq!(
        std::fs::read_to_string(c.root.join("src/lib.rs")).unwrap(),
        "pub fn sum(a: i32, b: i32) -> i32 {\n    a + b\n}\n// sum\n"
    );
    std::fs::write(c.root.join("src/lib.rs"), "changed by someone else\n").unwrap();
    assert_eq!(
        call(&c, "edit_file", e("changed", "x"))
            .await
            .unwrap_err()
            .code,
        "stale"
    );
    // multi_edit is atomic.
    call(&c, "read_file", json!({"path": "src/lib.rs"}))
        .await
        .unwrap();
    let r = call(&c, "multi_edit", json!({"path": "src/lib.rs", "edits": [{"old_string": "changed", "new_string": "edited"}, {"old_string": "missing", "new_string": "x"}]})).await;
    assert_eq!(r.unwrap_err().code, "not_found");
    assert_eq!(
        std::fs::read_to_string(c.root.join("src/lib.rs")).unwrap(),
        "changed by someone else\n"
    );
    call(&c, "multi_edit", json!({"path": "src/lib.rs", "edits": [{"old_string": "changed", "new_string": "edited"}, {"old_string": "someone else", "new_string": "me"}]})).await.unwrap();
    assert_eq!(
        std::fs::read_to_string(c.root.join("src/lib.rs")).unwrap(),
        "edited by me\n"
    );
}

#[tokio::test]
async fn write_creates_but_overwrite_needs_read_and_git_is_read_only() {
    let (_t, c) = setup();
    call(
        &c,
        "write_file",
        json!({"path": "deliverables/new/hello.txt", "content": "hi\n"}),
    )
    .await
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(c.root.join("deliverables/new/hello.txt")).unwrap(),
        "hi\n"
    );
    call(
        &c,
        "write_file",
        json!({"path": "deliverables/new/hello.txt", "content": "again\n"}),
    )
    .await
    .unwrap();
    assert_eq!(
        call(
            &c,
            "write_file",
            json!({"path": "README.md", "content": "x"})
        )
        .await
        .unwrap_err()
        .code,
        "read_first"
    );
    assert_eq!(
        call(
            &c,
            "write_file",
            json!({"path": ".git/hooks/pre-commit", "content": "x"})
        )
        .await
        .unwrap_err()
        .code,
        "read_only"
    );
}

#[tokio::test]
async fn apply_patch_is_atomic() {
    let (_t, c) = setup();
    let good = "*** Begin Patch\n*** Add File: docs/a.md\n+A\n*** Update File: src/lib.rs\n@@\n-    a - b\n+    a + b\n*** Update File: README.md\n*** Move to: docs/README.md\n@@\n-# demo\n+# Demo\n*** End Patch";
    let out = call(&c, "apply_patch", json!({"patch": good}))
        .await
        .unwrap();
    assert!(
        out.text.contains("A docs/a.md") && out.text.contains("R README.md -> docs/README.md"),
        "{}",
        out.text
    );
    assert!(std::fs::read_to_string(c.root.join("src/lib.rs"))
        .unwrap()
        .contains("a + b"));
    assert_eq!(
        std::fs::read_to_string(c.root.join("docs/README.md")).unwrap(),
        "# Demo\n"
    );
    assert!(!c.root.join("README.md").exists());
    let bad = "*** Begin Patch\n*** Add File: docs/b.md\n+B\n*** Update File: src/lib.rs\n@@\n-not there\n+x\n*** End Patch";
    assert_eq!(
        call(&c, "apply_patch", json!({"patch": bad}))
            .await
            .unwrap_err()
            .code,
        "hunk_failed"
    );
    assert!(
        !c.root.join("docs/b.md").exists(),
        "nothing written when a hunk fails"
    );
    let escape = "*** Begin Patch\n*** Add File: ../evil.txt\n+x\n*** End Patch";
    assert_eq!(
        call(&c, "apply_patch", json!({"patch": escape}))
            .await
            .unwrap_err()
            .code,
        "out_of_scope"
    );
}

#[tokio::test]
async fn search_tools_respect_gitignore() {
    let (_t, c) = setup();
    let g = call(&c, "glob", json!({"pattern": "**/*.rs"}))
        .await
        .unwrap();
    assert_eq!(g.text, "src/lib.rs");
    let gr = call(&c, "grep", json!({"pattern": "fn \\w+", "glob": "*.rs"}))
        .await
        .unwrap();
    assert!(
        gr.text.starts_with("src/lib.rs:1:pub fn add"),
        "{}",
        gr.text
    );
    assert!(!gr.text.contains("target"));
    let files = call(
        &c,
        "grep",
        json!({"pattern": "ADD", "case_insensitive": true, "output": "files"}),
    )
    .await
    .unwrap();
    assert!(files.text.starts_with("src/lib.rs\n"));
    assert_eq!(
        call(&c, "grep", json!({"pattern": "("}))
            .await
            .unwrap_err()
            .code,
        "invalid_pattern"
    );
    let l = call(&c, "list_dir", json!({"depth": 2})).await.unwrap();
    assert!(
        l.text.contains("src/") && l.text.contains("src/lib.rs") && !l.text.contains("target"),
        "{}",
        l.text
    );
}

/// ADR-129: a background command (a server) is started, read while it runs, stopped, and does not outlive the run.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_background_command_is_read_while_it_runs_and_stopped() {
    let (_t, c) = setup();
    // Prints a line, then keeps running like a server would.
    let cmd = match c.executor.shell() {
        agenium_sandbox::Shell::Cmd => "echo listening & ping -n 120 127.0.0.1 > nul",
        _ => "echo listening; sleep 120",
    };
    let out = call(&c, "shell", json!({"command": cmd, "background": true}))
        .await
        .unwrap();
    assert!(out.text.contains("job 1"), "{}", out.text);
    let read = call(&c, "shell_output", json!({"job": 1, "wait_ms": 20000}))
        .await
        .unwrap();
    assert!(read.text.contains("still running"), "{}", read.text);
    assert!(read.text.contains("listening"), "{}", read.text);
    // Nothing new since the last read.
    let again = call(&c, "shell_output", json!({"job": 1})).await.unwrap();
    assert!(again.text.contains("(no new output)"), "{}", again.text);
    let stopped = call(&c, "shell_stop", json!({"job": 1})).await.unwrap();
    assert!(stopped.text.contains("stopped"), "{}", stopped.text);
    let e = call(&c, "shell_output", json!({"job": 9}))
        .await
        .unwrap_err();
    assert_eq!(e.code, "unknown_job");
    assert!(e.message.contains("1 ("), "{}", e.message);
    // A second one ends with the run: dropping the context stops it.
    call(&c, "shell", json!({"command": cmd, "background": true}))
        .await
        .unwrap();
    let started = std::time::Instant::now();
    drop(c);
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
}

/// The run's stop ends a command at once, its process tree with it (it ran on to its timeout, up to 10 minutes), and
/// gives up an approval the user has not given (the run waited for it).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stopped_run_ends_its_command_and_its_wait_for_approval() {
    let (_t, mut c) = setup();
    let cmd = match c.executor.shell() {
        agenium_sandbox::Shell::Cmd => "ping -n 120 127.0.0.1 > nul",
        _ => "sleep 120",
    };
    let stop_after = |c: &ToolCtx, ms: u64| {
        let stop = c.stop.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
            stop.store(true, std::sync::atomic::Ordering::SeqCst);
        });
    };
    stop_after(&c, 800);
    let started = std::time::Instant::now();
    let out = call(&c, "shell", json!({"command": cmd, "timeout_ms": 300000}))
        .await
        .unwrap();
    assert!(
        started.elapsed() < std::time::Duration::from_secs(10),
        "{:?}",
        started.elapsed()
    );
    assert!(out.text.starts_with("stopped:"), "{}", out.text);

    struct Waits;
    #[async_trait::async_trait]
    impl CommandGate for Waits {
        async fn check(&self, _: &str, _: &Path) -> GateDecision {
            std::future::pending().await
        }
    }
    c.gate = Arc::new(Waits);
    c.stop.store(false, std::sync::atomic::Ordering::SeqCst);
    stop_after(&c, 500);
    let started = std::time::Instant::now();
    let e = call(&c, "shell", json!({"command": "echo hi"}))
        .await
        .unwrap_err();
    assert_eq!(e.code, "stopped", "{e}");
    assert!(started.elapsed() < std::time::Duration::from_secs(3));
}

/// ADR-133: in a vendor CLI's run (a foreground limit) a command that ends within the limit answers as usual; a longer
/// one goes on as a job whose end the run hands over afterwards; one waiting for the user's approval past the limit
/// goes on waiting, and the run's stop gives it up.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cli_runs_long_commands_as_jobs_it_is_told_the_end_of() {
    let (_t, mut c) = setup();
    // Wide apart: a shell's first start after a build can take most of a second (a scan of the new binaries), and
    // `echo hi` then counted as a long command (a flaky failure, audit of 2026-10-31).
    c.foreground_limit = Some(std::time::Duration::from_secs(3));
    let quick = call(&c, "shell", json!({"command": "echo hi"}))
        .await
        .unwrap();
    assert!(quick.text.starts_with("exit code 0"), "{}", quick.text);
    assert!(c.jobs.awaited_going().is_empty() && c.jobs.take_ended_untold().is_empty());

    let slow = match c.executor.shell() {
        agenium_sandbox::Shell::Cmd => "ping -n 9 127.0.0.1 > nul & echo done",
        agenium_sandbox::Shell::PowerShell(_) => "Start-Sleep -Seconds 8; echo done",
        agenium_sandbox::Shell::Bash(_) => "sleep 8; echo done",
    };
    let out = call(&c, "shell", json!({"command": slow})).await.unwrap();
    assert!(
        out.text.contains("still running") && out.text.contains("goes on as job"),
        "{}",
        out.text
    );
    assert_eq!(c.jobs.awaited_going().len(), 1);
    let started = std::time::Instant::now();
    while !c.jobs.awaited_going().is_empty()
        && started.elapsed() < std::time::Duration::from_secs(20)
    {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let ended = c.jobs.take_ended_untold();
    assert_eq!(ended.len(), 1);
    let m = ended_message(&c, &ended[0]);
    assert!(
        m.contains(") has ended: exit code 0") && m.contains("done"),
        "{m}"
    );
    assert!(c.jobs.take_ended_untold().is_empty(), "handed over once");

    struct Waits;
    #[async_trait::async_trait]
    impl CommandGate for Waits {
        async fn check(&self, _: &str, _: &Path) -> GateDecision {
            std::future::pending().await
        }
    }
    c.gate = Arc::new(Waits);
    let out = call(&c, "shell", json!({"command": "echo later"}))
        .await
        .unwrap();
    assert!(
        out.text.contains("waiting for the user's approval"),
        "{}",
        out.text
    );
    c.stop.store(true, std::sync::atomic::Ordering::SeqCst);
    let started = std::time::Instant::now();
    while !c.jobs.awaited_going().is_empty()
        && started.elapsed() < std::time::Duration::from_secs(5)
    {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let ended = c.jobs.take_ended_untold();
    assert!(ended_message(&c, &ended[0]).contains("stopped before it ended"));
}

#[tokio::test]
async fn shell_gate_timeout_and_truncation_with_artifact() {
    let (_t, mut c) = setup();
    let long = match c.executor.shell() {
        agenium_sandbox::Shell::Bash(_) => "for i in $(seq 1 1000); do echo line $i; done",
        agenium_sandbox::Shell::Cmd => "for /L %i in (1,1,1000) do @echo line %i",
        agenium_sandbox::Shell::PowerShell(_) => "1..1000 | ForEach-Object { \"line $_\" }",
    };
    let out = call(&c, "shell", json!({"command": long})).await.unwrap();
    assert!(
        out.truncated && out.text.contains("lines omitted"),
        "{}",
        &out.text[..200.min(out.text.len())]
    );
    let full = out.full.clone().unwrap();
    assert!(full.starts_with("artifacts/tool-output/run1/"));
    assert!(std::fs::read_to_string(c.root.join(&full))
        .unwrap()
        .contains("line 1000"));
    let out = call(&c, "shell", json!({"command": "exit 7"}))
        .await
        .unwrap();
    assert!(out.text.starts_with("exit code 7"));
    assert_eq!(
        call(&c, "shell", json!({"command": "echo hi", "workdir": ".."}))
            .await
            .unwrap_err()
            .code,
        "out_of_scope"
    );
    c.gate = Arc::new(DenyAll("the user declined".into()));
    let e = call(&c, "shell", json!({"command": "echo hi"}))
        .await
        .unwrap_err();
    assert_eq!(e.code, "command_denied");
    assert!(e.message.contains("the user declined"));
}

#[tokio::test]
async fn todo_replaces_checklist() {
    let (_t, c) = setup();
    let out = call(&c, "todo", json!({"items": [{"text": "read code", "status": "done"}, {"text": "fix", "status": "in_progress"}]})).await.unwrap();
    assert!(out.text.starts_with("1/2 done\n[x] read code\n[>] fix"));
    assert_eq!(c.todo().len(), 2);
}

/// ADR-102: a CRLF file with a byte-order mark is edited with the model's LF text and keeps both; typographic quotes
/// match their ASCII form, and the result says so.
#[tokio::test]
async fn edits_keep_line_ends_and_bom_and_match_typographic_text() {
    let (w, c) = setup();
    let f = w.root.join("src/win.cs");
    std::fs::write(&f, "\u{feff}var s = \u{201C}hi\u{201D};\r\nreturn s;\r\n").unwrap();
    call(&c, "read_file", json!({"path": "src/win.cs"}))
        .await
        .unwrap();
    let out = call(
        &c,
        "edit_file",
        json!({"path": "src/win.cs", "old_string": "var s = \"hi\";\nreturn", "new_string": "var s = \"hello\";\nreturn"}),
    )
    .await
    .unwrap();
    assert!(out.text.contains("matched after ignoring"), "{}", out.text);
    assert_eq!(
        std::fs::read_to_string(&f).unwrap(),
        "\u{feff}var s = \"hello\";\r\nreturn s;\r\n"
    );
}

/// `view_image` hands an image of the worktree to the model: the type comes from the bytes, not the name; a file that
/// is not an image, one too large or one outside the worktree is refused.
#[tokio::test]
async fn view_image_returns_the_image_and_refuses_the_rest() {
    let (_t, c) = setup();
    let png = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0];
    std::fs::write(c.root.join("shot.jpg"), png).unwrap();
    let out = call(&c, "view_image", json!({"path": "shot.jpg"}))
        .await
        .unwrap();
    assert_eq!(out.images.len(), 1);
    assert_eq!(out.images[0].media_type, "image/png");
    assert_eq!(out.images[0].label, "shot.jpg");
    assert_eq!(out.images[0].data_b64, "iVBORw0KGgoAAAAA");
    assert!(out.text.contains("image/png"), "{}", out.text);
    let e = call(&c, "view_image", json!({"path": "README.md"}))
        .await
        .unwrap_err();
    assert_eq!(e.code, "not_an_image");
    std::fs::write(
        c.root.join("big.png"),
        vec![0u8; (agenium_tools::MAX_IMAGE_BYTES + 1) as usize],
    )
    .unwrap();
    let e = call(&c, "view_image", json!({"path": "big.png"}))
        .await
        .unwrap_err();
    assert_eq!(e.code, "too_large");
    assert!(call(&c, "view_image", json!({"path": "../outside.png"}))
        .await
        .is_err());
}

/// Path review of 2026-10-29: a write scope holds where a write really leads, not only as named. A link
/// `src/auth/l -> ../billing` took `src/auth/l/x.rs` into `src/billing`. (Skipped where links cannot be made.)
#[tokio::test]
async fn a_link_does_not_take_a_write_out_of_the_scope() {
    let (_t, mut c) = setup();
    std::fs::create_dir_all(c.root.join("src/auth")).unwrap();
    std::fs::create_dir_all(c.root.join("src/billing")).unwrap();
    // A junction where a symbolic link needs a right the user may not have (no Developer Mode).
    #[cfg(windows)]
    let made =
        std::os::windows::fs::symlink_dir(c.root.join("src/billing"), c.root.join("src/auth/l"))
            .or_else(|_| {
                // mklink takes a `/` in a path for a switch.
                let win = |p: std::path::PathBuf| p.to_string_lossy().replace('/', "\\");
                std::process::Command::new("cmd")
                    .args(["/C", "mklink", "/J"])
                    .arg(win(c.root.join("src/auth/l")))
                    .arg(win(c.root.join("src/billing")))
                    .output()
                    .ok()
                    .filter(|o| o.status.success())
                    .map(|_| ())
                    .ok_or(())
            });
    #[cfg(unix)]
    let made = std::os::unix::fs::symlink("../billing", c.root.join("src/auth/l"));
    if made.is_err() {
        eprintln!("no link here (no right to make one): skipped");
        return;
    }
    c.write_scope =
        agenium_tools::paths::WriteScope::new(vec![vec!["src/auth/**".into()]], vec![]).unwrap();
    let e = call(
        &c,
        "write_file",
        json!({"path": "src/auth/l/x.rs", "content": "x"}),
    )
    .await
    .unwrap_err();
    assert_eq!(e.code, "out_of_scope", "{e}");
    assert!(!c.root.join("src/billing/x.rs").exists());
    call(
        &c,
        "write_file",
        json!({"path": "src/auth/ok.rs", "content": "x"}),
    )
    .await
    .unwrap();
}

/// Path review of 2026-10-29: a file with another name (a hard link to a file outside the worktree) is neither read
/// nor written: it may be a file out there (a secret to read, a file the write would change). Both go through the open
/// file, whose names the OS counts; grep leaves it out.
#[tokio::test]
async fn a_hard_link_out_of_the_worktree_is_not_read_or_written() {
    let (t, c) = setup();
    let outside = t.root.parent().unwrap().join("outside.txt");
    std::fs::write(&outside, "keep me").unwrap();
    if std::fs::hard_link(&outside, c.root.join("linked.txt")).is_err() {
        eprintln!("no hard link here: skipped");
        return;
    }
    let e = call(&c, "read_file", json!({"path": "linked.txt"}))
        .await
        .unwrap_err();
    assert_eq!(e.code, "out_of_scope", "{e}");
    assert!(e.message.contains("other names"), "{e}");
    let g = call(&c, "grep", json!({"pattern": "keep me"}))
        .await
        .unwrap();
    assert!(!g.text.contains("linked.txt"), "{}", g.text);
    assert!(
        g.text.contains("1 file(s) with other names"),
        "said, not a bare no-match: {}",
        g.text
    );
    call(
        &c,
        "write_file",
        json!({"path": "linked.txt", "content": "changed"}),
    )
    .await
    .unwrap_err();
    assert_eq!(std::fs::read_to_string(&outside).unwrap(), "keep me");
    // A file read while it had one name, given another out there before the write.
    std::fs::write(c.root.join("mine.txt"), "mine").unwrap();
    call(&c, "read_file", json!({"path": "mine.txt"}))
        .await
        .unwrap();
    std::fs::hard_link(
        c.root.join("mine.txt"),
        t.root.parent().unwrap().join("other.txt"),
    )
    .unwrap();
    let e = call(
        &c,
        "write_file",
        json!({"path": "mine.txt", "content": "changed"}),
    )
    .await
    .unwrap_err();
    assert_eq!(e.code, "out_of_scope", "{e}");
    assert!(e.message.contains("other names"), "{e}");
    assert_eq!(
        std::fs::read_to_string(c.root.join("mine.txt")).unwrap(),
        "mine"
    );
    call(
        &c,
        "write_file",
        json!({"path": "plain.txt", "content": "fine"}),
    )
    .await
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(c.root.join("plain.txt")).unwrap(),
        "fine"
    );
}

/// Audit H2 (2026-10-31): a full output was written with a plain write, through a link a sandboxed command could
/// plant at the name it would take. A name already taken is left alone and the next one is used.
#[test]
fn a_full_output_never_writes_over_what_is_at_its_name() {
    let (w, c) = setup();
    let first = c.save_output("one").unwrap();
    let taken = w.root.join(first.replace("0001", "0002"));
    std::fs::write(&taken, "the user's file").unwrap();
    let second = c.save_output("two").unwrap();
    assert!(second.ends_with("0003.log"), "{second}");
    assert_eq!(std::fs::read_to_string(&taken).unwrap(), "the user's file");
    assert_eq!(
        std::fs::read_to_string(w.root.join(&second)).unwrap(),
        "two"
    );
}

#[cfg(unix)]
#[test]
fn a_full_output_is_not_written_through_a_planted_link() {
    let (w, c) = setup();
    let outside = tempfile::tempdir().unwrap();
    let target = outside.path().join("bashrc");
    std::fs::write(&target, "export PATH\n").unwrap();
    let first = c.save_output("one").unwrap();
    // The next name, a link out of the worktree; and a folder of the next run's outputs turned into one.
    let next = w.root.join(first.replace("0001", "0002"));
    std::os::unix::fs::symlink(&target, &next).unwrap();
    let saved = c.save_output("rm -rf ~").unwrap();
    assert!(saved.ends_with("0003.log"), "{saved}");
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "export PATH\n");

    // A later run whose output folder was turned into a link out of the worktree.
    let other = ToolCtx::new(&w.root, "run2", Arc::new(AllowAll));
    std::os::unix::fs::symlink(outside.path(), w.root.join("artifacts/tool-output/run2")).unwrap();
    assert_eq!(
        other.save_output("x"),
        None,
        "nothing is written out of the worktree"
    );
    // At most an empty file is left out there (as `write_file` leaves one), never the output.
    assert!(std::fs::read_dir(outside.path()).unwrap().all(|e| {
        let n = e.unwrap().file_name();
        n == "bashrc" || std::fs::metadata(outside.path().join(&n)).unwrap().len() == 0
    }));
}

/// Audit L4 (2026-10-31): a model's huge offset or limit overflowed the window arithmetic (a panic in a debug build).
#[tokio::test]
async fn a_huge_read_window_is_read_whole() {
    let (_w, c) = setup();
    let out = call(
        &c,
        "read_file",
        json!({"path": "src/lib.rs", "offset": 2, "limit": u64::MAX}),
    )
    .await
    .unwrap();
    assert!(
        out.text.contains("a - b") && !out.text.contains("showing lines"),
        "{}",
        out.text
    );
}

/// Audit L3 (2026-10-31): a FIFO a command left in the worktree (`mkfifo`) blocked read_file and write_file for good,
/// on an async thread and past the run's stop. It is refused at once.
#[cfg(unix)]
#[tokio::test]
async fn a_fifo_in_the_worktree_is_refused_not_waited_on() {
    let (w, c) = setup();
    let fifo = w.root.join("pipe.txt");
    let path = std::ffi::CString::new(fifo.to_string_lossy().as_bytes()).unwrap();
    // SAFETY: a valid NUL-terminated path.
    assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
    let quick = std::time::Duration::from_secs(5);
    let read = tokio::time::timeout(quick, call(&c, "read_file", json!({"path": "pipe.txt"})))
        .await
        .expect("read_file returned");
    assert!(read.is_err());
    let write = tokio::time::timeout(
        quick,
        call(
            &c,
            "write_file",
            json!({"path": "pipe.txt", "content": "x"}),
        ),
    )
    .await
    .expect("write_file returned");
    assert!(write.is_err());
}
