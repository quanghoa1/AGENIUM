//! The optional judge (Jev): settings, keys, each use only when on, shadow mode, redaction.

use super::*;

/// The judge runs in two modes (ADR-81): every use is on by default, but without a key the judge is off and Agenium
/// works as before; with a key it is on, and with every key out of quota it is off again.
#[test]
fn the_judge_is_optional_and_off_without_a_usable_key() {
    let f = fixture();
    let d = agenium_config::JudgeSection::default();
    assert!(
        d.review_assist
            && d.skill_hint
            && d.memory_gate
            && d.skill_quality
            && d.done_check
            && d.find_code
    );
    assert!(f.app.judge().unwrap().is_none(), "off without a key");
    f.app.add_judge_key("main", "ts-test-key").unwrap();
    assert!(f.app.judge().unwrap().is_some(), "on with a key");
    let mut h = f.app.judge_health();
    h.keys_off.insert(
        "main".into(),
        agenium_orchestrator::judge::KeyOff {
            quota: true,
            reason: "TypeSafe quota exhausted".into(),
            at: String::new(),
            until: None,
            failures: 1,
        },
    );
    std::fs::write(
        f.app.layout().judge_health(),
        serde_json::to_string(&h).unwrap(),
    )
    .unwrap();
    assert!(
        f.app.judge().unwrap().is_none(),
        "out of quota until re-enabled"
    );
}

/// The judge is set from the app: its key goes to the vault (never to config.toml), a saved key is kept when the form
/// leaves the field empty, without a key it stays off, and an empty endpoint turns it off.
#[test]
fn the_judge_is_set_from_the_app_with_its_key_in_the_vault() {
    use agenium_orchestrator::judge::{JudgeInput, JUDGE_SECRET};
    let f = fixture();
    let form = |key: Option<&str>| JudgeInput {
        base_url: "https://api.typesafe.ai/".into(),
        model: "jev-1.14.0".into(),
        api_key: key.map(str::to_owned),
        review_assist: true,
        skill_hint: true,
        ..Default::default()
    };
    f.app.set_judge(&form(None)).unwrap();
    assert!(
        f.app.judge().unwrap().is_none(),
        "without a key it stays off"
    );
    let j = f.app.set_judge(&form(Some("ts-test-key"))).unwrap();
    assert_eq!(
        (j.base_url.as_str(), j.api_key_ref.as_str()),
        ("https://api.typesafe.ai", JUDGE_SECRET)
    );
    assert!(j.review_assist && j.skill_hint && !j.memory_gate);
    let stored = f.app.secret(JUDGE_SECRET).unwrap().unwrap();
    assert_eq!(stored.expose_str().unwrap(), "ts-test-key");
    let toml = std::fs::read_to_string(f.app.layout().config()).unwrap();
    assert!(
        toml.contains("judge/typesafe") && !toml.contains("ts-test-key"),
        "{toml}"
    );
    assert!(f.app.judge().unwrap().is_some());
    // Saved again without a key: the stored one stays.
    assert_eq!(
        f.app.set_judge(&form(None)).unwrap().api_key_ref,
        JUDGE_SECRET
    );
    assert!(f
        .app
        .set_judge(&JudgeInput {
            base_url: "http://example.com".into(),
            ..form(None)
        })
        .is_err());
    // Plain HTTP only to this machine: the host is compared whole (the key would go to another host in clear).
    for remote in [
        "http://localhost.evil.example",
        "http://127.0.0.1.evil.example/v1",
        "http://localhost@evil.example",
        "http://[::1].evil.example",
    ] {
        let j = JudgeInput {
            base_url: remote.into(),
            ..form(None)
        };
        assert!(f.app.set_judge(&j).is_err(), "{remote}");
    }
    for local in [
        "http://localhost:8080",
        "http://127.0.0.1:9000/v1",
        "http://[::1]:8080",
    ] {
        let j = JudgeInput {
            base_url: local.into(),
            ..form(None)
        };
        assert_eq!(f.app.set_judge(&j).unwrap().base_url, local);
    }
    // Off.
    f.app
        .set_judge(&JudgeInput {
            base_url: String::new(),
            ..form(None)
        })
        .unwrap();
    assert!(f.app.judge().unwrap().is_none());
}

/// ADR-82: the quick skill pick is fail-open. Off by default and with the flag off it returns nothing, and it never
/// asks the judge when there are fewer than two skills to choose between — so the worker always runs.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn skill_hint_is_off_by_default_and_needs_two_skills() {
    let f = fixture();
    let a = solo_with_plan(&f, &[("PI-1", &[])]);
    let k = started(&f, a.id, "PI-1", "Refactor the parser", &[], &[]);
    let t = ticket(&f, &k.to_string());
    let two = [
        (
            "refactor".to_string(),
            "restructure code safely".to_string(),
        ),
        ("docs".to_string(), "write documentation".to_string()),
    ];
    // Off by default.
    assert!(f.app.skill_hint(&t, &two).await.is_none());
    // On, but asked with a single skill: no question, no hint.
    f.app
        .update_config(|c| {
            c.judge = agenium_config::JudgeSection {
                base_url: "http://127.0.0.1:1".into(),
                model: "jev-latest".into(),
                skill_hint: true,
                ..Default::default()
            }
        })
        .unwrap();
    assert!(
        f.app.skill_hint(&t, &two[..1]).await.is_none(),
        "one skill never reaches the judge"
    );
}

/// ADR-82: with the judge on and confident, the pick is prepended as a hint naming the chosen skill; a "none" pick
/// adds nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn skill_hint_prepends_the_judges_pick() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    // A one-shot mock Jev that replies with the given answer body to the first request.
    async fn mock(answer: &'static str) -> String {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut sock, _) = l.accept().await.unwrap();
            let mut buf = [0u8; 8192];
            // Read until the request headers end; the body is irrelevant to the mock.
            loop {
                let n = sock.read(&mut buf).await.unwrap();
                if n == 0 || buf[..n].windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            let body = answer.as_bytes();
            let resp = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n",
                body.len()
            );
            sock.write_all(resp.as_bytes()).await.unwrap();
            sock.write_all(body).await.unwrap();
            sock.flush().await.unwrap();
        });
        format!("http://{addr}")
    }

    let two = [
        (
            "refactor".to_string(),
            "restructure code safely".to_string(),
        ),
        ("docs".to_string(), "write documentation".to_string()),
    ];

    let f = fixture();
    let a = solo_with_plan(&f, &[("PI-1", &[])]);
    let k = started(&f, a.id, "PI-1", "Refactor the parser", &[], &[]);
    let t = ticket(&f, &k.to_string());

    // A confident pick becomes a hint naming the skill.
    let base =
        mock(r#"{"answers":{"skill":{"type":"choice","choice":"refactor","confidence":0.95}}}"#)
            .await;
    f.app
        .update_config(|c| {
            c.judge = agenium_config::JudgeSection {
                base_url: base,
                model: "jev-latest".into(),
                skill_hint: true,
                ..Default::default()
            }
        })
        .unwrap();
    let hint = f.app.skill_hint(&t, &two).await.expect("a confident pick");
    assert!(hint.contains("refactor"), "names the picked skill: {hint}");
    // Each ask is in the judge's activity, for the user to see.
    let seen = &f.app.judge_activity(5)[0];
    assert_eq!((seen.use_.as_str(), seen.ok), ("skill_hint", true));
    assert_eq!(seen.ticket.as_deref(), Some(k.as_str()));
    assert!(
        seen.outcome.contains("suggested `refactor` (95%)"),
        "{}",
        seen.outcome
    );

    // "none" (or a low-confidence pick) adds no hint.
    let base =
        mock(r#"{"answers":{"skill":{"type":"choice","choice":"none","confidence":0.9}}}"#).await;
    f.app.update_config(|c| c.judge.base_url = base).unwrap();
    assert!(
        f.app.skill_hint(&t, &two).await.is_none(),
        "a 'none' pick is not a hint"
    );
}

/// ADR-75: before a review the judge reads the worker's report against each criterion. Off by default; on, each
/// criterion gets a verdict with its probability (found / unclear / no evidence) for the review task, and a criterion
/// the judge did not answer is left out.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review_assist_reads_each_criterion_only_when_on() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    // A one-shot mock Jev that replies with the given answer body to the first request.
    async fn mock(answer: &'static str) -> String {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut sock, _) = l.accept().await.unwrap();
            let mut buf = [0u8; 8192];
            loop {
                let n = sock.read(&mut buf).await.unwrap();
                if n == 0 || buf[..n].windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            let body = answer.as_bytes();
            let resp = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n",
                body.len()
            );
            sock.write_all(resp.as_bytes()).await.unwrap();
            sock.write_all(body).await.unwrap();
            sock.flush().await.unwrap();
        });
        format!("http://{addr}")
    }

    let f = fixture();
    let a = solo_with_plan(&f, &[("PI-1", &[])]);
    let k = started(&f, a.id, "PI-1", "Write the greeting", &[], &[]);
    let t = ticket(&f, &k.to_string());
    let ac = t.acceptance[0].id.clone();
    let report = "Wrote deliverables/hello.txt with the greeting; read it back.";
    assert!(
        f.app.review_evidence(&t, report).await.is_none(),
        "off by default"
    );

    let base = mock(r#"{"answers":{"AC-1":{"type":"noul","noul":0.92}}}"#).await;
    f.app
        .update_config(|c| {
            c.judge = agenium_config::JudgeSection {
                base_url: base,
                model: "jev-latest".into(),
                review_assist: true,
                ..Default::default()
            }
        })
        .unwrap();
    let read = f.app.review_evidence(&t, report).await.expect("a read");
    assert_eq!(ac, "AC-1");
    assert!(read.contains("- AC-1: evidence found (92%)"), "{read}");
    let seen = &f.app.judge_activity(5)[0];
    assert_eq!(seen.use_, "review_assist");
    assert_eq!(seen.outcome, "AC-1: evidence found (92%)");

    let base = mock(r#"{"answers":{"AC-1":{"type":"noul","noul":0.5}}}"#).await;
    f.app.update_config(|c| c.judge.base_url = base).unwrap();
    let read = f.app.review_evidence(&t, report).await.expect("a read");
    assert!(read.contains("- AC-1: unclear (50%)"), "{read}");

    let base = mock(r#"{"answers":{}}"#).await;
    f.app.update_config(|c| c.judge.base_url = base).unwrap();
    assert!(
        f.app.review_evidence(&t, report).await.is_none(),
        "no answer, no read"
    );
}

/// ADR-85: the memory gate is fail-open. Off by default it returns None (extract as usual); on, it returns Some(false)
/// only when the judge is confident nothing is worth remembering, Some(true) when it is, and None when unsure.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn memory_gate_skips_only_on_a_confident_no() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    async fn mock(answer: &'static str) -> String {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut sock, _) = l.accept().await.unwrap();
            let mut buf = [0u8; 8192];
            loop {
                let n = sock.read(&mut buf).await.unwrap();
                if n == 0 || buf[..n].windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            let body = answer.as_bytes();
            let resp = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n",
                body.len()
            );
            sock.write_all(resp.as_bytes()).await.unwrap();
            sock.write_all(body).await.unwrap();
            sock.flush().await.unwrap();
        });
        format!("http://{addr}")
    }

    let f = fixture();
    // Off by default: None, so the caller extracts as usual.
    assert_eq!(f.app.worth_remembering("routine work").await, None);

    let on = |base: String| {
        move |c: &mut agenium_config::Config| {
            c.judge = agenium_config::JudgeSection {
                base_url: base.clone(),
                model: "jev-latest".into(),
                memory_gate: true,
                ..Default::default()
            }
        }
    };
    // Confident "no" (0.05): skip is allowed.
    f.app
        .update_config(on(mock(
            r#"{"answers":{"worth":{"type":"noul","noul":0.05}}}"#,
        )
        .await))
        .unwrap();
    assert_eq!(f.app.worth_remembering("routine work").await, Some(false));
    // Confident "yes" (0.95): do not skip.
    f.app
        .update_config(on(mock(
            r#"{"answers":{"worth":{"type":"noul","noul":0.95}}}"#,
        )
        .await))
        .unwrap();
    assert_eq!(
        f.app.worth_remembering("a new convention").await,
        Some(true)
    );
    // Unsure (0.5): None, extract as usual.
    f.app
        .update_config(on(mock(
            r#"{"answers":{"worth":{"type":"noul","noul":0.5}}}"#,
        )
        .await))
        .unwrap();
    assert_eq!(f.app.worth_remembering("maybe").await, None);
}

/// ADR-92 with and without the judge: off by default the judge layer reports nothing (the deterministic gates decide);
/// on, a confident contradiction names the closest skill.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_judge_reads_contradictions_only_when_on() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let t = team();
    let f = &t.f;
    let text = "## Steps\n1. Skip the regression test when the fix is small.\n";
    let subject = "Ship small fixes quickly without a regression test.";
    assert!(
        f.app
            .judged_contradictions(t.web, "x", subject, text)
            .await
            .is_empty(),
        "off by default"
    );

    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut sock, _) = l.accept().await.unwrap();
        let mut buf = [0u8; 16384];
        loop {
            let n = sock.read(&mut buf).await.unwrap();
            if n == 0 || buf[..n].windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        let body = br#"{"answers":{"c0":{"type":"noul","noul":0.92},"c1":{"type":"noul","noul":0.1},"c2":{"type":"noul","noul":0.05}}}"#;
        let head = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n",
            body.len()
        );
        sock.write_all(head.as_bytes()).await.unwrap();
        sock.write_all(body).await.unwrap();
        sock.flush().await.unwrap();
    });
    f.app
        .update_config(|c| {
            c.judge = agenium_config::JudgeSection {
                base_url: format!("http://{addr}"),
                model: "jev-latest".into(),
                skill_quality: true,
                ..Default::default()
            }
        })
        .unwrap();
    let hits = f.app.judged_contradictions(t.web, "x", subject, text).await;
    assert_eq!(hits.len(), 1, "{hits:?}");
    let lib = f.app.skill_registry().library();
    assert!(
        lib.get(&hits[0]).is_some(),
        "names a skill the agent can use: {hits:?}"
    );
}

/// ADR-126: a report that reads as done while nothing checked the work after its last change goes back once — only
/// with the judge on and `done_check` enabled, only when a check applies, never when a check passed after the change.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unverified_done_goes_back_only_when_the_judge_is_sure() {
    use agenium_orchestrator::judge::done_check::run_facts;
    use agenium_store::core::runs::LogEntry;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    async fn mock(answer: &'static str) -> String {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut sock, _) = l.accept().await.unwrap();
            let mut buf = [0u8; 8192];
            loop {
                let n = sock.read(&mut buf).await.unwrap();
                if n == 0 || buf[..n].windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            let resp = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{answer}",
                answer.len()
            );
            sock.write_all(resp.as_bytes()).await.unwrap();
        });
        format!("http://{addr}")
    }
    let entry = |n: u32, tool: &str, args: serde_json::Value, result: &str| LogEntry {
        n,
        kind: "tool".into(),
        data: serde_json::json!({ "tool": tool, "args": args, "ok": true, "result": result }),
        at: String::new(),
    };

    let f = fixture();
    let a = solo_with_plan(&f, &[("PI-1", &[])]);
    let k = started(&f, a.id, "PI-1", "Fix the parser", &[], &[]);
    let t = ticket(&f, &k.to_string());
    let report: agenium_core::forms::WorkReport = serde_json::from_value(serde_json::json!({
        "ticket_id": k.to_string(),
        "summary": "Fixed the parser; all tests pass.",
        "confidence": 0.9,
        "acceptance": t.acceptance.iter().map(|ac| serde_json::json!({"ac_id": ac.id, "status": "Met", "evidence": "fixed"})).collect::<Vec<_>>(),
    }))
    .unwrap();
    let unchecked = run_facts(&[entry(
        1,
        "edit_file",
        serde_json::json!({"path": "src/parser.rs"}),
        "edited",
    )]);
    let sure = r#"{"answers":{"claims_done":{"type":"noul","noul":0.95},"check_applies":{"type":"noul","noul":0.9},"claims_checked":{"type":"noul","noul":0.85}}}"#;

    // Judge off: nothing is asked, the report goes through.
    assert!(f
        .app
        .unverified_done(&t, &report, &unchecked)
        .await
        .is_none());

    let on = |base: String| {
        f.app
            .update_config(|c| {
                c.judge = agenium_config::JudgeSection {
                    base_url: base,
                    model: "jev-latest".into(),
                    done_check: true,
                    ..Default::default()
                }
            })
            .unwrap()
    };
    on(mock(sure).await);
    let msg = f
        .app
        .unverified_done(&t, &report, &unchecked)
        .await
        .expect("a confident unverified done goes back");
    assert!(
        msg.contains("no command ran after your last change"),
        "{msg}"
    );
    assert!(msg.contains("says checks passed"), "{msg}");
    let seen = &f.app.judge_activity(1)[0];
    assert_eq!(seen.use_, "done_check");
    assert!(seen.outcome.starts_with("sent back"), "{}", seen.outcome);

    // A check that passed after the last change settles it without asking.
    let checked = run_facts(&[
        entry(
            1,
            "edit_file",
            serde_json::json!({"path": "src/parser.rs"}),
            "edited",
        ),
        entry(
            2,
            "shell",
            serde_json::json!({"command": "cargo test"}),
            "exit code 0\nok",
        ),
    ]);
    let before = f.app.judge_activity(50).len();
    assert!(f.app.unverified_done(&t, &report, &checked).await.is_none());
    assert_eq!(f.app.judge_activity(50).len(), before, "not asked");

    // Not sure a check applies (prose, notes): accepted.
    on(mock(r#"{"answers":{"claims_done":{"type":"noul","noul":0.95},"check_applies":{"type":"noul","noul":0.2},"claims_checked":{"type":"noul","noul":0.1}}}"#).await);
    assert!(f
        .app
        .unverified_done(&t, &report, &unchecked)
        .await
        .is_none());
    assert!(f.app.judge_activity(1)[0].outcome.starts_with("accepted"));
}

/// ADR-127: the done check asks the review's questions in its own request; the review run then reads them from the
/// answer cache and makes no request (the mock answers once and is gone).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_review_reads_answers_prefetched_at_submit() {
    let f = fixture();
    let a = solo_with_plan(&f, &[("PI-1", &[])]);
    let k = started(&f, a.id, "PI-1", "Fix the parser", &[], &[]);
    let t = ticket(&f, &k.to_string());
    assert!(!t.acceptance.is_empty());
    let (report, facts) = unchecked_report(&t);
    let mut answers = serde_json::json!({
        "claims_done": {"type": "noul", "noul": 0.95},
        "check_applies": {"type": "noul", "noul": 0.2},
        "claims_checked": {"type": "noul", "noul": 0.1},
    });
    for ac in &t.acceptance {
        answers[&ac.id] = serde_json::json!({"type": "noul", "noul": 0.9});
    }
    let base = jev_once(serde_json::json!({ "answers": answers }).to_string()).await;
    judge_on(&f, base, |j| {
        j.done_check = true;
        j.review_assist = true;
    });
    assert!(
        f.app.unverified_done(&t, &report, &facts).await.is_none(),
        "a check does not apply: accepted"
    );
    let text = agenium_orchestrator::judge::report_text(&report);
    let evidence = f
        .app
        .review_evidence(&t, &text)
        .await
        .expect("answered from the cache");
    assert!(evidence.contains("evidence found (90%)"), "{evidence}");
    let seen = &f.app.judge_activity(1)[0];
    assert_eq!((seen.use_.as_str(), seen.cached), ("review_assist", true));
}

/// ADR-127: in shadow mode a use asks and records what it would do, and changes nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shadow_mode_records_and_changes_nothing() {
    let f = fixture();
    let a = solo_with_plan(&f, &[("PI-1", &[])]);
    let k = started(&f, a.id, "PI-1", "Fix the parser", &[], &[]);
    let t = ticket(&f, &k.to_string());
    let (report, facts) = unchecked_report(&t);
    let base = jev_once(
        r#"{"answers":{"claims_done":{"type":"noul","noul":0.95},"check_applies":{"type":"noul","noul":0.9},"claims_checked":{"type":"noul","noul":0.1}},"usage":{"input_tokens":512}}"#.into(),
    )
    .await;
    judge_on(&f, base, |j| {
        j.done_check = true;
        j.shadow = true;
    });
    assert!(
        f.app.unverified_done(&t, &report, &facts).await.is_none(),
        "nothing sent back"
    );
    let seen = &f.app.judge_activity(1)[0];
    assert!(seen.shadow);
    assert!(
        seen.outcome.starts_with("[shadow] sent back"),
        "{}",
        seen.outcome
    );
    assert_eq!(
        seen.usage,
        Some(serde_json::json!({"input_tokens": 512})),
        "billed usage kept"
    );
}

/// ADR-127: a long result loses only the chunks the judge is confident are not needed and that rank low; the head,
/// the tail and anything with an error stay.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_long_log_loses_only_confidently_unneeded_chunks() {
    let f = fixture();
    // 150 lines = 5 chunks of 30; chunk 3 has a failure (always kept), so chunks 1 and 2 are asked about.
    let mut lines: Vec<String> = (0..150)
        .map(|i| format!("   Compiling crate{i} v1.0.0"))
        .collect();
    lines[0] = "exit code 101".into();
    lines[100] = "test parser::cut ... FAILED".into();
    let text = lines.join("\n");
    let base = jev_once(
        r#"{"answers":{"chunk_1":{"type":"noul","noul":0.05},"chunk_2":{"type":"noul","noul":0.6}}}"#.into(),
    )
    .await;
    judge_on(&f, base, |j| j.trim_output = true);
    let cut = f
        .app
        .trim_output("Fix the parser", "PI-1-T1", "shell", "cargo test", &text)
        .await
        .expect("chunk 1 goes");
    assert!(
        cut.contains("[... 30 lines left out by the output trimmer"),
        "{cut}"
    );
    assert!(cut.starts_with("exit code 101"));
    assert!(cut.contains("FAILED") && cut.contains("crate149") && cut.contains("crate60"));
    assert!(!cut.contains("crate45\n") && !cut.contains("crate31\n"));
    assert!(f.app.judge_activity(1)[0]
        .outcome
        .contains("left out 30 of 150 lines"));
}

/// ADR-127: no secret reaches the judge — every text passes the redactor in the one place all uses ask through.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn secrets_never_reach_the_judge() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let seen = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    let s2 = seen.clone();
    tokio::spawn(async move {
        let (mut sock, _) = l.accept().await.unwrap();
        let mut buf = Vec::new();
        let mut tmp = [0u8; 65536];
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
        *s2.lock().unwrap() = String::from_utf8_lossy(&buf[head_end..]).to_string();
        let answer = r#"{"answers":{"check_applies":{"type":"noul","noul":0.2},"claims_checked":{"type":"noul","noul":0.1}}}"#;
        let resp = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{answer}",
            answer.len()
        );
        sock.write_all(resp.as_bytes()).await.unwrap();
    });
    let f = fixture();
    let a = solo_with_plan(&f, &[("PI-1", &[])]);
    let k = started(&f, a.id, "PI-1", "Fix the parser", &[], &[]);
    let t = ticket(&f, &k.to_string());
    let secret = "s3cr3t-deploy-token-0123456789";
    f.app.redactor().register("deploy/token", secret);
    let (mut report, facts) = unchecked_report(&t);
    report.summary = format!("Deployed with DEPLOY_TOKEN={secret}; all tests pass.");
    judge_on(&f, base, |j| j.done_check = true);
    f.app.unverified_done(&t, &report, &facts).await;
    let body = seen.lock().unwrap().clone();
    assert!(
        body.contains("Deployed with"),
        "the request was made: {body}"
    );
    assert!(
        !body.contains(secret),
        "the secret reached the judge: {body}"
    );
    assert!(body.contains("[REDACTED"), "{body}");
}
