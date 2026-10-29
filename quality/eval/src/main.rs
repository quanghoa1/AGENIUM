//! `cargo run -p agenium-eval -- [--task ID] [--out DIR]` runs the golden tasks with the scripted model.
//! With a real model: `--root <agenium root> --provider <id> --model <model> [--effort high] [--approve-commands]`
//! (uses the provider and API key configured in that root; costs tokens). Or `--provider <id> --kind anthropic
//! --base-url default --key-env <VAR> --model <model>`: a provider built from a test key in the environment.
//! With a vendor CLI (CliBackend): `--cli codex --model gpt-6-luna` or `--cli agy --model gemini-3.8-flash-low`
//! (uses the CLI's own login and quota; codex runs inside the sandbox, agy's commands need approval).
//! Scenarios instead of the single-agent tasks: `--team`, `--learning` (ADR-91/92), `--mcp-keys` (two agents, one MCP
//! server, two keys with different roles; ADR-62), `--quota same|team|back` (a quota used up mid-ticket, the work goes on),
//! `--memory-reuse on|off` (memory
//! on or off for ticket 2; a CLI agent extracts memories with `--extract <provider> --extract-model <model>` and `--root`;
//! `--models <dir>` gives the agent the local embedding model from `<dir>`, else memory uses the hashing embedder).

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
            effort: arg(&args, "--effort")
                .unwrap_or_else(|| "high".into())
                .parse()
                .map_err(|e: String| anyhow!(e))?,
            approve_commands: args.iter().any(|a| a == "--approve-commands"),
        },
        None => Mode::Scripted,
        Some(provider_id) => {
            // Either a root's configured provider (key in its vault), or one built from `--kind`, `--base-url`
            // (`default` = the vendor's) and the key in the environment variable `--key-env` (a test key that must
            // not be stored).
            let provider: Arc<dyn agenium_llm::Provider> = match arg(&args, "--key-env") {
                Some(var) => {
                    let key = std::env::var(&var).map_err(|_| anyhow!("{var} is not set"))?;
                    agenium_llm::build_provider(&agenium_llm::ProviderSpec {
                        custom: Default::default(),
                        id: provider_id.clone(),
                        kind: arg(&args, "--kind").unwrap_or_else(|| "openai_compat".into()),
                        base_url: arg(&args, "--base-url").filter(|b| b != "default"),
                        api_key: Some(key),
                        max_concurrency: 2,
                    })?
                }
                None => {
                    let root = PathBuf::from(arg(&args, "--root").ok_or_else(|| {
                        anyhow!("--root or --key-env is required with --provider")
                    })?);
                    let keyring: Arc<dyn agenium_secrets::KeyringBackend> =
                        Arc::new(agenium_secrets::OsKeyring::default());
                    let (app, _) = agenium_orchestrator::App::open(
                        &root,
                        agenium_orchestrator::AppOptions {
                            keyring,
                            checks: Arc::new(agenium_orchestrator::NoCheckFactory),
                        },
                    )?;
                    let source =
                        agenium_orchestrator::runtime::ConfiguredProviders::new(Arc::new(app));
                    agenium_orchestrator::runtime::ProviderSource::provider(&source, &provider_id)?
                }
            };
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
    // `--compare <dir>`: two conditions of earlier runs with 95 % paired bootstrap intervals (stats.rs).
    if let Some(dir) = arg(&args, "--compare") {
        let a = arg(&args, "--a").unwrap_or_else(|| "memory-reuse-on".into());
        let b = arg(&args, "--b").unwrap_or_else(|| "memory-reuse-off".into());
        print!(
            "{}",
            agenium_eval::stats::compare(std::path::Path::new(&dir), &a, &b)?
        );
        return Ok(());
    }
    let only = arg(&args, "--task");
    let mut results = Vec::new();
    // `--team`: the team task (M4 acceptance) instead of the single-agent tasks.
    if args.iter().any(|a| a == "--team") {
        eprintln!("running {} ...", agenium_eval::team::ID);
        results.push(
            agenium_eval::team::run(
                &mode,
                arg(&args, "--worker-cli")
                    .map(|c| (c, arg(&args, "--worker-model").unwrap_or_default())),
            )
            .await?,
        );
    }
    // `--mcp-keys`: two workers reach one MCP server with their own keys, an admin's and a member's (ADR-62).
    if args.iter().any(|a| a == "--mcp-keys") {
        eprintln!("running {} ...", agenium_eval::mcp_keys::ID);
        results.push(
            agenium_eval::mcp_keys::run(
                &mode,
                arg(&args, "--worker-cli")
                    .map(|c| (c, arg(&args, "--worker-model").unwrap_or_default())),
            )
            .await?,
        );
    }
    // `--quota same|team|back`: a worker's quota runs out in the middle of a ticket, and the work goes on (quota.rs).
    if let Some(how) = arg(&args, "--quota") {
        eprintln!("running {}-{how} ...", agenium_eval::quota::ID);
        results.push(
            agenium_eval::quota::run(
                &mode,
                &how,
                arg(&args, "--worker-cli")
                    .map(|c| (c, arg(&args, "--worker-model").unwrap_or_default())),
            )
            .await?,
        );
    }
    // `--learning`: the learning-loop scenario (ADR-91/92) instead of the single-agent tasks.
    if args.iter().any(|a| a == "--learning") {
        eprintln!("running {} ...", agenium_eval::learning::ID);
        results.push(agenium_eval::learning::run(&mode).await?);
    }
    // `--memory-reuse on|off`: does memory make ticket 2 better (memory_reuse.rs).
    let reuse = arg(&args, "--memory-reuse");
    if let Some(on) = &reuse {
        let extract = match arg(&args, "--extract") {
            Some(id) => {
                let root = PathBuf::from(
                    arg(&args, "--root")
                        .ok_or_else(|| anyhow!("--root is required with --extract"))?,
                );
                let (app, _) = agenium_orchestrator::App::open(
                    &root,
                    agenium_orchestrator::AppOptions {
                        keyring: Arc::new(agenium_secrets::OsKeyring::default()),
                        checks: Arc::new(agenium_orchestrator::NoCheckFactory),
                    },
                )?;
                let source = agenium_orchestrator::runtime::ConfiguredProviders::new(Arc::new(app));
                let p = agenium_orchestrator::runtime::ProviderSource::provider(&source, &id)?;
                let model = arg(&args, "--extract-model")
                    .ok_or_else(|| anyhow!("--extract-model is required with --extract"))?;
                Some((p, model))
            }
            None => None,
        };
        eprintln!("running {}-{on} ...", agenium_eval::memory_reuse::ID);
        let models = arg(&args, "--models").map(PathBuf::from);
        results.push(
            agenium_eval::memory_reuse::run(
                &mode,
                on == "on",
                extract,
                models.as_deref(),
                args.iter().any(|a| a == "--chain"),
            )
            .await?,
        );
    }
    for t in tasks()
        .iter()
        .filter(|_| {
            !args
                .iter()
                .any(|a| a == "--team" || a == "--learning" || a == "--mcp-keys" || a == "--quota")
                && reuse.is_none()
        })
        .filter(|t| match only.as_deref() {
            Some(o) => o == t.id,
            None => !agenium_eval::OPT_IN.contains(&t.id),
        })
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
