//! CliRun against a real installed CLI. Opt-in (spends the user's CLI quota):
//! `AGENIUM_REAL_CLI=devin [AGENIUM_REAL_CLI_MODEL=swe-2-medium] cargo test -p agenium-cli-backend --test real_cli -- --nocapture`

use std::path::Path;
use std::sync::{Arc, Mutex};

use agenium_backend::{Cancel, Effect, Observer, Outcome, Protocol, WorkOrder};
use agenium_cli_backend::{acp_args, CliKind, CliRun, Limits};
use agenium_llm::{Message, SystemBlock, ToolSpec};
use agenium_tools::{CommandGate, GateDecision, ToolError};
use async_trait::async_trait;
use serde_json::{json, Value};

#[derive(Default)]
struct Proto(Mutex<Vec<Value>>);

#[async_trait]
impl Protocol for Proto {
    async fn call(&self, name: &str, args: Value, _: &str) -> Result<(Value, Effect), ToolError> {
        eprintln!("PROTOCOL {name} {args}");
        self.0.lock().unwrap().push(args);
        Ok((json!("report accepted; the run ends"), Effect::End))
    }
}

struct DenyGate;

#[async_trait]
impl CommandGate for DenyGate {
    async fn check(&self, command: &str, _: &Path) -> GateDecision {
        GateDecision::Deny(format!("commands are not approved in this test: {command}"))
    }
}

struct Print;

impl Observer for Print {
    fn tool(&self, name: &str, args: &Value, r: &Result<String, ToolError>) {
        let r = match r {
            Ok(t) => format!("ok {}", t.chars().take(120).collect::<String>()),
            Err(e) => format!("ERR {e}"),
        };
        eprintln!(
            "TOOL {name} {} -> {r}",
            args.to_string().chars().take(160).collect::<String>()
        );
    }
    fn message(&self, m: &Message) {
        eprintln!("MSG {}", m.text().chars().take(200).collect::<String>());
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn real_cli_writes_a_file_and_reports() {
    let Ok(name) = std::env::var("AGENIUM_REAL_CLI") else {
        eprintln!("skipped: set AGENIUM_REAL_CLI=devin to run");
        return;
    };
    let kind = CliKind::ALL
        .into_iter()
        .find(|k| k.program() == name)
        .expect("known CLI");
    let info = agenium_cli_backend::detect::detect(kind);
    let program = info.path.expect("CLI installed");
    let model = std::env::var("AGENIUM_REAL_CLI_MODEL").unwrap_or_default();
    let args = acp_args(kind, &model).expect("CLI speaks ACP");
    let dir = tempfile::tempdir().unwrap();
    let proto = Proto::default();
    let ctx = agenium_tools::ToolCtx::new(
        dir.path(),
        "run-1",
        Arc::new(agenium_tools::DenyAll("no commands".into())),
    );
    let r = CliRun {
        program,
        args,
        env: Default::default(),
        cwd: dir.path().to_path_buf(),
        label: format!("{name}:{model}"),
        mode: Some("accept-edits".into()),
        protocol: &proto,
        tools: vec![],
        ctx: &ctx,
        protocol_specs: vec![ToolSpec {
            name: "submit_report".into(),
            description: "Report the result of the ticket. Ends the run.".into(),
            parameters: json!({"type": "object", "properties": {"summary": {"type": "string"}}, "required": ["summary"]}),
        }],
        gate: Arc::new(DenyGate),
        observer: &Print,
        cancel: Cancel::default(),
        limits: Limits {
            max_steps: 20,
            ..Limits::default()
        },
        chat: false,
    };
    let order = WorkOrder {
        system: vec![SystemBlock {
            text: "You are a worker agent. Finish every ticket with submit_report.".into(),
            cache: true,
        }],
        messages: vec![Message::user_text(
            "Ticket AGN-1: create the file hello.txt containing exactly the word hello. Do not run shell commands. \
             Then call submit_report with a one-line summary.",
        )],
    };
    let report = r.run(&order).await;
    eprintln!(
        "OUTCOME {:?} steps={} usage={:?}",
        report.outcome, report.steps, report.usage
    );
    assert_eq!(report.outcome, Outcome::Completed);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("hello.txt"))
            .unwrap()
            .trim(),
        "hello"
    );
    assert_eq!(proto.0.lock().unwrap().len(), 1);
}
