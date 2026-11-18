//! Agenium's own browser (ADR-165): a run that reaches the network browses in one browser kept across runs, used like
//! a person uses one. Skipped when the machine has no Edge or Chrome.
mod common;

use std::path::Path;

use common::*;
use serde_json::json;

const PAGE: &str = r#"<!doctype html><title>Account</title><h1>Settings</h1>
<label for="mail">Email</label> <input id="mail" type="email">
<label>Password <input id="pw" type="password"></label>
<select id="plan" aria-label="Plan"><option value="f">Free</option><option value="p">Pro</option></select>
<input id="file" type="file" aria-label="Attachment">
<button id="save" onclick="setTimeout(() => { document.getElementById('out').textContent = 'Saved ' +
  document.getElementById('mail').value + ' ' + document.getElementById('plan').value;
  localStorage.setItem('k', 'v1'); document.cookie = 'c=v2; max-age=3600; path=/'; }, 600)">Save</button>
<p id="out"></p><a href="/two">Two</a>
<div id="list" style="height:100px;overflow:auto"><div style="height:2000px">tall</div></div>"#;

/// A tiny site on this machine: `/` is the page, anything else a second page.
async fn site() -> String {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    tokio::spawn(async move {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        while let Ok((mut s, _)) = l.accept().await {
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                let n = s.read(&mut buf).await.unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]);
                let body = if req.starts_with("GET / ") {
                    PAGE
                } else {
                    "<!doctype html><title>Two</title><p>second page</p>"
                };
                let _ = s
                    .write_all(
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        )
                        .as_bytes(),
                    )
                    .await;
            });
        }
    });
    format!("http://127.0.0.1:{port}/")
}

fn ref_of(text: &str, has: &str) -> String {
    text.lines()
        .find(|l| l.starts_with('[') && l.contains(has))
        .and_then(|l| l[1..].split(']').next())
        .unwrap_or_else(|| panic!("no control with {has} in:\n{text}"))
        .to_owned()
}

fn shared_ctx(root: &Path, profile: &Path) -> agenium_tools::ToolCtx {
    let mut c = ctx(root);
    c.browser_profile = Some(profile.to_path_buf());
    c
}

async fn b(c: &agenium_tools::ToolCtx, args: serde_json::Value) -> String {
    call(c, "browser", args.clone())
        .await
        .unwrap_or_else(|e| panic!("{args}: {e}"))
        .text
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agenium_s_browser_is_used_like_a_person_s_and_keeps_what_sites_saved() {
    if agenium_tools::browser::find_browser().is_none() {
        eprintln!("no Edge or Chrome on this machine: skipped");
        return;
    }
    let url = site().await;
    let (wt, _) = setup();
    let dir = tempfile::tempdir().unwrap();
    let profile = dir.path().join("browser");
    let c = shared_ctx(&wt.root, &profile);

    let first = b(&c, json!({"action": "open", "url": url})).await;
    assert!(first.contains("Settings"), "{first}");
    let mail = ref_of(&first, "\"Email\"");
    b(&c, json!({"action": "type", "ref": mail, "text": "a@b.c"})).await;
    let o = b(&c, json!({"action": "look"})).await;
    b(
        &c,
        json!({"action": "type", "ref": ref_of(&o, "\"Password\""), "text": "hunter2"}),
    )
    .await;
    let o = b(&c, json!({"action": "look"})).await;
    assert!(
        o.lines()
            .any(|l| l.contains("input[password]") && l.contains("filled")),
        "{o}"
    );
    assert!(!o.contains("hunter2"), "a password never shows: {o}");
    assert!(o.contains("value=\"a@b.c\""), "{o}");
    b(
        &c,
        json!({"action": "select", "ref": ref_of(&o, "\"Plan\""), "text": "Pro"}),
    )
    .await;
    let o = b(&c, json!({"action": "look"})).await;
    b(
        &c,
        json!({"action": "click", "ref": ref_of(&o, "button \"Save\"")}),
    )
    .await;
    let o = b(&c, json!({"action": "wait_for", "text": "Saved a@b.c p"})).await;
    assert!(o.contains("Saved a@b.c p"), "{o}");
    // A ref of an older look is refused, not sent to whatever now has that number.
    let e = call(&c, "browser", json!({"action": "click", "ref": mail}))
        .await
        .unwrap_err();
    assert_eq!(e.code, "stale_ref", "{e}");

    std::fs::write(wt.root.join("up.txt"), "hello").unwrap();
    b(
        &c,
        json!({"action": "upload", "ref": ref_of(&o, "\"Attachment\""), "path": "up.txt"}),
    )
    .await;
    let name = b(
        &c,
        json!({"action": "eval", "expression": "document.getElementById('file').files[0].name"}),
    )
    .await;
    assert!(name.contains("up.txt"), "{name}");
    b(
        &c,
        json!({"action": "scroll", "selector": "#list", "text": "down"}),
    )
    .await;
    let top = b(
        &c,
        json!({"action": "eval", "expression": "document.getElementById('list').scrollTop > 0"}),
    )
    .await;
    assert_eq!(top.trim(), "true", "the list under the mouse scrolled");
    let o = b(&c, json!({"action": "look"})).await;
    let two = b(&c, json!({"action": "click", "ref": ref_of(&o, "\"Two\"")})).await;
    assert!(two.contains("second page"), "{two}");
    let back = b(&c, json!({"action": "back"})).await;
    assert!(back.contains("Settings"), "{back}");
    let e = call(
        &c,
        "browser",
        json!({"action": "wait_for", "text": "never", "timeout_ms": 300}),
    )
    .await
    .unwrap_err();
    assert_eq!(e.code, "wait_timeout");
    drop(c);

    // The next run's tab is in the same browser: what the site saved is there.
    let stored = "localStorage.getItem('k') + ' ' + document.cookie";
    let c = shared_ctx(&wt.root, &profile);
    b(&c, json!({"action": "open", "url": url})).await;
    let v = b(&c, json!({"action": "eval", "expression": stored})).await;
    assert!(v.contains("v1") && v.contains("c=v2"), "{v}");
    drop(c);

    // And after the browser closed and started again, as after a restart of Agenium.
    agenium_tools::browser::shutdown(&profile).await;
    let c = shared_ctx(&wt.root, &profile);
    b(&c, json!({"action": "open", "url": url})).await;
    let v = b(&c, json!({"action": "eval", "expression": stored})).await;
    assert!(v.contains("v1") && v.contains("c=v2"), "kept on disk: {v}");
    drop(c);
    agenium_tools::browser::shutdown(&profile).await;
}

#[tokio::test]
async fn a_run_off_the_network_has_no_window_for_the_user_and_no_private_addresses_open_on_it() {
    let (_wt, c) = setup();
    let e = call(&c, "browser", json!({"action": "show"}))
        .await
        .unwrap_err();
    assert_eq!(e.code, "show_unavailable");
    let root = std::path::Path::new(".");
    let open = |u: &str| agenium_tools::browser::url_allowed(u, &[], root, true);
    assert!(open("https://mail.google.com/").is_ok());
    assert!(open("http://localhost:3000/").is_ok());
    assert_eq!(
        open("http://192.168.1.1/").unwrap_err().code,
        "address_not_allowed"
    );
    assert_eq!(
        open("http://169.254.169.254/").unwrap_err().code,
        "address_not_allowed"
    );
    assert!(agenium_tools::browser::url_allowed(
        "http://192.168.1.1/",
        &["192.168.1.1".into()],
        root,
        true
    )
    .is_ok());
}

/// By hand (a window shows on the screen, the internet is used): `cargo test -p agenium-tools --test browser_shared
/// -- --ignored`. Google's sign-in page reads with its email field named; `show` puts the page in a window and the
/// next step goes on in it; nothing is signed in.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore]
async fn a_real_sign_in_page_reads_and_shows_in_a_window() {
    let (wt, _) = setup();
    let dir = tempfile::tempdir().unwrap();
    let profile = dir.path().join("browser");
    let c = shared_ctx(&wt.root, &profile);
    let o = b(
        &c,
        json!({"action": "open", "url": "https://mail.google.com/"}),
    )
    .await;
    println!("{}", o.chars().take(1500).collect::<String>());
    assert!(o.contains("accounts.google.com"), "{o}");
    assert!(
        o.lines()
            .any(|l| l.starts_with('[') && l.contains("\"Email or phone\"")),
        "{o}"
    );
    let shown = b(&c, json!({"action": "show"})).await;
    assert!(shown.contains("ask_question"), "{shown}");
    let o = b(&c, json!({"action": "look"})).await;
    assert!(
        o.contains("accounts.google.com"),
        "the run goes on in the window: {o}"
    );
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    drop(c);
    agenium_tools::browser::shutdown(&profile).await;
}
