//! Live text of a real CLI (the helpers' chats show it as it is written): runs one chat turn and reports when the
//! pieces arrived, and whether they make exactly the final answer (nothing shown twice, nothing missing).
//! `cargo run -p agenium-cli-backend --example cli_live -- claude [model] [effort: low|medium|high|max]`

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use agenium_backend::{Cancel, Effect, Observer, Protocol, WorkOrder};
use agenium_cli_backend::{CliKind, CliRun, Limits};
use agenium_llm::{ChatEvent, Message, Role, SystemBlock};
use agenium_tools::ToolError;
use async_trait::async_trait;

struct NoTools;

#[async_trait]
impl Protocol for NoTools {
    async fn call(
        &self,
        name: &str,
        _: serde_json::Value,
        _: &str,
    ) -> Result<(serde_json::Value, Effect), ToolError> {
        Err(ToolError::new(
            "no_tools",
            format!("{name}: answer in text"),
        ))
    }
}

/// When each piece came, and what it said.
struct Pieces {
    start: Instant,
    text: Mutex<Vec<(Duration, String)>>,
    thinking: Mutex<usize>,
}

impl Observer for Pieces {
    fn event(&self, e: &ChatEvent) {
        match e {
            ChatEvent::TextDelta(t) => self
                .text
                .lock()
                .unwrap()
                .push((self.start.elapsed(), t.clone())),
            ChatEvent::ThinkingDelta(t) => *self.thinking.lock().unwrap() += t.len(),
            _ => {}
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let kind = match args.first().map(String::as_str) {
        Some("claude") => CliKind::Claude,
        _ => return Err("usage: cli_live claude [model] [effort]".into()),
    };
    let model = args.get(1).cloned().unwrap_or_default();
    let effort: Option<agenium_core::Effort> = args
        .get(2)
        .map(|e| serde_json::from_value(serde_json::Value::String(e.clone())))
        .transpose()?;
    let dir = std::env::temp_dir().join(format!("agenium-cli-live-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let driver = agenium_cli_backend::driver(kind).ok_or("not driven")?;
    let info = agenium_cli_backend::detect::detect(kind);
    let program = driver.program(&info.path.ok_or("CLI not installed")?);
    let ctx = agenium_tools::ToolCtx::new(
        &dir,
        "cli-live",
        Arc::new(agenium_tools::DenyAll("no commands".into())),
    );
    let pieces = Pieces {
        start: Instant::now(),
        text: Mutex::default(),
        thinking: Mutex::default(),
    };
    let run = CliRun {
        driver,
        program,
        model: model.clone(),
        effort,
        env: ctx.env.clone(),
        cwd: dir.clone(),
        label: format!("{}:{model}", kind.program()),
        protocol: &NoTools,
        protocol_specs: vec![],
        tools: vec![],
        ctx: &ctx,
        session: None,
        observer: &pieces,
        cancel: Cancel::default(),
        limits: Limits {
            max_steps: 4,
            timeout: Duration::from_secs(300),
            ..Limits::default()
        },
        chat: true,
        nudge: None,
        subagents: false,
        read_only: true,
        uncontained: false,
        on_limits: None,
        steer: None,
    };
    let order = WorkOrder {
        system: vec![SystemBlock {
            text: "Answer in plain text.".into(),
            cache: false,
        }],
        messages: vec![Message::user_text(
            "Write about 300 words on how a small software team splits work between a lead, a planner and two \
             workers.",
        )],
    };
    let report = run.run(&order).await;
    let last = report
        .messages
        .iter()
        .rev()
        .find(|m| m.role == Role::Assistant && !m.text().trim().is_empty())
        .map(Message::text)
        .unwrap_or_default();
    let got = pieces.text.lock().unwrap().clone();
    let joined: String = got.iter().map(|(_, t)| t.as_str()).collect();
    println!("outcome: {:?}", report.outcome);
    println!(
        "pieces: {}, first at {:.1} s, last at {:.1} s; reasoning {} chars",
        got.len(),
        got.first().map_or(0.0, |p| p.0.as_secs_f64()),
        got.last().map_or(0.0, |p| p.0.as_secs_f64()),
        pieces.thinking.lock().unwrap()
    );
    println!("answer: {} chars", last.len());
    let live = got.len() > 1;
    let same = joined.trim() == last.trim();
    println!(
        "{}",
        if live && same {
            "PASS the answer came in pieces as it was written, and they make the answer exactly"
        } else if !live {
            "FAIL the answer came in one piece"
        } else {
            "FAIL the pieces do not make the answer (shown twice or missing)"
        }
    );
    let _ = std::fs::remove_dir_all(&dir);
    let _: PathBuf = dir;
    Ok(())
}
