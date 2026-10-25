//! `cargo run -p agenium-eval -- [--task ID] [--out DIR]` runs the golden tasks with the scripted model.
//! With a real model: `--root <agenium root> --provider <id> --model <model> [--effort high] [--approve-commands]`
//! (uses the provider and API key configured in that root; costs tokens).
//! With a vendor CLI (CliBackend): `--cli codex --model gpt-6-luna` or `--cli agy --model gemini-3.8-flash-low`
//! (uses the CLI's own login and quota; codex runs inside the sandbox, agy's commands need approval).

use std::path::PathBuf;
use std::sync::Arc;

use agenium_eval::{run_task, table, tasks, Mode};
use anyhow::{anyhow, Result};

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("AGENIUM_LOG")
                .unwrap_or_else(|_| "warn".into()),
        )
        .with_writer(std::io::stderr)
        .init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mode = match arg(&args, "--provider") {
        None if arg(&args, "--cli").is_some() => Mode::Cli {
            cli: arg(&args, "--cli").unwrap_or_default(),
            model: arg(&args, "--model").unwrap_or_default(),
            approve_commands: args.iter().any(|a| a == "--approve-commands"),
        },
        None => Mode::Scripted,
        Some(provider_id) => {
            let root = PathBuf::from(
                arg(&args, "--root")
                    .ok_or_else(|| anyhow!("--root is required with --provider"))?,
            );
            let keyring: Arc<dyn agenium_secrets::KeyringBackend> =
                Arc::new(agenium_secrets::OsKeyring::default());
            let (app, _) = agenium_orchestrator::App::open(
                &root,
                agenium_orchestrator::AppOptions {
                    keyring,
                    checks: Arc::new(agenium_orchestrator::NoCheckFactory),
                },
            )?;
            let source = agenium_orchestrator::runtime::ConfiguredProviders::new(Arc::new(app));
            let provider =
                agenium_orchestrator::runtime::ProviderSource::provider(&source, &provider_id)?;
            Mode::Model {
                provider,
                model: arg(&args, "--model")
                    .ok_or_else(|| anyhow!("--model is required with --provider"))?,
                effort: arg(&args, "--effort")
                    .unwrap_or_else(|| "high".into())
                    .parse()
                    .map_err(|e: String| anyhow!(e))?,
                approve_commands: args.iter().any(|a| a == "--approve-commands"),
            }
        }
    };
    let only = arg(&args, "--task");
    let mut results = Vec::new();
    // `--team`: the team task (M4 acceptance) instead of the single-agent tasks.
    if args.iter().any(|a| a == "--team") {
        eprintln!("running {} ...", agenium_eval::team::ID);
        results.push(agenium_eval::team::run(&mode).await?);
    }
    // `--learning`: the learning-loop scenario (ADR-91/92) instead of the single-agent tasks.
    if args.iter().any(|a| a == "--learning") {
        eprintln!("running {} ...", agenium_eval::learning::ID);
        results.push(agenium_eval::learning::run(&mode).await?);
    }
    for t in tasks()
        .iter()
        .filter(|_| !args.iter().any(|a| a == "--team" || a == "--learning"))
        .filter(|t| only.as_deref().is_none_or(|o| o == t.id))
    {
        eprintln!("running {} ...", t.id);
        results.push(run_task(t, &mode).await?);
    }
    print!("{}", table(&results));
    let out = PathBuf::from(arg(&args, "--out").unwrap_or_else(|| "quality/eval/results".into()));
    std::fs::create_dir_all(&out)?;
    let label = match &mode {
        Mode::Scripted => "scripted".to_owned(),
        Mode::Model { model, .. } => model.replace(['/', ':'], "_"),
        Mode::Cli { cli, model, .. } => format!("cli-{cli}-{model}").replace(['/', ':'], "_"),
    };
    let file = out.join(format!(
        "{}-{label}.json",
        chrono::Utc::now().format("%Y%m%dT%H%M%SZ")
    ));
    std::fs::write(&file, serde_json::to_vec_pretty(&results)?)?;
    eprintln!("results written to {}", file.display());
    if results.iter().any(|r| !r.passed) {
        std::process::exit(1);
    }
    Ok(())
}
