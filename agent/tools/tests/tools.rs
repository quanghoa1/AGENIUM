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

/// The test worktree. Its commands' TEMP lies outside it, under the system TEMP, and goes with it.
struct Worktree {
    root: std::path::PathBuf,
    _tmp: tempfile::TempDir,
}

impl Drop for Worktree {
    fn drop(&mut self) {
        agenium_sandbox::remove_run_temp_dir(&self.root);
    }
}

fn setup() -> (Worktree, ToolCtx) {
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
    (Worktree { root, _tmp: tmp }, c)
}

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
            "edit_file",
            "fetch",
            "glob",
            "grep",
            "list_dir",
            "multi_edit",
            "read_file",
            "shell",
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

/// A local HTTP server answering every connection with `reply`; returns its port.
async fn serve(reply: impl Fn() -> Vec<u8> + Send + Sync + 'static) -> u16 {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    let reply = Arc::new(reply);
    tokio::spawn(async move {
        while let Ok((mut s, _)) = l.accept().await {
            let reply = reply.clone();
            tokio::spawn(async move {
                let mut b = [0u8; 4096];
                let _ = s.read(&mut b).await;
                let _ = s.write_all(&reply()).await;
                // Keep the connection open: a body without a length ends only when it closes.
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            });
        }
    });
    port
}

fn ok_reply(body: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 200 OK\r\ncontent-type: text/plain\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

/// Review of 2026-10-27: the fetch client followed redirects, so the allowlist held for the first URL only, a host
/// let in by a wildcard could be a private address, and the whole body was read before the 2 MB cut.
#[tokio::test]
async fn fetch_checks_every_redirect_and_private_addresses_and_stops_at_the_cap() {
    let (_t, mut c) = setup();
    let secret = serve(|| ok_reply("internal admin page")).await;
    let redirect = serve(move || {
        format!("HTTP/1.1 302 Found\r\nlocation: http://127.0.0.1:{secret}/admin\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
            .into_bytes()
    })
    .await;
    // `localhost` is allowlisted by name; the redirect goes to 127.0.0.1, which is not.
    c.fetch_allow = vec!["localhost".into()];
    let e = call(
        &c,
        "fetch",
        json!({"url": format!("http://localhost:{redirect}/")}),
    )
    .await
    .unwrap_err();
    assert_eq!(e.code, "domain_not_allowed", "{e:?}");
    // Allowlisted by its exact name, the same redirect is followed.
    c.fetch_allow = vec!["localhost".into(), "127.0.0.1".into()];
    let out = call(
        &c,
        "fetch",
        json!({"url": format!("http://localhost:{redirect}/")}),
    )
    .await
    .unwrap();
    assert!(out.text.contains("internal admin page"), "{}", out.text);
    // A wildcard lets in a loopback address: refused.
    c.fetch_allow = vec!["*.0.0.1".into()];
    let e = call(
        &c,
        "fetch",
        json!({"url": format!("http://127.0.0.1:{secret}/")}),
    )
    .await
    .unwrap_err();
    assert_eq!(e.code, "address_not_allowed", "{e:?}");

    // A body without a length that never ends: read up to the cap, then the tool returns.
    let endless = serve(|| {
        let mut r =
            b"HTTP/1.1 200 OK\r\ncontent-type: text/plain\r\nconnection: close\r\n\r\n".to_vec();
        r.extend(std::iter::repeat_n(b'a', 3 * 1024 * 1024));
        r
    })
    .await;
    c.fetch_allow = vec!["127.0.0.1".into()];
    let out = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        call(
            &c,
            "fetch",
            json!({"url": format!("http://127.0.0.1:{endless}/")}),
        ),
    )
    .await
    .expect("the fetch stops at the cap instead of waiting for the end of the body")
    .unwrap();
    let full = std::fs::read_to_string(c.root.join(out.full.unwrap())).unwrap();
    let body = full.matches('a').count();
    assert!(
        (2 * 1024 * 1024..=2 * 1024 * 1024 + 64).contains(&body),
        "{body}"
    );

    use std::net::IpAddr;
    for ip in [
        "127.0.0.1",
        "10.1.2.3",
        "172.16.0.1",
        "192.168.1.1",
        "169.254.169.254",
        "100.64.0.1",
        "0.0.0.0",
        "::1",
        "::",
        "fe80::1",
        "fd00::1",
        "::ffff:127.0.0.1",
        "::ffff:169.254.169.254",
        "64:ff9b::a9fe:a9fe",
    ] {
        assert!(!public_ip(ip.parse::<IpAddr>().unwrap()), "{ip}");
    }
    for ip in ["93.184.215.14", "2606:4700::1111", "::ffff:8.8.8.8"] {
        assert!(public_ip(ip.parse::<IpAddr>().unwrap()), "{ip}");
    }
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
