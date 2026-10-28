//! The context window: clearing old results, compaction, notes, overflow.

use super::*;

/// ADR-100: past 60 % of the window, long tool results older than the last four are cleared (no model call).
#[tokio::test]
async fn old_tool_results_are_cleared_when_the_context_fills() {
    let (_t, ctx, tools, proto) = harness();
    let big = "x".repeat(6000);
    let p = ScriptedProvider::new("m");
    p.push(using(
        100,
        vec![("write_file", json!({"path": "big.txt", "content": big}))],
    ));
    p.push(using(100, vec![("read_file", json!({"path": "big.txt"}))]));
    for d in 1..=4 {
        p.push(using(100, vec![("list_dir", json!({"depth": d}))]));
    }
    // 70 % in use: the next request has the old read cleared, the last four results whole.
    p.push(using(7_000, vec![("list_dir", json!({"depth": 5}))]));
    p.push(Box::new(|req: &ChatRequest| {
        let all: String = req
            .messages
            .iter()
            .flat_map(|m| m.parts.iter())
            .filter_map(|p| match p {
                Part::ToolResult { name, content, .. } => Some(format!("{name}: {content}\n")),
                _ => None,
            })
            .collect();
        assert!(
            all.contains("read_file: [cleared to save context: "),
            "{all}"
        );
        assert!(!all.contains(&"x".repeat(1000)), "the big result is gone");
        assert_eq!(all.matches("list_dir: ").count(), 5);
        assert!(!all.contains("list_dir: [cleared"), "recent results stay");
        Ok(agenium_llm::mock::calls_response(
            "",
            vec![("submit_report", json!({"ticket_id": "AGN-1"}))],
        ))
    }));
    let r = run(&p, &ctx, &tools, &proto, small_window())
        .run(&order())
        .await;
    assert_eq!(r.outcome, Outcome::Completed);
    assert_eq!(p.remaining(), 0);
}

/// ADR-100: past 85 % the run's conversation is replaced by the model's notes and the run goes on.
#[tokio::test]
async fn a_full_context_is_summarised_and_the_run_goes_on() {
    let (_t, ctx, tools, proto) = harness();
    let p = ScriptedProvider::new("m");
    p.push(using(90_000, vec![("list_dir", json!({}))]));
    p.push_text("Task: AGN-1. Done: listed the files. Next: submit the report.");
    p.push(Box::new(|req: &ChatRequest| {
        assert!(!req.tools.is_empty(), "work goes on with tools");
        assert_eq!(req.messages.len(), 2, "{:?}", req.messages);
        assert!(req.messages[0].text().contains("# Ticket AGN-1"));
        let note = req.messages[1].text();
        assert!(note.starts_with("[Context compacted"), "{note}");
        assert!(note.contains("Next: submit the report."), "{note}");
        Ok(agenium_llm::mock::calls_response(
            "",
            vec![("submit_report", json!({"ticket_id": "AGN-1"}))],
        ))
    }));
    let r = run(&p, &ctx, &tools, &proto, roomy_window())
        .run(&order())
        .await;
    assert_eq!(r.outcome, Outcome::Completed);
    // ADR-113: the notes request is the run's exact prefix plus the instruction, tools listed but not callable, so
    // the provider's prompt cache serves it.
    let reqs = p.requests();
    let (step, notes) = (&reqs[0], &reqs[1]);
    assert!(!notes.tools.is_empty() && notes.tools_off);
    assert_eq!(notes.tools, step.tools);
    assert_eq!(notes.system, step.system);
    assert_eq!(&notes.messages[..step.messages.len()], &step.messages[..]);
    assert!(notes
        .messages
        .last()
        .unwrap()
        .text()
        .starts_with("The context is nearly full"));
    // What the run keeps (and a later run resumes from) starts with the notes.
    assert!(r.messages[0].text().starts_with("[Context compacted"));
}

/// ADR-113: when the whole prefix does not fit the notes request, the smaller request of ADR-103 is sent instead.
#[tokio::test]
async fn notes_fall_back_to_a_smaller_request_when_the_prefix_does_not_fit() {
    let (_t, ctx, tools, proto) = harness();
    let p = ScriptedProvider::new("m");
    p.push(using(90_000, vec![("list_dir", json!({}))]));
    p.push_error(too_long());
    p.push_text("Task: AGN-1. Next: submit the report.");
    p.push_calls("", vec![("submit_report", json!({"ticket_id": "AGN-1"}))]);
    let r = run(&p, &ctx, &tools, &proto, roomy_window())
        .run(&order())
        .await;
    assert_eq!(r.outcome, Outcome::Completed);
    let reqs = p.requests();
    assert!(reqs[1].tools_off && reqs[2].tools_off);
    assert!(reqs[2]
        .messages
        .last()
        .unwrap()
        .text()
        .starts_with("The context is nearly full"));
    assert!(r.messages[0].text().starts_with("[Context compacted"));
}

/// ADR-100: after `max_compactions`, a full context checkpoints the run as before.
#[tokio::test]
async fn after_two_compactions_the_run_checkpoints() {
    let (_t, ctx, tools, proto) = harness();
    let p = ScriptedProvider::new("m");
    for d in 1..=3 {
        p.push(using(9_000, vec![("list_dir", json!({"depth": d}))]));
        if d < 3 {
            p.push_text(&format!("Notes {d}."));
        }
    }
    p.push_text("Progress: listed three times.");
    let r = run(&p, &ctx, &tools, &proto, small_window())
        .run(&order())
        .await;
    assert_eq!(
        r.outcome,
        Outcome::Checkpoint {
            summary: "Progress: listed three times.".into(),
            reason: "context".into()
        }
    );
    assert_eq!(p.remaining(), 0);
    // Off: the first full context checkpoints.
    let p = ScriptedProvider::new("m");
    p.push(using(9_000, vec![("list_dir", json!({}))]));
    p.push_text("Progress.");
    let r = run(
        &p,
        &ctx,
        &tools,
        &proto,
        LoopConfig {
            max_compactions: 0,
            ..small_window()
        },
    )
    .run(&order())
    .await;
    assert!(matches!(r.outcome, Outcome::Checkpoint { .. }));
}

/// ADR-101: tool calls in an answer cut by the output limit are not run; the model is told to send them again.
#[tokio::test]
async fn calls_cut_by_the_output_limit_are_not_run() {
    let (t, ctx, tools, proto) = harness();
    let p = ScriptedProvider::new("m");
    p.push(Box::new(|_| {
        let mut r = agenium_llm::mock::calls_response(
            "",
            vec![("write_file", json!({"path": "half.txt", "content": "par"}))],
        );
        r.stop = StopReason::MaxTokens;
        Ok(r)
    }));
    p.push(Box::new(|req: &ChatRequest| {
        let last = req.messages.last().unwrap();
        assert!(
            matches!(&last.parts[0], Part::ToolResult { content, is_error: true, .. } if content.contains("truncated")),
            "{last:?}"
        );
        Ok(agenium_llm::mock::calls_response(
            "",
            vec![("submit_report", json!({"ticket_id": "AGN-1"}))],
        ))
    }));
    let r = run(&p, &ctx, &tools, &proto, cfg()).run(&order()).await;
    assert_eq!(r.outcome, Outcome::Completed);
    assert!(
        !t.path().join("half.txt").exists(),
        "the cut call did not run"
    );
}

/// ADR-101: thinking is stamped with the model that wrote it and replayed to it.
#[tokio::test]
async fn thinking_is_kept_for_the_model_that_wrote_it() {
    let (_t, ctx, tools, proto) = harness();
    let p = ScriptedProvider::new("m");
    p.push(Box::new(|_| {
        let mut r = agenium_llm::mock::calls_response("", vec![("list_dir", json!({}))]);
        r.message.parts.insert(
            0,
            Part::Thinking {
                text: "look first".into(),
                signature: Some("sig".into()),
                provider: "anthropic".into(),
                redacted: false,
                model: None,
            },
        );
        Ok(r)
    }));
    p.push(Box::new(|req: &ChatRequest| {
        let kept = req
            .messages
            .iter()
            .flat_map(|m| m.parts.iter())
            .any(|x| matches!(x, Part::Thinking { model: Some(m), .. } if m == "scripted"));
        assert!(kept, "{:?}", req.messages);
        Ok(agenium_llm::mock::calls_response(
            "",
            vec![("submit_report", json!({"ticket_id": "AGN-1"}))],
        ))
    }));
    let r = run(&p, &ctx, &tools, &proto, cfg()).run(&order()).await;
    assert_eq!(r.outcome, Outcome::Completed);
}

/// ADR-103: a provider's context overflow frees old tool results and sends again.
#[tokio::test]
async fn an_overflow_clears_old_results_and_sends_again() {
    let (_t, ctx, tools, proto) = harness();
    let p = ScriptedProvider::new("m");
    p.push_calls(
        "",
        vec![(
            "write_file",
            json!({"path": "big.txt", "content": "y".repeat(3000)}),
        )],
    );
    p.push_calls("", vec![("read_file", json!({"path": "big.txt"}))]);
    p.push_calls("", vec![("list_dir", json!({"depth": 1}))]);
    p.push_calls("", vec![("list_dir", json!({"depth": 2}))]);
    p.push_error(too_long());
    p.push(Box::new(|req: &ChatRequest| {
        // Only results are cleared (the write call's own arguments stay).
        let results: String = req
            .messages
            .iter()
            .flat_map(|m| m.parts.iter())
            .filter_map(|p| match p {
                Part::ToolResult { content, .. } => Some(content.as_str()),
                _ => None,
            })
            .collect();
        assert!(results.contains("[cleared to save context"), "{results}");
        assert!(!results.contains(&"y".repeat(500)));
        Ok(agenium_llm::mock::calls_response(
            "",
            vec![("submit_report", json!({"ticket_id": "AGN-1"}))],
        ))
    }));
    let r = run(&p, &ctx, &tools, &proto, cfg()).run(&order()).await;
    assert_eq!(r.outcome, Outcome::Completed);
}

/// ADR-103: with nothing to clear, the overflow is answered with notes; a second overflow in a row is reported.
#[tokio::test]
async fn an_overflow_with_nothing_to_clear_compacts_then_gives_up() {
    let (_t, ctx, tools, proto) = harness();
    let p = ScriptedProvider::new("m");
    p.push_calls("", vec![("list_dir", json!({}))]);
    p.push_error(too_long());
    p.push_text("Notes: listed. Next: submit.");
    p.push(Box::new(|req: &ChatRequest| {
        assert!(req.messages[1].text().starts_with("[Context compacted"));
        Ok(agenium_llm::mock::calls_response(
            "",
            vec![("submit_report", json!({"ticket_id": "AGN-1"}))],
        ))
    }));
    let r = run(&p, &ctx, &tools, &proto, cfg()).run(&order()).await;
    assert_eq!(r.outcome, Outcome::Completed);

    let p = ScriptedProvider::new("m");
    p.push_error(too_long());
    p.push_text("Notes.");
    p.push_error(too_long());
    let r = run(&p, &ctx, &tools, &proto, cfg()).run(&order()).await;
    assert!(
        matches!(&r.outcome, Outcome::Failed { message } if message.contains("prompt is too long")),
        "{:?}",
        r.outcome
    );
}

/// ADR-103: notes cut by the output limit are not used; the run checkpoints.
#[tokio::test]
async fn notes_cut_by_the_output_limit_are_not_used() {
    let (_t, ctx, tools, proto) = harness();
    let p = ScriptedProvider::new("m");
    p.push(using(9_000, vec![("list_dir", json!({}))]));
    p.push(Box::new(|_| {
        let mut r = agenium_llm::mock::calls_response("Notes that stop mid", vec![]);
        r.stop = StopReason::MaxTokens;
        Ok(r)
    }));
    p.push_text("Progress: listed.");
    let r = run(&p, &ctx, &tools, &proto, small_window())
        .run(&order())
        .await;
    assert!(
        matches!(&r.outcome, Outcome::Checkpoint { reason, .. } if reason == "context"),
        "{:?}",
        r.outcome
    );
}

/// ADR-103: a large tool result counts before the next request, so notes are written before it is sent.
#[tokio::test]
async fn a_large_new_result_counts_before_the_next_request() {
    let (t, ctx, tools, proto) = harness();
    std::fs::write(t.path().join("huge.txt"), "z".repeat(40_000)).unwrap();
    let p = ScriptedProvider::new("m");
    p.push(using(100, vec![("read_file", json!({"path": "huge.txt"}))]));
    p.push_text("Notes: read huge.txt. Next: submit.");
    p.push_calls("", vec![("submit_report", json!({"ticket_id": "AGN-1"}))]);
    let r = run(
        &p,
        &ctx,
        &tools,
        &proto,
        LoopConfig {
            context_window: 4_000,
            ..cfg()
        },
    )
    .run(&order())
    .await;
    assert_eq!(r.outcome, Outcome::Completed);
    assert!(
        p.requests()[1].tools_off,
        "notes came before the big request"
    );
    let notes_ask = format!("{:?}", p.requests()[1].messages);
    assert!(
        !notes_ask.contains(&"z".repeat(2500)),
        "results are cut for the notes request"
    );
}
