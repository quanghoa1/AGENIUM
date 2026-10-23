use std::path::Path;
use std::sync::Arc;

use agenium_tools::*;
use serde_json::json;

fn ctx(root: &Path) -> ToolCtx {
    ToolCtx::new(root, "run1", Arc::new(AllowAll))
}

fn tool(name: &str) -> Arc<dyn Tool> {
    work_tools().into_iter().find(|t| t.name() == name).unwrap()
}

async fn call(c: &ToolCtx, name: &str, args: serde_json::Value) -> Result<ToolOutput, ToolError> {
    tool(name).call(c, args).await
}

fn setup() -> (tempfile::TempDir, ToolCtx) {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("wt");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("src/lib.rs"),
        "pub fn add(a: i32, b: i32) -> i32 {\n    a - b\n}\n// add\n",
    )
    .unwrap();
    std::fs::write(root.join("README.md"), "# demo\n").unwrap();
    std::fs::write(root.join(".gitignore"), "target/\n").unwrap();
    std::fs::create_dir_all(root.join("target")).unwrap();
    std::fs::write(root.join("target/junk.rs"), "add").unwrap();
    let c = ctx(&root);
    (tmp, c)
}

#[test]
fn catalogue_is_sorted_and_described() {
    let names: Vec<&str> = work_tools().iter().map(|t| t.name()).collect();
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted);
    assert_eq!(
        names,
        [
            "apply_patch",
            "edit_file",
            "fetch",
            "glob",
            "grep",
            "list_dir",
            "multi_edit",
            "read_file",
            "shell",
            "todo",
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

#[tokio::test]
async fn shell_gate_timeout_and_truncation_with_artifact() {
    let (_t, mut c) = setup();
    let long = match c.executor.shell() {
        agenium_sandbox::Shell::Bash(_) => "for i in $(seq 1 1000); do echo line $i; done",
        agenium_sandbox::Shell::Cmd => "for /L %i in (1,1,1000) do @echo line %i",
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
async fn fetch_needs_allowlist_and_marks_untrusted() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let (_t, mut c) = setup();
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (mut s, _) = l.accept().await.unwrap();
        let mut b = [0u8; 4096];
        let _ = s.read(&mut b).await;
        let body = "<html><head><script>steal()</script></head><body><h1>Docs</h1><p>Use &amp; enjoy</p></body></html>";
        let resp = format!("HTTP/1.1 200 OK\r\ncontent-type: text/html\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len());
        s.write_all(resp.as_bytes()).await.unwrap();
    });
    let url = format!("http://127.0.0.1:{port}/docs");
    assert_eq!(
        call(&c, "fetch", json!({"url": url}))
            .await
            .unwrap_err()
            .code,
        "domain_not_allowed"
    );
    assert_eq!(
        call(&c, "fetch", json!({"url": "file:///etc/passwd"}))
            .await
            .unwrap_err()
            .code,
        "invalid_url"
    );
    c.fetch_allow = vec!["127.0.0.1".into()];
    let out = call(&c, "fetch", json!({"url": url})).await.unwrap();
    assert!(out.untrusted);
    assert!(
        out.text.contains("Docs")
            && out.text.contains("Use & enjoy")
            && !out.text.contains("steal"),
        "{}",
        out.text
    );
    assert!(
        domain_allowed("api.crates.io", &["*.crates.io".into()])
            && !domain_allowed("evilcrates.io", &["*.crates.io".into()])
    );
}

#[tokio::test]
async fn todo_replaces_checklist() {
    let (_t, c) = setup();
    let out = call(&c, "todo", json!({"items": [{"text": "read code", "status": "done"}, {"text": "fix", "status": "in_progress"}]})).await.unwrap();
    assert!(out.text.starts_with("1/2 done\n[x] read code\n[>] fix"));
    assert_eq!(c.todo().len(), 2);
}
