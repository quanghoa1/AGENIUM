//! Does `find_code` take work off a coding agent (ADR-128)? The jevgrep measure: the same tasks done, at what cost.
//! Each question of `data/find-code-cases.json` is given to a real model twice — with the usual read tools (read_file,
//! list_dir, glob, grep) and with `find_code` added — on this repository, read-only. Reported per run: whether the
//! answer names the right file, the model's tokens (input incl. cache reads, output), steps, and the judge's tokens.
//!
//! cargo run -p agenium-eval --example find_code_ab -- --provider <id> --model <id>
//!     (--root <root with the provider> | --base-url <url> --key-env <VAR> [--kind openai_compat]) [--only <n>]
//! and `JEV_KEY` in the environment. Keys are read from the environment or the root's vault only.

use std::path::PathBuf;
use std::sync::Arc;

use agenium_backend::*;
use agenium_core::Effort;
use agenium_llm::*;
use agenium_orchestrator::judge::JudgeInput;
use agenium_tools::{AllowAll, Tool, ToolCtx, ToolError};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde_json::{json, Value};

#[derive(serde::Deserialize)]
struct Case {
    question: String,
    file: String,
}

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

/// `submit_report {answer}` ends the run and keeps the answer.
#[derive(Default)]
struct Report(std::sync::Mutex<Option<String>>);

#[async_trait]
impl Protocol for Report {
    async fn call(&self, name: &str, args: Value, _id: &str) -> Result<(Value, Effect), ToolError> {
        match name {
            "submit_report" => {
                *self.0.lock().unwrap() = Some(args["answer"].as_str().unwrap_or("").to_owned());
                Ok((json!("report accepted"), Effect::End))
            }
            _ => Err(ToolError::new("unknown_tool", name.to_owned())),
        }
    }
}

/// The tool calls of a run, by name.
#[derive(Default)]
struct Calls(std::sync::Mutex<Vec<String>>);

impl Observer for Calls {
    fn tool(&self, name: &str, _args: &Value, _r: &std::result::Result<String, ToolError>) {
        self.0.lock().unwrap().push(name.to_owned());
    }
}

struct Outcome {
    right: bool,
    tokens_in: u64,
    tokens_out: u64,
    steps: u32,
    calls: Vec<String>,
}

async fn run(
    provider: &dyn Provider,
    model: &str,
    root: &std::path::Path,
    question: &str,
    file: &str,
    extra: Option<Arc<dyn Tool>>,
) -> Outcome {
    let ctx = ToolCtx::new(root, "find-code-ab", Arc::new(AllowAll));
    let mut tools: Vec<Arc<dyn Tool>> = agenium_tools::work_tools()
        .into_iter()
        .filter(|t| matches!(t.name(), "read_file" | "list_dir" | "glob" | "grep"))
        .collect();
    tools.extend(extra);
    let mut specs = work_specs(&tools);
    specs.push(ToolSpec {
        name: "submit_report".into(),
        description: "Finish: give your answer.".into(),
        parameters: json!({"type": "object", "properties": {"answer": {"type": "string"}}, "required": ["answer"]}),
    });
    let report = Report::default();
    let calls = Calls::default();
    let r = Run {
        provider,
        tools: &tools,
        ctx: &ctx,
        protocol: &report,
        specs: sorted_specs(specs),
        protocol_names: vec!["submit_report".into()],
        cfg: LoopConfig {
            model: model.to_owned(),
            effort: Effort::LOW,
            max_tokens: 4000,
            max_steps: 25,
            ..Default::default()
        },
        observer: &calls,
        cancel: Cancel::default(),
        chat: false,
        nudge: None,
        steer: None,
    };
    let order = WorkOrder {
        system: vec![SystemBlock {
            text: "You are a coding agent exploring a repository you have not seen. Use the tools to find the code, \
                   read enough of it to be sure, then call submit_report with the file path and the function that \
                   does it. Do not change anything."
                .into(),
            cache: false,
        }],
        messages: vec![Message::user_text(format!("In this repository: {question}"))],
    };
    let rep = r.run(&order).await;
    drop(r);
    let used = calls.0.lock().unwrap().clone();
    let answer = report.0.lock().unwrap().clone().unwrap_or_default();
    let name = file
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(file);
    Outcome {
        right: answer.replace('\\', "/").contains(name),
        tokens_in: rep.usage.input + rep.usage.cache_read,
        tokens_out: rep.usage.output,
        steps: rep.steps,
        calls: used,
    }
}

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let provider_id = arg(&args, "--provider").ok_or_else(|| anyhow!("--provider"))?;
    let model = arg(&args, "--model").ok_or_else(|| anyhow!("--model"))?;
    let provider: Arc<dyn Provider> = match arg(&args, "--base-url") {
        Some(base) => {
            let var =
                arg(&args, "--key-env").ok_or_else(|| anyhow!("--key-env with --base-url"))?;
            build_provider(&ProviderSpec {
                custom: Default::default(),
                id: provider_id.clone(),
                kind: arg(&args, "--kind").unwrap_or_else(|| "openai_compat".into()),
                base_url: Some(base),
                api_key: Some(std::env::var(&var).map_err(|_| anyhow!("{var} is not set"))?),
                max_concurrency: 2,
            })?
        }
        None => {
            let root =
                PathBuf::from(arg(&args, "--root").ok_or_else(|| anyhow!("--root or --base-url"))?);
            let (app, _) = agenium_orchestrator::App::open(
                &root,
                agenium_orchestrator::AppOptions {
                    keyring: Arc::new(agenium_secrets::OsKeyring::default()),
                    checks: Arc::new(agenium_orchestrator::NoCheckFactory),
                },
            )?;
            let source = agenium_orchestrator::runtime::ConfiguredProviders::new(Arc::new(app));
            agenium_orchestrator::runtime::ProviderSource::provider(&source, &provider_id)?
        }
    };
    let jev = std::env::var("JEV_KEY").map_err(|_| anyhow!("set JEV_KEY"))?;
    let tmp = tempfile::tempdir()?;
    let (app, _) = agenium_orchestrator::App::open(
        &tmp.path().join("root"),
        agenium_orchestrator::AppOptions {
            keyring: Arc::new(agenium_secrets::MemoryKeyring::default()),
            checks: Arc::new(agenium_orchestrator::NoCheckFactory),
        },
    )?;
    app.set_judge(&JudgeInput {
        base_url: "https://api.typesafe.ai".into(),
        model: "jev-1.14.0".into(),
        find_code: true,
        ..Default::default()
    })?;
    app.add_judge_key("eval", &jev)?;
    let app = Arc::new(app);
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let cases: Vec<Case> = serde_json::from_str(&std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data/find-code-cases.json"),
    )?)?;
    let only: Option<usize> = arg(&args, "--only").and_then(|n| n.parse().ok());
    println!("## find_code A/B on {model} (read tools only vs + find_code)\n");
    println!("| # | right (without / with) | model tokens in (without / with) | out (without / with) | steps (without / with) | Jev tokens | tools used with find_code |");
    println!("|---|---|---|---|---|---|---|");
    // right without, right with, input without, input with, Jev input
    let mut tot = [0u64; 5];
    for (i, c) in cases.iter().enumerate().take(only.unwrap_or(cases.len())) {
        let a = run(provider.as_ref(), &model, &repo, &c.question, &c.file, None).await;
        let before = app.judge_activity(500).len();
        let b = run(
            provider.as_ref(),
            &model,
            &repo,
            &c.question,
            &c.file,
            Some(agenium_orchestrator::judge::find_code::tool(
                app.clone(),
                None,
            )),
        )
        .await;
        let acts = app.judge_activity(500);
        let jev: u64 = acts[..acts.len() - before]
            .iter()
            .filter_map(|x| x.usage.as_ref()?["input_tokens"].as_u64())
            .sum();
        tot[0] += u64::from(a.right);
        tot[1] += u64::from(b.right);
        tot[2] += a.tokens_in;
        tot[3] += b.tokens_in;
        tot[4] += jev;
        println!(
            "| {} | {} / {} | {} / {} | {} / {} | {} / {} | {jev} | {} |",
            i + 1,
            if a.right { "yes" } else { "no" },
            if b.right { "yes" } else { "no" },
            a.tokens_in,
            b.tokens_in,
            a.tokens_out,
            b.tokens_out,
            a.steps,
            b.steps,
            b.calls.join(", ")
        );
    }
    let n = only.unwrap_or(cases.len()).min(cases.len());
    println!(
        "\nRight: {}/{n} without, {}/{n} with find_code. Model input tokens: {} without, {} with ({:+.1} %). Jev: {} \
         input tokens (≈ ${:.4}).",
        tot[0],
        tot[1],
        tot[2],
        tot[3],
        (tot[3] as f64 - tot[2] as f64) / tot[2].max(1) as f64 * 100.0,
        tot[4],
        tot[4] as f64 * 0.042 / 1e6
    );
    Ok(())
}
