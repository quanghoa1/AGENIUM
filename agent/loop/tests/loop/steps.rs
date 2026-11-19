//! The loop's steps: submit, repeated and invalid calls, nudges, checkpoints, errors, cancel.

use super::*;

#[tokio::test]
async fn completes_on_submit_and_carries_call_ids() {
    let (_t, ctx, tools, proto) = harness();
    let p = ScriptedProvider::new("m");
    p.push_calls(
        "I'll write it.",
        vec![(
            "write_file",
            json!({"path": "hello.txt", "content": "hi\n"}),
        )],
    );
    p.push_calls("", vec![("submit_report", json!({"ticket_id": "AGN-1"}))]);
    let r = run(&p, &ctx, &tools, &proto, cfg()).run(&order()).await;
    assert_eq!(r.outcome, Outcome::Completed);
    assert_eq!(r.steps, 2);
    assert_eq!(
        std::fs::read_to_string(ctx.root.join("hello.txt")).unwrap(),
        "hi\n"
    );
    assert_eq!(
        proto.calls.lock().unwrap()[0],
        (
            "submit_report".to_string(),
            "call_submit_report_0".to_string()
        )
    );
    // Requests are append-only: each request extends the previous one (prompt cache).
    let reqs = p.requests();
    assert_eq!(
        reqs[1].messages[..reqs[0].messages.len()],
        reqs[0].messages[..]
    );
    assert!(reqs[0]
        .tools
        .iter()
        .map(|t| t.name.as_str())
        .collect::<Vec<_>>()
        .windows(2)
        .all(|w| w[0] <= w[1]));
    assert_eq!(
        r.messages.len(),
        4,
        "assistant, results, assistant, results"
    );
    assert_eq!(r.usage.input, 200);
}

#[tokio::test]
async fn identical_call_is_refused_then_stops_but_different_args_do_not() {
    let (_t, ctx, tools, proto) = harness();
    std::fs::write(ctx.root.join("a.txt"), "a").unwrap();
    std::fs::write(ctx.root.join("b.txt"), "b").unwrap();
    let p = ScriptedProvider::new("m");
    for f in ["a.txt", "b.txt", "a.txt", "b.txt"] {
        p.push_calls("", vec![("read_file", json!({"path": f}))]);
    }
    for _ in 0..4 {
        p.push_calls("", vec![("read_file", json!({"path": "a.txt"}))]);
    }
    let r = run(&p, &ctx, &tools, &proto, cfg()).run(&order()).await;
    // The third identical call is refused with a warning (the run goes on); the fourth stops it.
    assert_eq!(
        r.outcome,
        Outcome::LoopDetected {
            tool: "read_file".into()
        }
    );
    assert_eq!(r.steps, 8);
    let warned = r
        .messages
        .iter()
        .any(|m| m.text().contains("was not run again"))
        || serde_json::to_string(&r.messages)
            .unwrap()
            .contains("was not run again");
    assert!(warned, "the agent was told to change course first");
}

#[tokio::test]
async fn the_same_refusal_three_times_in_a_row_stops_the_run() {
    let (_t, ctx, tools, proto) = harness();
    let p = ScriptedProvider::new("m");
    for x in 1..=3 {
        p.push_calls("", vec![("no_such_tool", json!({ "x": x }))]);
    }
    let r = run(&p, &ctx, &tools, &proto, cfg()).run(&order()).await;
    assert!(
        matches!(&r.outcome, Outcome::TooManyErrors { .. }),
        "{:?}",
        r.outcome
    );
    assert_eq!(r.steps, 3);
}

#[tokio::test]
async fn changing_refusals_get_twice_the_room() {
    let (_t, ctx, tools, proto) = harness();
    let p = ScriptedProvider::new("m");
    p.push_calls("", vec![("no_such_tool", json!({}))]);
    p.push_calls("", vec![("submit_report", json!({}))]);
    // A valid call in between resets the counter.
    p.push_calls("", vec![("list_dir", json!({}))]);
    p.push_calls("", vec![("submit_report", json!({}))]);
    p.push_calls("", vec![("no_such_tool", json!({"x": 1}))]);
    p.push(Box::new(|_| {
        Ok(mock::calls_response(
            "",
            vec![(
                "read_file",
                json!({agenium_llm::INVALID_JSON_KEY: "{\"path\": \"a"}),
            )],
        ))
    }));
    // Three refusals in a row, each a different one: the agent may be fixing its call, so it goes on (up to six).
    for name in ["missing_one", "missing_two"] {
        p.push_calls("", vec![(name, json!({}))]);
    }
    p.push_calls("", vec![("missing_three", json!({}))]);
    let r = run(&p, &ctx, &tools, &proto, cfg()).run(&order()).await;
    assert!(
        matches!(&r.outcome, Outcome::TooManyErrors { last } if last.contains("missing_three")),
        "{:?}",
        r.outcome
    );
    assert_eq!(r.steps, 9);
}

#[tokio::test]
async fn text_without_tools_is_nudged_then_stops() {
    let (_t, ctx, tools, proto) = harness();
    let p = ScriptedProvider::new("m");
    p.push_text("I think it is done.");
    p.push_calls("", vec![("ask_question", json!({}))]);
    let r = run(&p, &ctx, &tools, &proto, cfg()).run(&order()).await;
    assert_eq!(r.outcome, Outcome::Parked);
    let second = &p.requests()[1];
    assert!(second
        .messages
        .last()
        .unwrap()
        .text()
        .starts_with("Text alone is not an action"));

    let p = ScriptedProvider::new("m");
    for _ in 0..3 {
        p.push_text("done");
    }
    let r = run(&p, &ctx, &tools, &proto, cfg()).run(&order()).await;
    assert_eq!(r.outcome, Outcome::NoProgress);
}

#[tokio::test]
async fn max_steps_takes_a_checkpoint_without_tools() {
    let (_t, ctx, tools, proto) = harness();
    let p = ScriptedProvider::new("m");
    p.push_calls("", vec![("list_dir", json!({}))]);
    p.push_calls("", vec![("list_dir", json!({"depth": 2}))]);
    p.push_text("Done: listed files. Next: write the code.");
    let r = run(
        &p,
        &ctx,
        &tools,
        &proto,
        LoopConfig {
            max_steps: 2,
            ..cfg()
        },
    )
    .run(&order())
    .await;
    assert_eq!(
        r.outcome,
        Outcome::Checkpoint {
            summary: "Done: listed files. Next: write the code.".into(),
            reason: "max_steps".into()
        }
    );
    assert!(
        p.requests()[2].tools_off,
        "the checkpoint request lists the tools but they cannot be called"
    );
}

#[tokio::test]
async fn provider_errors_map_to_agent_states() {
    let (_t, ctx, tools, proto) = harness();
    let p = ScriptedProvider::new("m");
    p.push_error(LlmError::Http {
        status: 402,
        kind: "billing_error".into(),
        message: "no credit".into(),
        quota: false,
        retry_after: None,
    });
    assert!(matches!(
        run(&p, &ctx, &tools, &proto, cfg())
            .run(&order())
            .await
            .outcome,
        Outcome::QuotaExhausted { .. }
    ));
    let p = ScriptedProvider::new("m");
    p.push_error(LlmError::Network("down".into()));
    p.push_error(LlmError::Network("down".into()));
    assert!(matches!(
        run(&p, &ctx, &tools, &proto, cfg())
            .run(&order())
            .await
            .outcome,
        Outcome::Offline { .. }
    ));
    let p = ScriptedProvider::new("m");
    p.push_error(LlmError::Http {
        status: 400,
        kind: "invalid_request_error".into(),
        message: "bad".into(),
        quota: false,
        retry_after: None,
    });
    assert!(matches!(
        run(&p, &ctx, &tools, &proto, cfg())
            .run(&order())
            .await
            .outcome,
        Outcome::Failed { .. }
    ));
    // Transient errors are retried transparently.
    let p = ScriptedProvider::new("m");
    p.push_error(LlmError::Http {
        status: 529,
        kind: "overloaded_error".into(),
        message: "".into(),
        quota: false,
        retry_after: None,
    });
    p.push_calls("", vec![("ask_question", json!({}))]);
    assert_eq!(
        run(&p, &ctx, &tools, &proto, cfg())
            .run(&order())
            .await
            .outcome,
        Outcome::Parked
    );
}

/// A stop during an outage's retry waits ends the run at once, with no further attempt (audit M-11).
#[tokio::test]
async fn a_stop_ends_the_retry_waits() {
    let (_t, ctx, tools, proto) = harness();
    let p = ScriptedProvider::new("m");
    for _ in 0..3 {
        p.push_error(LlmError::Http {
            status: 529,
            kind: "overloaded_error".into(),
            message: "".into(),
            quota: false,
            retry_after: None,
        });
    }
    let mut c = cfg();
    c.retry = RetryPolicy {
        base: Duration::from_secs(60),
        cap: Duration::from_secs(60),
        max_attempts: 3,
        max_total: Duration::from_secs(300),
    };
    let r = run(&p, &ctx, &tools, &proto, c);
    let stop = r.cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        stop.cancel();
    });
    let started = std::time::Instant::now();
    assert_eq!(r.run(&order()).await.outcome, Outcome::Cancelled);
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(p.remaining(), 2, "no attempt after the stop");
}

#[tokio::test]
async fn cancel_and_chat_mode() {
    let (_t, ctx, tools, proto) = harness();
    let p = ScriptedProvider::new("m");
    let r = run(&p, &ctx, &tools, &proto, cfg());
    r.cancel.cancel();
    assert_eq!(r.run(&order()).await.outcome, Outcome::Cancelled);
    let p = ScriptedProvider::new("m");
    p.push_text("Here is my answer.");
    let mut r = run(&p, &ctx, &tools, &proto, cfg());
    r.chat = true;
    let rep = r.run(&order()).await;
    assert_eq!(rep.outcome, Outcome::Completed);
    assert_eq!(rep.messages[0].text(), "Here is my answer.");
}

/// A declined answer (a safety classifier, mid-answer) ends the run with the vendor's reason and is not kept in the
/// conversation: a resumed run starts before it.
#[tokio::test]
async fn a_refused_answer_is_dropped_and_its_reason_kept() {
    let (_t, ctx, tools, proto) = harness();
    let p = ScriptedProvider::new("m");
    p.push_calls("", vec![("read_file", json!({"path": "missing.txt"}))]);
    p.push(Box::new(|_| {
        let mut r = agenium_llm::mock::calls_response("Here is the explo", vec![]);
        r.stop = StopReason::Refusal;
        r.refusal = Some(Refusal {
            category: Some("cyber".into()),
            explanation: Some("declined: could enable cyber harm".into()),
        });
        Ok(r)
    }));
    let r = run(&p, &ctx, &tools, &proto, cfg()).run(&order()).await;
    assert_eq!(
        r.outcome,
        Outcome::Refused {
            why: "cyber: declined: could enable cyber harm".into()
        }
    );
    assert_eq!(r.outcome.label(), "Stopped:refusal");
    assert!(
        !r.messages
            .iter()
            .any(|m| m.text().contains("Here is the explo")),
        "the declined answer is not kept"
    );
    assert_eq!(
        r.messages.last().unwrap().role,
        Role::User,
        "the conversation ends with the tool results"
    );
}

/// A tenth of the steps (at least 3) before `max_steps` the agent is told what is left, once, so it wraps up.
#[tokio::test]
async fn the_agent_is_told_when_its_steps_run_low() {
    let (_t, ctx, tools, proto) = harness();
    let p = ScriptedProvider::new("m");
    for d in 1..=7 {
        p.push_calls("", vec![("list_dir", json!({"depth": d}))]);
    }
    p.push_calls("", vec![("submit_report", json!({"ticket_id": "AGN-1"}))]);
    let r = run(
        &p,
        &ctx,
        &tools,
        &proto,
        LoopConfig {
            max_steps: 10,
            ..cfg()
        },
    )
    .run(&order())
    .await;
    assert_eq!(r.outcome, Outcome::Completed);
    let reqs = p.requests();
    let told = |i: usize| {
        reqs[i]
            .messages
            .iter()
            .filter(|m| {
                m.text()
                    .starts_with("3 model steps are left in this run (of 10)")
            })
            .count()
    };
    assert_eq!(told(6), 0, "not before");
    assert_eq!(told(7), 1, "before the 8th request, with 3 steps left");
}
