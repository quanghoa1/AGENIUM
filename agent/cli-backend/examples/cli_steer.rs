//! Real check of ADR-110: a message sent to a running CLI Lead is followed. The CLI reads three files and reports
//! through Agenium's `submit_report`; when it makes its first tool call the "user" asks for a word at the end of the
//! report. Uses the CLI's own login (subscription) and a scratch folder.
//!
//! cargo run -p agenium-cli-backend --example cli_steer -- <claude|grok|codex|agy> [model] [scratch-dir]

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agenium_backend::{Cancel, Effect, Observer, Protocol, Steer, WorkOrder};
use agenium_cli_backend::{CliKind, CliRun, Limits};
use agenium_llm::{Message, SystemBlock, ToolSpec};
use agenium_tools::ToolError;
use async_trait::async_trait;
use serde_json::{json, Value};

#[derive(Default)]
struct Report(Mutex<Option<String>>);

#[async_trait]
impl Protocol for Report {
    async fn call(&self, name: &str, args: Value, _: &str) -> Result<(Value, Effect), ToolError> {
        match name {
            "submit_report" => {
                *self.0.lock().unwrap() = Some(args["answer"].as_str().unwrap_or("").to_owned());
                Ok((json!("report accepted"), Effect::End))
            }
            "note" => Ok((json!("noted"), Effect::Continue)),
            _ => Err(ToolError::new("unknown_tool", name.to_owned())),
        }
    }
}

/// Sends the user's message at the CLI's first tool call and prints what happens.
struct Watch {
    steer: Steer,
    sent: Mutex<bool>,
}

impl Observer for Watch {
    fn message(&self, m: &Message) {
        let t = m.text();
        println!(
            "  message: {}",
            t.chars().take(100).collect::<String>().replace('\n', " ")
        );
    }
    fn tool(&self, name: &str, _: &Value, _: &Result<String, ToolError>) {
        println!("  tool: {name}");
        let mut sent = self.sent.lock().unwrap();
        if !*sent {
            *sent = true;
            self.steer
                .lock()
                .unwrap()
                .push("Also end your report with the word BANANA.".into());
            println!("  (user message queued)");
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let kind = match args.first().map(String::as_str) {
        Some("claude") => CliKind::Claude,
        Some("grok") => CliKind::Grok,
        Some("codex") => CliKind::Codex,
        Some("agy") => CliKind::Agy,
        _ => return Err("usage: cli_steer <claude|grok|codex|agy> [model] [scratch-dir]".into()),
    };
    let model = args.get(1).cloned().unwrap_or_default();
    let dir = args.get(2).map(PathBuf::from).unwrap_or_else(|| {
        std::env::temp_dir().join(format!("agenium-cli-steer-{}", std::process::id()))
    });
    let wt = dir.join("wt");
    std::fs::create_dir_all(&wt)?;
    for n in ["a.txt", "b.txt", "c.txt"] {
        std::fs::write(wt.join(n), format!("{n}: the {n} file\n"))?;
    }
    let driver = agenium_cli_backend::driver(kind).ok_or("not driven")?;
    let info = agenium_cli_backend::detect::detect(kind);
    let program = driver.program(&info.path.ok_or("CLI not installed")?);
    println!(
        "{} {} ({:?})",
        kind.program(),
        info.version.unwrap_or_default(),
        info.login
    );

    let steer: Steer = Default::default();
    let watch = Watch {
        steer: steer.clone(),
        sent: Mutex::new(false),
    };
    let report = Report::default();
    let ctx = agenium_tools::ToolCtx::new(
        &wt,
        "cli-steer",
        Arc::new(agenium_tools::DenyAll("no commands".into())),
    );
    let r = CliRun {
        driver,
        program,
        model: model.clone(),
        effort: None,
        env: Default::default(),
        cwd: wt.clone(),
        label: format!("{}:{model}", kind.program()),
        protocol: &report,
        protocol_specs: vec![
            ToolSpec {
                name: "submit_report".into(),
                description: "Finish: give your answer.".into(),
                parameters: json!({"type": "object", "properties": {"answer": {"type": "string"}}, "required": ["answer"]}),
            },
            ToolSpec {
                name: "note".into(),
                description: "Record your progress (one line).".into(),
                parameters: json!({"type": "object", "properties": {"text": {"type": "string"}}, "required": ["text"]}),
            },
        ],
        tools: vec![],
        ctx: &ctx,
        session: None,
        observer: &watch,
        cancel: Cancel::default(),
        limits: Limits {
            timeout: Duration::from_secs(600),
            ..Limits::default()
        },
        chat: false,
        nudge: None,
        subagents: false,
        read_only: false,
        uncontained: false,
        on_limits: None,
        steer: Some(steer.clone()),
    };
    let order = WorkOrder {
        system: vec![SystemBlock {
            text: "You are a careful agent working in a folder. Use your file tools; when done, call the agenium \
                   submit_report tool with your answer."
                .into(),
            cache: false,
        }],
        messages: vec![Message::user_text(
            "Read a.txt, then b.txt, then c.txt, one at a time; after each file call the agenium note tool with one \
             line about it. Then call submit_report with a one-line summary of them.",
        )],
    };
    let rep = r.run(&order).await;
    let answer = report.0.lock().unwrap().clone();
    let ok = answer.as_deref().is_some_and(|a| a.contains("BANANA"));
    println!("outcome: {:?}", rep.outcome);
    println!("answer: {answer:?}");
    println!("left in the queue: {:?}", steer.lock().unwrap());
    println!(
        "{}",
        if ok {
            "PASS a message sent during the run is followed"
        } else {
            "FAIL"
        }
    );
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}
