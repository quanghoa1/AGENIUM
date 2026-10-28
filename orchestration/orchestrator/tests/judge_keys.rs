//! The judge's keys (ADR-128 §2): the keys in use take turns, one request each; a key out of quota is set aside, the
//! user is told which one by name, and it stays out until the user re-enables it; a failing key rests (30 minutes, then
//! an hour) and comes back by itself; the request a key failed goes on with another key; with every key out of quota
//! the judge is off and Agenium works on without it (fail-open). Against a mock Jev that answers per key.

mod common;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use common::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// A mock Jev: the reply for each key (by its value) is `(status, body)`, changeable while it runs; it records the key
/// of every request in order.
#[derive(Clone, Default)]
struct Mock {
    replies: Arc<Mutex<HashMap<String, (u16, String)>>>,
    seen: Arc<Mutex<Vec<String>>>,
}

const WORTH: &str = r#"{"answers":{"worth":{"type":"noul","noul":0.9}}}"#;
const QUOTA: &str = r#"{"detail":"monthly quota exceeded: add credit"}"#;
const REJECTED: &str = r#"{"detail":"invalid api key"}"#;

impl Mock {
    fn set(&self, key: &str, status: u16, body: &str) {
        self.replies
            .lock()
            .unwrap()
            .insert(key.to_owned(), (status, body.to_owned()));
    }

    fn seen(&self) -> Vec<String> {
        self.seen.lock().unwrap().clone()
    }

    async fn start(&self) -> String {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", l.local_addr().unwrap());
        let me = self.clone();
        tokio::spawn(async move {
            loop {
                let (mut sock, _) = l.accept().await.unwrap();
                let me = me.clone();
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
                    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
                    let field = |name: &str| {
                        head.lines()
                            .find_map(|l| {
                                let (k, v) = l.split_once(':')?;
                                k.trim()
                                    .eq_ignore_ascii_case(name)
                                    .then(|| v.trim().to_owned())
                            })
                            .unwrap_or_default()
                    };
                    let len: usize = field("content-length").parse().unwrap_or(0);
                    while buf.len() - head_end < len {
                        let n = sock.read(&mut tmp).await.unwrap();
                        buf.extend_from_slice(&tmp[..n]);
                    }
                    let key = field("authorization")
                        .trim_start_matches("Bearer ")
                        .to_owned();
                    me.seen.lock().unwrap().push(key.clone());
                    let (status, body) = me
                        .replies
                        .lock()
                        .unwrap()
                        .get(&key)
                        .cloned()
                        .unwrap_or((401, REJECTED.into()));
                    let resp = format!(
                        "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    sock.write_all(resp.as_bytes()).await.unwrap();
                });
            }
        });
        base
    }
}

/// A judge on `base` with keys named `names`, each with the value `<name>-value-0123456789`.
fn judge(f: &Fixture, base: String, names: &[&str]) {
    f.app
        .update_config(|c| {
            c.judge = agenium_config::JudgeSection {
                base_url: base,
                ..Default::default()
            }
        })
        .unwrap();
    for n in names {
        f.app.add_judge_key(n, &value(n)).unwrap();
    }
}

fn value(name: &str) -> String {
    format!("{name}-value-0123456789")
}

/// One ask through a real use (the memory gate); each episode text differs, so no answer comes from the cache.
async fn ask(f: &Fixture, i: usize) -> Option<bool> {
    f.app
        .worth_remembering(&format!("episode {i}: fixed the parser"))
        .await
}

fn notices(f: &Fixture) -> Vec<String> {
    f.app
        .store()
        .read(|c| agenium_store::core::events::recent_of_kind(c, "NeedsUser", 50))
        .unwrap()
        .into_iter()
        .filter_map(|e| e.payload["reason"].as_str().map(str::to_owned))
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_keys_take_turns() {
    let f = fixture();
    let mock = Mock::default();
    mock.set(&value("alpha"), 200, WORTH);
    mock.set(&value("beta"), 200, WORTH);
    judge(&f, mock.start().await, &["alpha", "beta"]);
    for i in 0..4 {
        assert_eq!(ask(&f, i).await, Some(true));
    }
    let seen = mock.seen();
    assert_eq!(seen.len(), 4);
    assert!(
        seen.iter().filter(|k| **k == value("alpha")).count() == 2,
        "{seen:?}"
    );
    assert!(
        seen.windows(2).all(|w| w[0] != w[1]),
        "alternating: {seen:?}"
    );
    assert!(f.app.judge_activity(4).iter().all(|a| a.key.is_some()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_key_out_of_quota_is_set_aside_until_the_user_re_enables_it() {
    let f = fixture();
    let mock = Mock::default();
    mock.set(&value("alpha"), 429, QUOTA);
    mock.set(&value("beta"), 200, WORTH);
    judge(&f, mock.start().await, &["alpha", "beta"]);
    // Whichever key is asked first, every ask is answered: alpha's request goes on with beta.
    for i in 0..4 {
        assert_eq!(ask(&f, i).await, Some(true), "ask {i}");
    }
    let alpha_requests = mock.seen().iter().filter(|k| **k == value("alpha")).count();
    assert_eq!(
        alpha_requests,
        1,
        "alpha is asked once, then set aside: {:?}",
        mock.seen()
    );
    let h = f.app.judge_health();
    assert!(h.keys_off["alpha"].quota && h.keys_off["alpha"].until.is_none());
    assert!(!h.disabled);
    let told = notices(&f);
    assert_eq!(told.len(), 1, "{told:?}");
    assert!(
        told[0].contains("Jev key \"alpha\" is out of quota") && told[0].contains("beta"),
        "{}",
        told[0]
    );
    // Credit added: the user re-enables alpha, and it takes turns again.
    mock.set(&value("alpha"), 200, WORTH);
    f.app.enable_judge(Some("alpha")).await.unwrap();
    assert!(f.app.judge_health().keys_off.is_empty());
    let before = mock.seen().len();
    for i in 10..14 {
        assert_eq!(ask(&f, i).await, Some(true));
    }
    let after: Vec<String> = mock.seen()[before..].to_vec();
    assert!(
        after.contains(&value("alpha")) && after.contains(&value("beta")),
        "{after:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn with_every_key_out_of_quota_the_judge_is_off_and_agenium_goes_on() {
    let f = fixture();
    let mock = Mock::default();
    mock.set(&value("alpha"), 429, QUOTA);
    mock.set(&value("beta"), 402, r#"{"detail":"payment required"}"#);
    judge(&f, mock.start().await, &["alpha", "beta"]);
    // Fail-open: no answer, the caller extracts memory as usual.
    assert_eq!(ask(&f, 0).await, None);
    assert!(f.app.judge().unwrap().is_none(), "the judge is off");
    let h = f.app.judge_health();
    assert!(h.disabled && h.keys_off.values().all(|k| k.quota));
    let told = notices(&f);
    assert_eq!(told.len(), 2, "{told:?}");
    assert!(
        told.iter()
            .any(|t| t.contains("Every Jev key is now out of quota: the judge is off")),
        "{told:?}"
    );
    // Nothing is retried by itself.
    let n = mock.seen().len();
    assert_eq!(ask(&f, 1).await, None);
    assert_eq!(mock.seen().len(), n, "no request while every key is out");
    // Re-enabled after credit: the judge is back.
    mock.set(&value("beta"), 200, WORTH);
    f.app.enable_judge(Some("beta")).await.unwrap();
    assert!(!f.app.judge_health().disabled);
    assert_eq!(ask(&f, 2).await, Some(true));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failing_key_rests_then_comes_back_by_itself() {
    let f = fixture();
    let mock = Mock::default();
    mock.set(&value("alpha"), 401, REJECTED);
    mock.set(&value("beta"), 200, WORTH);
    judge(&f, mock.start().await, &["alpha", "beta"]);
    for i in 0..3 {
        assert_eq!(ask(&f, i).await, Some(true));
    }
    let off = f.app.judge_health().keys_off["alpha"].clone();
    assert!(!off.quota && off.failures == 1);
    let until = chrono::DateTime::parse_from_rfc3339(off.until.as_deref().unwrap()).unwrap();
    let rest = until
        .signed_duration_since(chrono::Utc::now())
        .num_minutes();
    assert!((28..=30).contains(&rest), "rests 30 minutes: {rest}");
    assert!(
        notices(&f).is_empty(),
        "a failing key comes back by itself: no notice"
    );
    // The rest is over: alpha takes turns again; still failing, it rests an hour.
    let mut h = f.app.judge_health();
    h.keys_off.get_mut("alpha").unwrap().until =
        Some((chrono::Utc::now() - chrono::Duration::minutes(1)).to_rfc3339());
    std::fs::write(
        f.app.layout().judge_health(),
        serde_json::to_string(&h).unwrap(),
    )
    .unwrap();
    for i in 10..12 {
        assert_eq!(ask(&f, i).await, Some(true));
    }
    let off = f.app.judge_health().keys_off["alpha"].clone();
    assert_eq!(off.failures, 2);
    let until = chrono::DateTime::parse_from_rfc3339(off.until.as_deref().unwrap()).unwrap();
    let rest = until
        .signed_duration_since(chrono::Utc::now())
        .num_minutes();
    assert!((58..=60).contains(&rest), "then an hour: {rest}");
    // Fixed in the meantime: once its rest is over and it answers, it is back for good.
    mock.set(&value("alpha"), 200, WORTH);
    let mut h = f.app.judge_health();
    h.keys_off.get_mut("alpha").unwrap().until =
        Some((chrono::Utc::now() - chrono::Duration::minutes(1)).to_rfc3339());
    std::fs::write(
        f.app.layout().judge_health(),
        serde_json::to_string(&h).unwrap(),
    )
    .unwrap();
    let before = mock.seen().len();
    for i in 20..24 {
        assert_eq!(ask(&f, i).await, Some(true));
    }
    assert!(mock.seen()[before..].contains(&value("alpha")));
}

#[test]
fn keys_live_in_the_vault_and_have_names() {
    let f = fixture();
    f.app
        .add_judge_key("team key", "ts-secret-value-123")
        .unwrap();
    let toml = std::fs::read_to_string(f.app.layout().config()).unwrap();
    assert!(
        toml.contains("team key") && !toml.contains("ts-secret-value-123"),
        "{toml}"
    );
    assert!(f.app.add_judge_key("", "x").is_err());
    assert!(f.app.add_judge_key("x", " ").is_err());
    f.app.remove_judge_key("team key").unwrap();
    assert!(f.app.config().judge.key_list().is_empty());
    assert!(f.app.remove_judge_key("team key").is_err());
}
