//! Real check through the runtime (ADR-110, ADR-149): a Solo agent on a vendor CLI is messaged while its chat turn
//! runs; the message waits in the queue (no second turn starts beside it) and the Lead's next chat turn follows it.
//! Uses the CLI's own login.
//!
//! cargo run -p agenium-eval --example cli_lead_steer -- <claude|grok|codex> [model]

use std::sync::Arc;
use std::time::Duration;

use agenium_core::agent::{BackendKind, Effort, RoleKind};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cli = args
        .first()
        .cloned()
        .ok_or("usage: cli_lead_steer <cli> [model]")?;
    let model = args.get(1).cloned().unwrap_or_default();
    let tmp = tempfile::tempdir()?;
    let (app, _) = agenium_orchestrator::App::open(
        &tmp.path().join("root"),
        agenium_orchestrator::AppOptions {
            keyring: Arc::new(agenium_secrets::MemoryKeyring::default()),
            checks: Arc::new(agenium_orchestrator::NoCheckFactory),
        },
    )?;
    let app = Arc::new(app);
    app.update_config(|c| c.memory.learn = false)?;
    let p = app.create_project(&agenium_orchestrator::CreateProject {
        name: "notes".into(),
        path: None,
        key_prefix: None,
        template: None,
        folder: None,
        repo: None,
    })?;
    let binding = agenium_core::ModelBinding {
        provider: cli.clone(),
        model: model.clone(),
        effort: Effort::LOW,
        max_tokens: None,
    };
    let (_, solo) = app.create_solo(
        p.id,
        &agenium_orchestrator::CreateAgent {
            name: "ada".into(),
            role: "solo".into(),
            role_kind: RoleKind::Lead,
            tags: vec![],
            model: binding.clone(),
        },
    )?;
    app.set_agent_backend(solo.id, BackendKind::Cli, &binding)?;
    let rt = Arc::new(agenium_orchestrator::runtime::Runtime::new(
        app.clone(),
        Arc::new(agenium_orchestrator::runtime::StaticProviders(vec![])),
        Arc::new(agenium_orchestrator::runtime::AutoApprove),
    ));
    let turn = {
        let rt = rt.clone();
        tokio::spawn(async move {
            rt.chat(
                p.id,
                "Look at the project's tickets and plan with your Agenium tools, then tell me in two sentences \
                 what state the project is in.",
            )
            .await
        })
    };
    // The second message: queued as soon as the Lead's turn is going on.
    let started = std::time::Instant::now();
    let mut delivered = false;
    while started.elapsed() < Duration::from_secs(120) && !turn.is_finished() {
        if rt.steer(p.id, "Also end your answer with the word BANANA.")? {
            delivered = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    println!(
        "queued for the running turn: {delivered} (after {:?})",
        started.elapsed()
    );
    let s = turn.await??;
    println!("outcome: {}", s.outcome);
    // The queue opens the Lead's next chat turn once the first one ended.
    let next = rt.deliver_queued(p.id).await?;
    println!("next turn: {} {}", next.kind, next.outcome);
    let last = next.text.clone();
    println!("reply: {last}");
    let ok = delivered && next.kind == "chat" && last.contains("BANANA");
    println!(
        "{} a message to a running CLI Lead is queued and followed",
        if ok { "PASS" } else { "FAIL" }
    );
    Ok(())
}
