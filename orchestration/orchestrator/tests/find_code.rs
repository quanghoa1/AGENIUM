//! `find_code` end to end against a mock Jev that reads each request the way the real one is asked — one criterion over
//! `items`, one question per item — and rates an item relevant when its text (a unit: its source) mentions refunds. A repository large
//! enough for the folder step; a file holding a key that must never be sent.

mod common;

use std::sync::{Arc, Mutex};

use common::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Answers every question `qK` with 0.95 when item K mentions "refund", else 0.05; keeps every request body.
async fn mock(bodies: Arc<Mutex<Vec<String>>>) -> String {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move {
        loop {
            let (mut sock, _) = l.accept().await.unwrap();
            let bodies = bodies.clone();
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut tmp = [0u8; 65536];
                let head_end = loop {
                    let n = sock.read(&mut tmp).await.unwrap();
                    if n == 0 {
                        return;
                    }
                    buf.extend_from_slice(&tmp[..n]);
                    if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        break p + 4;
                    }
                };
                let head = String::from_utf8_lossy(&buf[..head_end]).to_ascii_lowercase();
                let len: usize = head
                    .lines()
                    .find_map(|l| l.strip_prefix("content-length:"))
                    .and_then(|v| v.trim().parse().ok())
                    .unwrap_or(0);
                while buf.len() - head_end < len {
                    let n = sock.read(&mut tmp).await.unwrap();
                    buf.extend_from_slice(&tmp[..n]);
                }
                let body = String::from_utf8_lossy(&buf[head_end..]).to_string();
                bodies.lock().unwrap().push(body.clone());
                let req: serde_json::Value = serde_json::from_str(&body).unwrap();
                let items = req["state"]["items"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                let mut answers = serde_json::Map::new();
                for (id, _) in req["questions"].as_object().unwrap() {
                    let k: usize = id.trim_start_matches('q').parse().unwrap();
                    // A unit is judged on its own source; a folder or a file on everything shown of it.
                    let text = match items[k].get("source") {
                        Some(src) => src.to_string(),
                        None => items[k].to_string(),
                    };
                    let relevant = text.to_lowercase().contains("refund");
                    answers.insert(
                        id.clone(),
                        serde_json::json!({"type": "noul", "noul": if relevant { 0.95 } else { 0.05 }}),
                    );
                }
                let out =
                    serde_json::json!({ "answers": answers, "usage": {"input_tokens": 1000} })
                        .to_string();
                let resp = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{out}",
                    out.len()
                );
                sock.write_all(resp.as_bytes()).await.unwrap();
            });
        }
    });
    base
}

fn write(root: &std::path::Path, rel: &str, text: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn find_code_returns_the_code_that_does_it() {
    let f = fixture();
    let bodies = Arc::new(Mutex::new(Vec::new()));
    let base = mock(bodies.clone()).await;
    f.app
        .update_config(|c| {
            c.judge = agenium_config::JudgeSection {
                base_url: base,
                ..Default::default()
            }
        })
        .unwrap();
    let repo = tempfile::tempdir().unwrap();
    let r = repo.path();
    for d in ["ui", "auth", "search", "reports", "mail", "admin"] {
        for i in 0..12 {
            write(
                r,
                &format!("src/{d}/part{i}.rs"),
                &format!("pub fn {d}_step_{i}() -> u32 {{\n    {i}\n}}\n"),
            );
        }
    }
    write(
        r,
        "src/billing/refund.rs",
        "use crate::money::Amount;\n\n/// Pays a customer back.\npub fn issue_refund(order: u64, amount: Amount) -> Result<(), String> {\n    ledger::credit(order, amount)\n}\n\npub fn format_receipt(order: u64) -> String {\n    format!(\"receipt {order}\")\n}\n",
    );
    write(
        r,
        "src/billing/invoice.rs",
        "pub fn invoice_total() -> u32 {\n    42\n}\n",
    );
    write(
        r,
        "src/billing/keys.rs",
        "// payment provider key\nconst KEY: &str = \"sk-ant-api03-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\";\n",
    );
    write(r, ".env", "REFUND_TOKEN=secret-token-value\n");
    let out = f
        .app
        .find_code(
            r,
            ".",
            "Where is a customer refund issued?",
            Some("PI-1-T1"),
        )
        .await
        .unwrap();
    assert!(out.contains("1. src/billing/refund.rs"), "{out}");
    assert!(
        out.contains("issue_refund") && out.contains("ledger::credit(order, amount)"),
        "{out}"
    );
    assert!(
        out.contains("     4\tpub fn issue_refund"),
        "line numbers as read_file shows them: {out}"
    );
    assert!(
        !out.contains("format_receipt(order: u64)"),
        "an unrelated unit is not returned: {out}"
    );
    let sent = bodies.lock().unwrap().join("\n");
    assert!(
        sent.contains("\"src/billing/\""),
        "the folder step ran (more than 60 files)"
    );
    assert!(!sent.contains("sk-ant-api03"), "the key is never sent");
    assert!(
        sent.contains("[REDACTED"),
        "the key was masked on its way out"
    );
    assert!(!sent.contains("secret-token-value"), ".env is never read");
    assert!(
        !sent.contains("src/ui/part3.rs"),
        "an unrelated folder is not opened"
    );
    let seen = &f.app.judge_activity(1)[0];
    assert_eq!(seen.use_, "find_code");
    assert!(
        seen.outcome.contains("1 relevant file(s)") || seen.outcome.contains("relevant file"),
        "{}",
        seen.outcome
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_the_judge_find_code_says_to_use_grep() {
    let f = fixture();
    let repo = tempfile::tempdir().unwrap();
    let e = f
        .app
        .find_code(repo.path(), ".", "Where is a customer refund issued?", None)
        .await
        .unwrap_err();
    assert_eq!(e.code, "unavailable");
    assert!(e.message.contains("grep"));
}
