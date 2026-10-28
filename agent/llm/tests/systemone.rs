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

/// A mock that answers its requests in order with `(status, body)`, one per connection (`Connection: close`), and
/// counts them.
async fn scripted(replies: Vec<(u16, Value)>) -> (String, Arc<Mutex<usize>>) {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    let count = Arc::new(Mutex::new(0usize));
    let c2 = count.clone();
    tokio::spawn(async move {
        for (status, body) in replies {
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
            *c2.lock().unwrap() += 1;
            let text = body.to_string();
            let resp = format!(
                "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{text}",
                text.len()
            );
            sock.write_all(resp.as_bytes()).await.unwrap();
        }
    });
    (format!("http://{addr}"), count)
}

fn one_noul() -> Judging {
    Judging::default()
        .state_text("report", "done")
        .noul("ok", "Is `report` done?")
}

/// A rate limit or a busy service is retried (it no longer reaches the caller, which used to turn the judge off).
#[tokio::test]
async fn busy_answers_are_retried() {
    let (url, count) = scripted(vec![
        (429, json!({"detail": "slow down"})),
        (503, json!({"detail": "overloaded"})),
        (
            200,
            json!({"answers": {"ok": {"type": "noul", "noul": 0.9}}}),
        ),
    ])
    .await;
    let a = Judge::new(&url, "k", "m")
        .unwrap()
        .ask(&one_noul())
        .await
        .unwrap();
    assert_eq!(a["ok"].noul, Some(0.9));
    assert_eq!(*count.lock().unwrap(), 3);
}

/// A rejected key and an exhausted quota are errors the user must act on; they are not retried.
#[tokio::test]
async fn key_and_quota_failures_are_not_retried() {
    for (status, detail, want) in [
        (401, "bad key", "TypeSafe key rejected"),
        (429, "monthly quota exceeded", "TypeSafe quota exhausted"),
        (402, "add credit", "TypeSafe quota exhausted"),
    ] {
        let (url, count) = scripted(vec![(status, json!({ "detail": detail }))]).await;
        let e = Judge::new(&url, "k", "m")
            .unwrap()
            .ask(&one_noul())
            .await
            .unwrap_err();
        assert!(
            matches!(&e, agenium_llm::LlmError::Config(m) if m.starts_with(want)),
            "{e}"
        );
        assert_eq!(*count.lock().unwrap(), 1);
    }
}

/// A malformed answer counts as no answer: a probability out of range, a choice that was not offered, choice
/// probabilities that do not cover the options or do not sum to one, a choice that is not the most probable.
#[tokio::test]
async fn malformed_answers_count_as_none() {
    let mut j = Judging::default().state_text("report", "done");
    for id in ["good", "too_big", "nan"] {
        j = j.noul(id, "Is `report` done?");
    }
    let opts: std::collections::BTreeMap<String, String> = [("a", "A"), ("b", "B")]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    for id in ["pick", "unknown", "partial", "not_top"] {
        j.questions.insert(
            id.into(),
            agenium_llm::systemone::Question::Choice {
                instructions: "Which?".into(),
                criteria: opts.clone(),
            },
        );
    }
    let (url, _) = scripted(vec![(
        200,
        json!({"answers": {
            "good": {"type": "noul", "noul": 0.2},
            "too_big": {"type": "noul", "noul": 1.7},
            "nan": {"type": "noul"},
            "pick": {"type": "choice", "choice": "a", "confidence": 0.8, "probabilities": {"a": 0.8, "b": 0.2}},
            "unknown": {"type": "choice", "choice": "c", "confidence": 0.8},
            "partial": {"type": "choice", "choice": "a", "probabilities": {"a": 0.8}},
            "not_top": {"type": "choice", "choice": "b", "probabilities": {"a": 0.8, "b": 0.2}},
        }}),
    )])
    .await;
    let a = Judge::new(&url, "k", "m").unwrap().ask(&j).await.unwrap();
    let mut kept: Vec<&str> = a.keys().map(String::as_str).collect();
    kept.sort();
    assert_eq!(kept, ["good", "pick"]);
}
