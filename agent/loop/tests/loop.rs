use std::sync::{Arc, Mutex};
use std::time::Duration;

use agenium_backend::*;
use agenium_llm::mock::ScriptedProvider;
use agenium_llm::*;
use agenium_tools::{AllowAll, ToolCtx, ToolError};
use async_trait::async_trait;
use serde_json::json;

struct FakeProtocol {
    calls: Mutex<Vec<(String, String)>>,
}

#[async_trait]
impl Protocol for FakeProtocol {
    async fn call(
        &self,
        name: &str,
        args: serde_json::Value,
        call_id: &str,
    ) -> Result<(serde_json::Value, Effect), ToolError> {
        self.calls
            .lock()
            .unwrap()
            .push((name.into(), call_id.into()));
        match name {
            "submit_report" if args.get("ticket_id").is_some() => {
                Ok((json!({"status": "Resolved"}), Effect::End))
            }
            "submit_report" => Err(ToolError::new(
                "invalid_arguments",
                "missing field ticket_id",
            )),
            "ask_question" => Ok((json!({"blocking": true}), Effect::Park)),
            _ => Err(ToolError::new("tool_not_allowed", "no")),
        }
    }
}

fn harness() -> (
    tempfile::TempDir,
    ToolCtx,
    Vec<Arc<dyn agenium_tools::Tool>>,
    FakeProtocol,
) {
    let tmp = tempfile::tempdir().unwrap();
    let ctx = ToolCtx::new(tmp.path(), "r1", Arc::new(AllowAll));
    (
        tmp,
        ctx,
        agenium_tools::work_tools(),
        FakeProtocol {
            calls: Mutex::new(vec![]),
        },
    )
}

fn cfg() -> LoopConfig {
    LoopConfig {
        model: "scripted".into(),
        retry: RetryPolicy {
            base: Duration::from_millis(1),
            cap: Duration::from_millis(1),
            max_attempts: 2,
            max_total: Duration::from_secs(1),
        },
        ..Default::default()
    }
}

fn order() -> WorkOrder {
    WorkOrder {
        system: vec![SystemBlock {
            text: prompt::system_rules(false),
            cache: true,
        }],
        messages: vec![Message::user_text("# Ticket AGN-1 ...")],
    }
}

fn run<'a>(
    p: &'a ScriptedProvider,
    ctx: &'a ToolCtx,
    tools: &'a [Arc<dyn agenium_tools::Tool>],
    proto: &'a FakeProtocol,
    cfg: LoopConfig,
) -> Run<'a> {
    let mut specs = work_specs(tools);
    specs.push(ToolSpec {
        name: "submit_report".into(),
        description: "x".into(),
        parameters: json!({"type": "object"}),
    });
    specs.push(ToolSpec {
        name: "ask_question".into(),
        description: "x".into(),
        parameters: json!({"type": "object"}),
    });
    Run {
        provider: p,
        tools,
        ctx,
        protocol: proto,
        specs: sorted_specs(specs),
        protocol_names: vec!["submit_report".into(), "ask_question".into()],
        cfg,
        observer: &NoopObserver,
        cancel: Cancel::default(),
        chat: false,
        nudge: None,
        steer: None,
    }
}

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
async fn identical_call_three_times_stops_but_different_args_do_not() {
    let (_t, ctx, tools, proto) = harness();
    std::fs::write(ctx.root.join("a.txt"), "a").unwrap();
    std::fs::write(ctx.root.join("b.txt"), "b").unwrap();
    let p = ScriptedProvider::new("m");
    for f in ["a.txt", "b.txt", "a.txt", "b.txt"] {
        p.push_calls("", vec![("read_file", json!({"path": f}))]);
    }
    for _ in 0..3 {
        p.push_calls("", vec![("read_file", json!({"path": "a.txt"}))]);
    }
    let r = run(&p, &ctx, &tools, &proto, cfg()).run(&order()).await;
    assert_eq!(
        r.outcome,
        Outcome::LoopDetected {
            tool: "read_file".into()
        }
    );
    assert_eq!(r.steps, 7);
}

#[tokio::test]
async fn three_invalid_calls_in_a_row_stop_the_run() {
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
    let r = run(&p, &ctx, &tools, &proto, cfg()).run(&order()).await;
    assert!(
        matches!(&r.outcome, Outcome::TooManyErrors { last } if last.contains("invalid_json")),
        "{:?}",
        r.outcome
    );
    assert_eq!(r.steps, 6);
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
        p.requests()[2].tools.is_empty(),
        "the checkpoint request offers no tools"
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

/// A scripted answer that reports `tokens` of context in use.
fn using(
    tokens: u64,
    calls: Vec<(&'static str, serde_json::Value)>,
) -> agenium_llm::mock::Responder {
    Box::new(move |_| {
        let mut r = agenium_llm::mock::calls_response("", calls.clone());
        r.usage.input = tokens;
        Ok(r)
    })
}

fn small_window() -> LoopConfig {
    LoopConfig {
        context_window: 10_000,
        ..cfg()
    }
}

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
    p.push(using(9_000, vec![("list_dir", json!({}))]));
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
    let r = run(&p, &ctx, &tools, &proto, small_window())
        .run(&order())
        .await;
    assert_eq!(r.outcome, Outcome::Completed);
    assert!(
        p.requests()[1].tools.is_empty(),
        "the notes are asked for without tools"
    );
    // What the run keeps (and a later run resumes from) starts with the notes.
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

fn too_long() -> LlmError {
    LlmError::Http {
        status: 400,
        kind: "invalid_request_error".into(),
        message: "prompt is too long: 210000 tokens > 200000 maximum".into(),
        quota: false,
        retry_after: None,
    }
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
        p.requests()[1].tools.is_empty(),
        "notes came before the big request"
    );
    let notes_ask = format!("{:?}", p.requests()[1].messages);
    assert!(
        !notes_ask.contains(&"z".repeat(2500)),
        "results are cut for the notes request"
    );
}

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

/// A read-only tool that takes a while (and counts how many run at once).
struct Slow(
    Arc<std::sync::atomic::AtomicUsize>,
    Arc<std::sync::atomic::AtomicUsize>,
);

#[async_trait]
impl agenium_tools::Tool for Slow {
    fn name(&self) -> &str {
        "slow_look"
    }
    fn description(&self) -> &str {
        "x"
    }
    fn parameters(&self) -> serde_json::Value {
        json!({"type": "object", "properties": {"n": {"type": "integer"}}})
    }
    fn read_only(&self) -> bool {
        true
    }
    async fn call(
        &self,
        _ctx: &ToolCtx,
        args: serde_json::Value,
    ) -> Result<agenium_tools::ToolOutput, ToolError> {
        use std::sync::atomic::Ordering::SeqCst;
        let now = self.0.fetch_add(1, SeqCst) + 1;
        self.1.fetch_max(now, SeqCst);
        tokio::time::sleep(Duration::from_millis(150)).await;
        self.0.fetch_sub(1, SeqCst);
        // The argument arrived as a number although the model sent a string (ADR-105).
        assert!(args["n"].is_i64(), "{args}");
        Ok(agenium_tools::ToolOutput::text(format!(
            "looked {}",
            args["n"]
        )))
    }
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
