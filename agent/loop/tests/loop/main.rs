use std::sync::{Arc, Mutex};
use std::time::Duration;

use agenium_backend::*;
use agenium_llm::mock::ScriptedProvider;
use agenium_llm::*;
use agenium_tools::{AllowAll, ToolCtx, ToolError};
use async_trait::async_trait;
use serde_json::json;

mod context;
mod messages;
mod steps;

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

/// A window where the whole prefix plus room for notes still fits after crossing 85 % (ADR-113).
fn roomy_window() -> LoopConfig {
    LoopConfig {
        context_window: 100_000,
        ..cfg()
    }
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

/// A tiny PNG: the signature and an IHDR chunk are all `view_image` checks.
const PNG: &[u8] = &[
    0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13, b'I', b'H', b'D', b'R', 0, 0, 0,
    1, 0, 0, 0, 1, 8, 6, 0, 0, 0,
];
