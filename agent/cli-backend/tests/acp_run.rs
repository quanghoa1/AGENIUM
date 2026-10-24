//! CliRun against the scripted ACP agent: the four controls of decision 3c end to end (MCP protocol tools,
//! permission decisions, step / loop limits, mandatory report).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agenium_backend::{Cancel, Effect, Observer, Outcome, Protocol, WorkOrder};
use agenium_cli_backend::{CliRun, Limits};
use agenium_llm::{Message, SystemBlock, ToolSpec};
use agenium_tools::{CommandGate, GateDecision, ToolError};
use async_trait::async_trait;
use serde_json::{json, Value};

#[derive(Default)]
struct Proto {
    calls: Mutex<Vec<(String, Value, String)>>,
}

#[async_trait]
impl Protocol for Proto {
    async fn call(
        &self,
        name: &str,
        args: Value,
        call_id: &str,
    ) -> Result<(Value, Effect), ToolError> {
        self.calls
            .lock()
            .unwrap()
            .push((name.into(), args.clone(), call_id.into()));
        match name {
            "submit_report" => Ok((json!("report accepted"), Effect::End)),
            _ => Err(ToolError::new("unknown_tool", name)),
        }
    }
}

struct Gate;

#[async_trait]
impl CommandGate for Gate {
    async fn check(&self, command: &str, _: &Path) -> GateDecision {
        if command == "echo hi" {
            GateDecision::Allow
        } else {
            GateDecision::Deny(format!("not approved: {command}"))
        }
    }
}

/// Stands for an MCP tool of the agent's own bindings.
struct Echo;

#[async_trait]
impl agenium_tools::Tool for Echo {
    fn name(&self) -> &str {
        "mcp__fake__echo"
    }
    fn description(&self) -> &str {
        "Echo text."
    }
    fn parameters(&self) -> Value {
        json!({"type": "object", "properties": {"text": {"type": "string"}}})
    }
    fn read_only(&self) -> bool {
        true
    }
    async fn call(
        &self,
        _: &agenium_tools::ToolCtx,
        args: Value,
    ) -> Result<agenium_tools::ToolOutput, ToolError> {
        let mut o = agenium_tools::ToolOutput::text(format!(
            "echo {}",
            args["text"].as_str().unwrap_or("")
        ));
        o.untrusted = true;
        Ok(o)
    }
}

#[derive(Default)]
struct Obs {
    tools: Mutex<Vec<(String, bool)>>,
    messages: Mutex<Vec<String>>,
}

impl Observer for Obs {
    fn message(&self, m: &Message) {
        self.messages.lock().unwrap().push(m.text());
    }
    fn tool(&self, name: &str, _: &Value, r: &Result<String, ToolError>) {
        self.tools.lock().unwrap().push((name.into(), r.is_ok()));
    }
}

fn order() -> WorkOrder {
    WorkOrder {
        system: vec![SystemBlock {
            text: "You are a worker.".into(),
            cache: true,
        }],
        messages: vec![Message::user_text("Ticket AGN-1: write a.txt")],
    }
}

fn spec() -> ToolSpec {
    ToolSpec {
        name: "submit_report".into(),
        description: "Report.".into(),
        parameters: json!({"type": "object"}),
    }
}

async fn run(
    scenario: &str,
    dir: &Path,
    limits: Limits,
    chat: bool,
) -> (agenium_backend::RunReport, Arc<Proto>, Arc<Obs>, String) {
    let proto = Arc::new(Proto::default());
    let obs = Arc::new(Obs::default());
    let ctx = agenium_tools::ToolCtx::new(
        dir,
        "run-1",
        Arc::new(agenium_tools::DenyAll("no commands".into())),
    );
    let r = CliRun {
        program: PathBuf::from(env!("CARGO_BIN_EXE_agenium-acp-test-agent")),
        args: vec![],
        env: [("ACP_SCENARIO".to_owned(), scenario.to_owned())].into(),
        cwd: dir.to_path_buf(),
        label: "fake:model".into(),
        mode: Some("accept-edits".into()),
        protocol: proto.as_ref(),
        tools: vec![Arc::new(Echo)],
        ctx: &ctx,
        protocol_specs: vec![spec()],
        gate: Arc::new(Gate),
        observer: obs.as_ref(),
        cancel: Cancel::default(),
        limits,
        chat,
    };
    let report = r.run(&order()).await;
    let log = std::fs::read_to_string(dir.join("acp-test.log")).unwrap_or_default();
    (report, proto, obs, log)
}

#[tokio::test(flavor = "multi_thread")]
async fn work_run_ends_through_submit_report() {
    let dir = tempfile::tempdir().unwrap();
    let (report, proto, obs, log) = run("report", dir.path(), Limits::default(), false).await;
    assert_eq!(report.outcome, Outcome::Completed, "log:\n{log}");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
        "hello"
    );
    assert!(log.contains("mode: accept-edits"), "{log}");
    assert!(log.contains("perm echo: allow-once"), "{log}");
    assert!(log.contains("perm curl: reject-once"), "{log}");
    assert!(log.contains("perm outside: reject-once"), "{log}");
    assert!(
        log.contains("tools: submit_report,mcp__fake__echo"),
        "{log}"
    );
    assert!(
        log.contains(r#"echo: "echo hi\n(Untrusted content"#),
        "agent MCP tools run through Agenium, output marked untrusted: {log}"
    );
    assert!(log.contains("report: report accepted"), "{log}");
    assert!(
        log.contains("cancel"),
        "Agenium cancels the turn after the report: {log}"
    );
    let calls = proto.calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].1, json!({"summary": "done"}));
    assert!(calls[0].2.starts_with("acp-"));
    let tools = obs.tools.lock().unwrap().clone();
    assert!(tools.contains(&("submit_report".into(), true)));
    assert!(tools.contains(&("cli:edit".into(), true)));
    assert_eq!(
        tools
            .iter()
            .filter(|t| t.0 == "cli:permission" && !t.1)
            .count(),
        2,
        "{tools:?}"
    );
    assert_eq!(report.steps, 1);
    assert_eq!(
        (
            report.usage.input,
            report.usage.cache_read,
            report.usage.output
        ),
        (60, 40, 10)
    );
    assert_eq!(report.model, "fake:model");
    assert!(report
        .messages
        .iter()
        .any(|m| m.text().contains("Work done")));
}

#[tokio::test(flavor = "multi_thread")]
async fn text_only_turns_are_nudged_then_stopped() {
    let dir = tempfile::tempdir().unwrap();
    let limits = Limits {
        nudge_limit: 2,
        ..Limits::default()
    };
    let (report, proto, obs, _) = run("chatty", dir.path(), limits, false).await;
    assert_eq!(report.outcome, Outcome::NoProgress);
    assert!(proto.calls.lock().unwrap().is_empty());
    let nudges = obs
        .messages
        .lock()
        .unwrap()
        .iter()
        .filter(|m| m.contains("submit_report"))
        .count();
    assert_eq!(nudges, 2);
    assert_eq!(report.usage.output, 30, "three prompts");
}

#[tokio::test(flavor = "multi_thread")]
async fn chat_run_completes_on_text() {
    let dir = tempfile::tempdir().unwrap();
    let (report, ..) = run("chatty", dir.path(), Limits::default(), true).await;
    assert_eq!(report.outcome, Outcome::Completed);
}

#[tokio::test(flavor = "multi_thread")]
async fn repeated_call_is_a_loop() {
    let dir = tempfile::tempdir().unwrap();
    let (report, _, _, log) = run("loop", dir.path(), Limits::default(), false).await;
    assert_eq!(
        report.outcome,
        Outcome::LoopDetected {
            tool: "cli:read".into()
        }
    );
    assert!(log.contains("cancel"));
    assert_eq!(report.steps, 3);
}

#[tokio::test(flavor = "multi_thread")]
async fn step_limit_checkpoints_with_a_note() {
    let dir = tempfile::tempdir().unwrap();
    let limits = Limits {
        max_steps: 2,
        timeout: Duration::from_secs(60),
        ..Limits::default()
    };
    let (report, ..) = run("steps", dir.path(), limits, false).await;
    assert_eq!(
        report.outcome,
        Outcome::Checkpoint {
            summary: "note: halfway".into(),
            reason: "max_steps".into()
        }
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn missing_cli_fails_cleanly() {
    let dir = tempfile::tempdir().unwrap();
    let proto = Proto::default();
    let obs = Obs::default();
    let ctx = agenium_tools::ToolCtx::new(
        dir.path(),
        "run-1",
        Arc::new(agenium_tools::DenyAll("no commands".into())),
    );
    let r = CliRun {
        program: dir.path().join("no-such-cli"),
        args: vec![],
        env: Default::default(),
        cwd: dir.path().to_path_buf(),
        label: "x".into(),
        mode: None,
        protocol: &proto,
        tools: vec![],
        ctx: &ctx,
        protocol_specs: vec![spec()],
        gate: Arc::new(Gate),
        observer: &obs,
        cancel: Cancel::default(),
        limits: Limits::default(),
        chat: false,
    };
    assert!(matches!(
        r.run(&order()).await.outcome,
        Outcome::Failed { .. }
    ));
}
