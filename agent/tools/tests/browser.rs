//! The run's browser (ADR-158): which pages open, and a real walk through a page when the machine has Edge or Chrome.
mod common;

use common::*;
use serde_json::json;

#[test]
fn only_this_machine_the_allowlist_and_worktree_files_open() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("wt");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("index.html"), "<p>hi</p>").unwrap();
    std::fs::write(d.path().join("outside.html"), "<p>no</p>").unwrap();
    let allow = vec!["docs.rs".to_owned()];
    let ok = |u: &str| agenium_tools::browser::url_allowed(u, &allow, &root).is_ok();
    assert!(ok("http://localhost:5173/"));
    assert!(ok("http://127.0.0.1:8080/app"));
    assert!(ok("http://[::1]:3000/"));
    assert!(ok("https://docs.rs/serde"));
    assert!(!ok("https://example.com/"));
    assert!(!ok("javascript:alert(1)"));
    assert!(!ok("chrome://settings"));
    let file = |p: &std::path::Path| reqwest_url(p);
    assert!(ok(&file(&root.join("index.html"))));
    assert!(!ok(&file(&d.path().join("outside.html"))));
}

fn reqwest_url(p: &std::path::Path) -> String {
    format!(
        "file:///{}",
        p.display()
            .to_string()
            .replace('\\', "/")
            .trim_start_matches('/')
    )
}

#[tokio::test]
async fn a_page_is_opened_used_and_shot() {
    if agenium_tools::browser::find_browser().is_none() {
        eprintln!("no Edge or Chrome on this machine: skipped");
        return;
    }
    let (wt, c) = setup();
    std::fs::write(
        wt.root.join("index.html"),
        r#"<!doctype html><title>Deck</title><h1>Cards</h1>
<input id="name" placeholder="New card"><button id="add" onclick="
  const li = document.createElement('li'); li.textContent = document.getElementById('name').value;
  document.getElementById('list').append(li); console.error('added ' + li.textContent);">Add</button>
<ul id="list"></ul>"#,
    )
    .unwrap();
    let url = reqwest_url(&wt.root.join("index.html"));
    let o = call(&c, "browser", json!({"action": "open", "url": url}))
        .await
        .unwrap();
    assert!(
        o.text.contains("Deck") && o.text.contains("Cards"),
        "{}",
        o.text
    );
    assert!(
        o.text.contains("→ #add") && o.text.contains("→ #name"),
        "{}",
        o.text
    );
    call(
        &c,
        "browser",
        json!({"action": "type", "selector": "#name", "text": "Rust"}),
    )
    .await
    .unwrap();
    let o = call(
        &c,
        "browser",
        json!({"action": "click", "selector": "#add"}),
    )
    .await
    .unwrap();
    assert!(
        o.text.contains("Console errors:") && o.text.contains("added Rust"),
        "{}",
        o.text
    );
    let v = call(
        &c,
        "browser",
        json!({"action": "eval", "expression": "document.querySelectorAll('#list li').length"}),
    )
    .await
    .unwrap();
    assert_eq!(v.text.trim(), "1");
    assert_eq!(
        call(
            &c,
            "browser",
            json!({"action": "click", "selector": "#nope"})
        )
        .await
        .unwrap_err()
        .code,
        "not_found"
    );
    let o = call(
        &c,
        "browser",
        json!({"action": "screenshot", "name": "deck"}),
    )
    .await
    .unwrap();
    assert_eq!(o.images.len(), 1);
    let png = std::fs::read(wt.root.join(".agenium/shots/deck.png")).unwrap();
    assert_eq!(&png[..4], b"\x89PNG");
    let o = call(
        &c,
        "browser",
        json!({"action": "screenshot", "name": "list", "selector": "#list"}),
    )
    .await
    .unwrap();
    assert!(o.text.contains(".agenium/shots/list.png"));
    // A host outside the allowlist is refused before any browser work.
    assert_eq!(
        call(
            &c,
            "browser",
            json!({"action": "open", "url": "https://example.com/"})
        )
        .await
        .unwrap_err()
        .code,
        "domain_not_allowed"
    );
}
