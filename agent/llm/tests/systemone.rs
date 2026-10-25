//! The TypeSafe "Jev" judge client against a local mock (ADR-75): the request shape it sends and the answers it reads.

use std::sync::{Arc, Mutex};

use agenium_llm::systemone::{Judge, Judging};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// One request the mock captured.
#[derive(Clone, Default)]
struct Seen {
    path: String,
    auth: Option<String>,
    danger: Option<String>,
    body: Value,
}

async fn mock(reply: Value) -> (String, Arc<Mutex<Option<Seen>>>) {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    let seen = Arc::new(Mutex::new(None));
    let s2 = seen.clone();
    tokio::spawn(async move {
        let (mut sock, _) = l.accept().await.unwrap();
        let mut buf = Vec::new();
        let mut tmp = [0u8; 8192];
        let head_end = loop {
            let n = sock.read(&mut tmp).await.unwrap();
            buf.extend_from_slice(&tmp[..n]);
            if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                break p + 4;
            }
        };
        let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
        let mut lines = head.lines();
        let path = lines
            .next()
            .unwrap_or_default()
            .split_whitespace()
            .nth(1)
            .unwrap_or_default()
            .to_owned();
        let mut s = Seen {
            path,
            ..Default::default()
        };
        let mut len = 0usize;
        for l in lines {
            if let Some((k, v)) = l.split_once(':') {
                let (k, v) = (k.trim().to_ascii_lowercase(), v.trim().to_owned());
                match k.as_str() {
                    "authorization" => s.auth = Some(v),
                    "typesafe-danger-zone-images" => s.danger = Some(v),
                    "content-length" => len = v.parse().unwrap_or(0),
                    _ => {}
                }
            }
        }
        while buf.len() - head_end < len {
            let n = sock.read(&mut tmp).await.unwrap();
            buf.extend_from_slice(&tmp[..n]);
        }
        s.body = serde_json::from_slice(&buf[head_end..head_end + len]).unwrap_or(Value::Null);
        *s2.lock().unwrap() = Some(s);
        let body = reply.to_string();
        let resp = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        sock.write_all(resp.as_bytes()).await.unwrap();
    });
    (format!("http://{addr}"), seen)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn judges_text_in_one_request() {
    let (url, seen) = mock(json!({
        "model": "jev-1.14.0",
        "answers": {
            "ac1": {"type": "noul", "noul": 0.95},
            "ac2": {"type": "noul", "noul": 0.1}
        }
    }))
    .await;
    let judge = Judge::new(&url, "k3y", "jev-latest").unwrap();
    let j = Judging::default()
        .state_text("report", "added hello.txt; test passed")
        .noul("ac1", "Does `report` create hello.txt?")
        .noul("ac2", "Does `report` delete the database?");
    let answers = judge.ask(&j).await.unwrap();
    assert!(answers["ac1"].is_true(0.7));
    assert!(!answers["ac2"].is_true(0.7));

    let s = seen.lock().unwrap().clone().unwrap();
    assert_eq!(s.path, "/v1/systemone");
    assert_eq!(s.auth.as_deref(), Some("Bearer k3y"));
    assert!(s.danger.is_none(), "no image: no danger-zone header");
    assert_eq!(s.body["model"], "jev-latest");
    assert_eq!(s.body["state"]["report"], "added hello.txt; test passed");
    assert_eq!(s.body["questions"]["ac1"]["type"], "noul");
    assert!(s.body["assets"].is_null());
}
