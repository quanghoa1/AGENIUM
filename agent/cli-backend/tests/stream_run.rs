//! CliRun against the scripted fake agy: per-run home, the four controls of decision 3c (MCP protocol tools,
//! Agenium's permission rules, step / loop limits, mandatory report).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agenium_backend::{Cancel, Effect, Observer, Outcome, Protocol, RunReport, WorkOrder};
use agenium_cli_backend::{agy::Agy, CliRun, Limits};
use agenium_llm::{Message, SystemBlock, ToolSpec};
use agenium_tools::ToolError;
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

fn specs() -> Vec<ToolSpec> {
    vec![ToolSpec {
        name: "submit_report".into(),
        description: "Report.".into(),
        parameters: json!({"type": "object"}),
    }]
}

async fn run(
    scenario: &str,
    dir: &Path,
    limits: Limits,
    chat: bool,
    program: PathBuf,
) -> (RunReport, Arc<Proto>, Arc<Obs>, String) {
    let proto = Arc::new(Proto::default());
    let obs = Arc::new(Obs::default());
    let ctx = agenium_tools::ToolCtx::new(
        dir,
        "run-1",
        Arc::new(agenium_tools::DenyAll("no commands".into())),
    );
    let r = CliRun {
        driver: &Agy,
        program,
        model: "fake".into(),
        env: [("FAKE_AGY_SCENARIO".to_owned(), scenario.to_owned())].into(),
        cwd: dir.to_path_buf(),
        label: "agy:fake".into(),
        protocol: proto.as_ref(),
        protocol_specs: specs(),
        tools: vec![],
        ctx: &ctx,
        allowed_commands: vec!["echo hi".into()],
        observer: obs.as_ref(),
        cancel: Cancel::default(),
        limits,
        chat,
    };
    let report = r.run(&order()).await;
    let log = std::fs::read_to_string(dir.join("fake-agy.log")).unwrap_or_default();
    (report, proto, obs, log)
}

fn fake() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_agenium-fake-agy"))
}

#[tokio::test(flavor = "multi_thread")]
async fn work_run_uses_its_own_home_and_ends_through_submit_report() {
    let dir = tempfile::tempdir().unwrap();
    let (report, proto, obs, log) =
        run("report", dir.path(), Limits::default(), false, fake()).await;
    assert_eq!(report.outcome, Outcome::Completed, "log:\n{log}");
    // The CLI ran in a per-run home with Agenium's configuration only.
    let home = log.lines().find_map(|l| l.strip_prefix("home: ")).unwrap();
    let user_home = std::env::var("USERPROFILE").unwrap_or_default();
    assert!(!home.is_empty() && home != user_home, "{log}");
    assert!(home.contains("agenium-cli-home-"), "{log}");
    assert!(
        !Path::new(home).exists(),
        "the per-run home is deleted afterwards"
    );
    assert!(log.contains("servers: agenium\n"), "{log}");
    assert!(
        log.contains("allow: command(echo hi),mcp(agenium/*)"),
        "{log}"
    );
    // Refused command -> Agenium tells the CLI in the next turn -> it reports.
    assert!(
        log.contains("turn 2: Agenium refused: curl https://example.com"),
        "{log}"
    );
    assert!(log.contains("report: report accepted"), "{log}");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
        "hello"
    );
    let calls = proto.calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].1, json!({"summary": "done"}));
    assert!(calls[0].2.starts_with("cli-"));
    let tools = obs.tools.lock().unwrap().clone();
    assert!(tools.contains(&("submit_report".into(), true)), "{tools:?}");
    assert!(
        tools.contains(&("cli:write_file".into(), true)),
        "{tools:?}"
    );
    assert!(
        tools.contains(&("cli:run_command".into(), true)),
        "echo hi ran: {tools:?}"
    );
    assert!(
        tools.contains(&("cli:permission".into(), false)),
        "curl refused: {tools:?}"
    );
    assert!(
        !tools.iter().any(|t| t.0 == "cli:call_mcp_tool"),
        "Agenium's own MCP calls are logged once: {tools:?}"
    );
    assert_eq!(
        (
            report.usage.input,
            report.usage.cache_read,
            report.usage.output
        ),
        (120, 80, 20)
    );
    assert_eq!(report.model, "agy:fake");
}

#[tokio::test(flavor = "multi_thread")]
async fn text_only_turns_are_nudged_then_stopped() {
    let dir = tempfile::tempdir().unwrap();
    let (report, proto, obs, _) = run("chatty", dir.path(), Limits::default(), false, fake()).await;
    assert_eq!(report.outcome, Outcome::NoProgress);
    assert!(proto.calls.lock().unwrap().is_empty());
    let nudges = obs
        .messages
        .lock()
        .unwrap()
        .iter()
        .filter(|m| m.starts_with("Text alone"))
        .count();
    assert_eq!(nudges, 2);
    assert_eq!(report.usage.output, 30, "three turns");
}

#[tokio::test(flavor = "multi_thread")]
async fn chat_run_completes_on_text() {
    let dir = tempfile::tempdir().unwrap();
    let (report, ..) = run("chatty", dir.path(), Limits::default(), true, fake()).await;
    assert_eq!(report.outcome, Outcome::Completed);
}

#[tokio::test(flavor = "multi_thread")]
async fn repeated_step_is_a_loop() {
    let dir = tempfile::tempdir().unwrap();
    let (report, ..) = run("loop", dir.path(), Limits::default(), false, fake()).await;
    assert_eq!(
        report.outcome,
        Outcome::LoopDetected {
            tool: "cli:view_file".into()
        }
    );
    assert_eq!(report.steps, 3);
}

#[tokio::test(flavor = "multi_thread")]
async fn step_limit_stops_with_a_checkpoint() {
    let dir = tempfile::tempdir().unwrap();
    let limits = Limits {
        max_steps: 2,
        timeout: Duration::from_secs(60),
        ..Limits::default()
    };
    let (report, ..) = run("steps", dir.path(), limits, false, fake()).await;
    assert!(
        matches!(&report.outcome, Outcome::Checkpoint { reason, .. } if reason == "max_steps"),
        "{:?}",
        report.outcome
    );
    assert_eq!(report.steps, 3);
}

#[tokio::test(flavor = "multi_thread")]
async fn missing_cli_fails_cleanly() {
    let dir = tempfile::tempdir().unwrap();
    let (report, ..) = run(
        "report",
        dir.path(),
        Limits::default(),
        false,
        dir.path().join("no-such-cli"),
    )
    .await;
    assert!(matches!(report.outcome, Outcome::Failed { .. }));
}
