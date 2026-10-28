//! Messages during a run, parallel read-only calls, notes on files, images.

use super::*;

/// ADR-104: messages put in the run's queue reach the model before its next request.
#[tokio::test]
async fn messages_sent_during_the_run_reach_the_next_request() {
    let (_t, ctx, tools, proto) = harness();
    let q: Steer = Default::default();
    let p = ScriptedProvider::new("m");
    let q2 = q.clone();
    p.push(Box::new(move |_| {
        q2.lock().unwrap().push("Use tabs.".into());
        Ok(agenium_llm::mock::calls_response(
            "",
            vec![("list_dir", json!({}))],
        ))
    }));
    p.push(Box::new(|req: &ChatRequest| {
        assert_eq!(
            req.messages.last().unwrap().text(),
            format!("{STEERED}Use tabs.")
        );
        Ok(agenium_llm::mock::calls_response(
            "",
            vec![("submit_report", json!({"ticket_id": "AGN-1"}))],
        ))
    }));
    let mut r = run(&p, &ctx, &tools, &proto, cfg());
    r.steer = Some(q.clone());
    assert_eq!(r.run(&order()).await.outcome, Outcome::Completed);
    assert!(q.lock().unwrap().is_empty());
}

/// ADR-105: read-only calls of one turn run together, results stay in call order; a number sent as a string arrives
/// as a number.
#[tokio::test]
async fn read_only_calls_run_together_and_arguments_are_repaired() {
    let (_t, ctx, mut tools, proto) = harness();
    let (now, peak) = (Arc::default(), Arc::default());
    tools.push(Arc::new(Slow(Arc::clone(&now), Arc::clone(&peak))));
    let p = ScriptedProvider::new("m");
    p.push_calls(
        "",
        vec![
            ("slow_look", json!({"n": "1"})),
            ("slow_look", json!({"n": "2"})),
            ("slow_look", json!({"n": "3"})),
        ],
    );
    p.push(Box::new(|req: &ChatRequest| {
        let results: Vec<String> = req
            .messages
            .last()
            .unwrap()
            .parts
            .iter()
            .filter_map(|p| match p {
                Part::ToolResult { content, .. } => Some(content.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(results, ["looked 1", "looked 2", "looked 3"]);
        Ok(agenium_llm::mock::calls_response(
            "",
            vec![("submit_report", json!({"ticket_id": "AGN-1"}))],
        ))
    }));
    let started = std::time::Instant::now();
    let r = run(&p, &ctx, &tools, &proto, cfg()).run(&order()).await;
    assert_eq!(r.outcome, Outcome::Completed);
    assert_eq!(
        peak.load(std::sync::atomic::Ordering::SeqCst),
        3,
        "all three at once"
    );
    assert!(started.elapsed() < Duration::from_millis(400));
}

/// ADR-103: a wordless 400 counts as an overflow only for a very large request.
#[tokio::test]
async fn a_wordless_400_is_an_overflow_only_for_a_huge_request() {
    let bare = || LlmError::Http {
        status: 400,
        kind: String::new(),
        message: "{\"model\":\"deepseek-v4.1-flash\"}".into(),
        quota: false,
        retry_after: None,
    };
    let (_t, ctx, tools, proto) = harness();
    // Ordinary request: reported as it is.
    let p = ScriptedProvider::new("m");
    p.push_error(bare());
    let r = run(&p, &ctx, &tools, &proto, cfg()).run(&order()).await;
    assert!(
        matches!(r.outcome, Outcome::Failed { .. }),
        "{:?}",
        r.outcome
    );
    // A huge one: notes, then on.
    let p = ScriptedProvider::new("m");
    p.push_error(bare());
    p.push_text("Notes: nothing done yet. Next: submit.");
    p.push_calls("", vec![("submit_report", json!({"ticket_id": "AGN-1"}))]);
    let mut huge = order();
    huge.messages = vec![Message::user_text("x".repeat(500_000))];
    let r = run(&p, &ctx, &tools, &proto, cfg()).run(&huge).await;
    assert_eq!(r.outcome, Outcome::Completed);
}

/// ADR-100: the notes list the files read and changed, and a second compaction updates the first notes and keeps
/// the lists.
#[tokio::test]
async fn notes_carry_the_files_read_and_changed() {
    let (t, ctx, tools, proto) = harness();
    std::fs::write(t.path().join("spec.md"), "the spec\n").unwrap();
    let p = ScriptedProvider::new("m");
    p.push(using(100, vec![("read_file", json!({"path": "spec.md"}))]));
    p.push(using(
        9_000,
        vec![(
            "write_file",
            json!({"path": "out.rs", "content": "fn main() {}\n"}),
        )],
    ));
    p.push(Box::new(|req: &ChatRequest| {
        let ask = req.messages.last().unwrap().text();
        assert!(
            ask.contains("## Next steps") && ask.contains("update them"),
            "{ask}"
        );
        Ok(agenium_llm::mock::calls_response(
            "## Goal\nfirst notes",
            vec![],
        ))
    }));
    p.push(using(9_000, vec![("list_dir", json!({}))]));
    p.push(Box::new(|req: &ChatRequest| {
        let earlier = req.messages[1].text();
        assert!(
            earlier.contains("first notes"),
            "the earlier notes are shown: {earlier}"
        );
        Ok(agenium_llm::mock::calls_response(
            "## Goal\nsecond notes",
            vec![],
        ))
    }));
    p.push(Box::new(|req: &ChatRequest| {
        let note = req.messages[1].text();
        assert!(note.contains("second notes"), "{note}");
        assert!(
            note.contains("<read-files>\nspec.md\n</read-files>"),
            "{note}"
        );
        assert!(
            note.contains("<modified-files>\nout.rs\n</modified-files>"),
            "{note}"
        );
        Ok(agenium_llm::mock::calls_response(
            "",
            vec![("submit_report", json!({"ticket_id": "AGN-1"}))],
        ))
    }));
    let r = run(&p, &ctx, &tools, &proto, small_window())
        .run(&order())
        .await;
    assert_eq!(r.outcome, Outcome::Completed);
}

/// ADR-113 (after DeepSeek's harness): a compaction summarises the older steps and keeps the most recent ones verbatim;
/// the notes request carries only the older part, a prefix of the run's last request.
#[tokio::test]
async fn a_compaction_keeps_the_most_recent_steps() {
    let (_t, ctx, tools, proto) = harness();
    let p = ScriptedProvider::new("m");
    p.push(using(10_000, vec![("list_dir", json!({"depth": 1}))]));
    p.push(using(20_000, vec![("list_dir", json!({"depth": 2}))]));
    p.push(using(90_000, vec![("list_dir", json!({"depth": 3}))]));
    p.push(Box::new(|req: &ChatRequest| {
        assert!(req.tools_off);
        let calls = format!("{:?}", req.messages);
        assert!(
            calls.contains("\"depth\": Number(1)") && !calls.contains("\"depth\": Number(3)"),
            "{calls}"
        );
        Ok(agenium_llm::mock::calls_response(
            "Done: listed twice. Next: one more listing, then report.",
            vec![],
        ))
    }));
    p.push(Box::new(|req: &ChatRequest| {
        // The work order, the notes, then the latest step as it was.
        let texts: Vec<String> = req.messages.iter().map(|m| m.text()).collect();
        assert!(texts[1].starts_with("[Context compacted"), "{texts:?}");
        let rest = format!("{:?}", &req.messages[2..]);
        assert!(
            rest.contains("\"depth\": Number(3)") && !rest.contains("\"depth\": Number(1)"),
            "{rest}"
        );
        Ok(agenium_llm::mock::calls_response(
            "",
            vec![("submit_report", json!({"ticket_id": "AGN-1"}))],
        ))
    }));
    let r = run(&p, &ctx, &tools, &proto, roomy_window())
        .run(&order())
        .await;
    assert_eq!(r.outcome, Outcome::Completed);
}

/// ADR-113: clearing an old result changes the history the later thinking was written on, so those thinking blocks go
/// (Claude rejects or drops them otherwise); thinking from before the cleared result stays.
#[tokio::test]
async fn clearing_an_old_result_drops_the_thinking_written_after_it() {
    let (_t, ctx, tools, proto) = harness();
    let big = "x".repeat(6000);
    let thought = |tokens: u64,
                   label: &'static str,
                   call: (&'static str, serde_json::Value)|
     -> agenium_llm::mock::Responder {
        Box::new(move |_| {
            let mut r = agenium_llm::mock::calls_response("", vec![call.clone()]);
            r.usage.input = tokens;
            r.message.parts.insert(
                0,
                Part::Thinking {
                    text: label.into(),
                    signature: Some("sig".into()),
                    provider: "mock".into(),
                    redacted: false,
                    model: None,
                },
            );
            Ok(r)
        })
    };

    let p = ScriptedProvider::new("m");
    p.push(thought(
        100,
        "before",
        ("write_file", json!({"path": "big.txt", "content": big})),
    ));
    p.push(thought(
        100,
        "reading",
        ("read_file", json!({"path": "big.txt"})),
    ));
    for (d, label) in [
        (1, "after-1"),
        (2, "after-2"),
        (3, "after-3"),
        (4, "after-4"),
    ] {
        p.push(thought(100, label, ("list_dir", json!({"depth": d}))));
    }
    p.push(thought(7_000, "after-5", ("list_dir", json!({"depth": 5}))));
    p.push(Box::new(|req: &ChatRequest| {
        let thinking: Vec<String> = req
            .messages
            .iter()
            .flat_map(|m| m.parts.iter())
            .filter_map(|p| match p {
                Part::Thinking { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(thinking, vec!["before", "reading"], "{thinking:?}");
        Ok(agenium_llm::mock::calls_response(
            "",
            vec![("submit_report", json!({"ticket_id": "AGN-1"}))],
        ))
    }));
    let r = run(&p, &ctx, &tools, &proto, small_window())
        .run(&order())
        .await;
    assert_eq!(r.outcome, Outcome::Completed);
}

/// `view_image` shows the image to a model that takes images, after the tool results and under its path; a model
/// that does not is told the image was not shown, and gets no image part.
#[tokio::test]
async fn an_image_reaches_a_vision_model_after_the_results() {
    for vision in [true, false] {
        let (_t, ctx, tools, proto) = harness();
        std::fs::create_dir_all(ctx.root.join("shots")).unwrap();
        std::fs::write(ctx.root.join("shots/home.png"), PNG).unwrap();
        let p = ScriptedProvider::new("m");
        p.push_calls(
            "",
            vec![
                ("view_image", json!({"path": "shots/home.png"})),
                ("list_dir", json!({})),
            ],
        );
        p.push_calls("", vec![("submit_report", json!({"ticket_id": "AGN-1"}))]);
        let r = run(&p, &ctx, &tools, &proto, LoopConfig { vision, ..cfg() })
            .run(&order())
            .await;
        assert_eq!(r.outcome, Outcome::Completed);
        let reqs = p.requests();
        let results = &reqs[1].messages.last().unwrap().parts;
        let kinds: Vec<&str> = results
            .iter()
            .map(|p| match p {
                Part::ToolResult { .. } => "result",
                Part::Text { .. } => "text",
                Part::Image { .. } => "image",
                _ => "other",
            })
            .collect();
        if vision {
            assert_eq!(kinds, ["result", "result", "text", "image"]);
            assert!(
                matches!(&results[2], Part::Text { text } if text == "[image: shots/home.png]")
            );
            assert!(
                matches!(&results[3], Part::Image { media_type, .. } if media_type == "image/png")
            );
        } else {
            assert_eq!(kinds, ["result", "result"]);
            assert!(
                matches!(&results[0], Part::ToolResult { content, .. } if content.contains("1 image(s) not shown")),
                "{results:?}"
            );
        }
    }
}
