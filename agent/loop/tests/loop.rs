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
