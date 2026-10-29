//! Does `find_code` take work off a coding agent (ADR-128)? The jevgrep measure: the same tasks done, at what cost.
//! Each question of a cases file is given to a real agent twice, read-only, on a repository: once with its usual read
//! tools and once with `find_code` added. Reported per run: whether the answer names the right file, the agent's
//! tokens (input incl. cache reads, output), steps, and the judge's tokens.
//!
//! The agent is an API model or an agent CLI; both work through Agenium's read tools (read_file, list_dir, glob, grep),
//! the CLI's reaching it through Agenium's MCP bridge as in a real run (ADR-129):
//!
//! cargo run -p agenium-eval --example find_code_ab -- [--repo <checkout> --cases <cases.json>]
//!     (--cli <claude|grok|codex|agy> [--model <id>]
//!      | --provider <id> --model <id> (--root <root with the provider> | --base-url <url> --key-env <VAR> [--kind ..]))
//!     [--only <n>]
//! and `JEV_KEY` in the environment. Keys are read from the environment or the root's vault only. Without `--repo`
//! and `--cases`: this repository and `data/find-code-cases.json`. A pinned outside repository keeps the answers fixed
//! while Agenium changes: `data/find-code-cases-flask.json` is for pallets/flask at tag 3.1.0.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use agenium_backend::*;
use agenium_cli_backend::CliKind;
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

const SYSTEM: &str = "You are a coding agent exploring a repository you have not seen. Use the tools to find the code, \
                      read enough of it to be sure, then call submit_report with the file path and the function that \
                      does it. Do not change anything.";

fn report_spec() -> ToolSpec {
    ToolSpec {
        name: "submit_report".into(),
        description: "Finish: give your answer.".into(),
        parameters: json!({"type": "object", "properties": {"answer": {"type": "string"}}, "required": ["answer"]}),
    }
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
    answer: String,
    tokens_in: u64,
    tokens_out: u64,
    steps: u32,
    calls: Vec<String>,
}

/// The answer names the file by the path the case gives (`sansio/app.py`, not any `app.py`).
fn names(answer: &str, file: &str) -> bool {
    answer
        .replace('\\', "/")
        .contains(file.trim_end_matches('/'))
}

fn order(question: &str) -> WorkOrder {
    WorkOrder {
        system: vec![SystemBlock {
            text: SYSTEM.into(),
            cache: false,
        }],
        messages: vec![Message::user_text(format!(
            "In this repository: {question}"
        ))],
    }
}

fn outcome(rep: RunReport, report: &Report, calls: &Calls, file: &str) -> Outcome {
    let answer = report.0.lock().unwrap().clone().unwrap_or_default();
    Outcome {
        right: names(&answer, file),
        answer,
        tokens_in: rep.usage.input + rep.usage.cache_read,
        tokens_out: rep.usage.output,
        steps: rep.steps,
        calls: calls.0.lock().unwrap().clone(),
    }
}

/// The agent under test.
enum Agent {
    Api(Arc<dyn Provider>),
    Cli(CliKind),
}

impl Agent {
    async fn run(
        &self,
        model: &str,
        root: &Path,
        case: &Case,
        extra: Option<Arc<dyn Tool>>,
    ) -> Outcome {
        match self {
            Agent::Api(p) => run_api(p.as_ref(), model, root, case, extra).await,
            Agent::Cli(k) => run_cli(*k, model, root, case, extra).await,
        }
    }
}

/// Agenium's read tools (read_file, list_dir, glob, grep) plus `extra`: what an API agent and, since ADR-129, a CLI
/// works through.
fn read_tools(extra: Option<Arc<dyn Tool>>) -> Vec<Arc<dyn Tool>> {
    let mut tools: Vec<Arc<dyn Tool>> = agenium_tools::work_tools()
        .into_iter()
        .filter(|t| matches!(t.name(), "read_file" | "list_dir" | "glob" | "grep"))
        .collect();
    tools.extend(extra);
    tools
}

/// One run of an API model with Agenium's read tools plus `extra`.
async fn run_api(
    provider: &dyn Provider,
    model: &str,
    root: &Path,
    case: &Case,
    extra: Option<Arc<dyn Tool>>,
) -> Outcome {
    let ctx = ToolCtx::new(root, "find-code-ab", Arc::new(AllowAll));
    let tools = read_tools(extra);
    let mut specs = work_specs(&tools);
    specs.push(report_spec());
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
    let rep = r.run(&order(&case.question)).await;
    drop(r);
    outcome(rep, &report, &calls, &case.file)
}

/// One run of an agent CLI as in a real run (ADR-129): Agenium's read tools and `extra` through its MCP bridge, its own
/// shell off and its own file tools limited to its scratch folder.
async fn run_cli(
    kind: CliKind,
    model: &str,
    root: &Path,
    case: &Case,
    extra: Option<Arc<dyn Tool>>,
) -> Outcome {
    let driver = agenium_cli_backend::driver(kind).expect("a driven CLI");
    let info = agenium_cli_backend::detect::detect(kind);
    let program = driver.program(&info.path.expect("the CLI is installed"));
    let ctx = ToolCtx::new(root, "find-code-ab", Arc::new(AllowAll));
    let report = Report::default();
    let calls = Calls::default();
    let r = agenium_cli_backend::CliRun {
        driver,
        program,
        model: model.to_owned(),
        effort: Some(Effort::LOW),
        env: Default::default(),
        cwd: root.to_path_buf(),
        label: format!("{}:{model}", kind.program()),
        protocol: &report,
        protocol_specs: vec![report_spec()],
        tools: read_tools(extra),
        ctx: &ctx,
        session: None,
        observer: &calls,
        cancel: Cancel::default(),
        limits: agenium_cli_backend::Limits {
            max_steps: 25,
            timeout: std::time::Duration::from_secs(600),
            ..Default::default()
        },
        chat: false,
        nudge: None,
        subagents: false,
        read_only: true,
        // As the runtime does: a sandboxed CLI runs on the host when no sandbox backend works here.
        uncontained: driver.control() == agenium_cli_backend::Control::Sandbox
            && agenium_sandbox::detect().backend.is_none(),
        on_limits: None,
        steer: None,
    };
    let rep = r.run(&order(&case.question)).await;
    drop(r);
    outcome(rep, &report, &calls, &case.file)
}

async fn api_provider(args: &[String]) -> Result<Arc<dyn Provider>> {
    let provider_id = arg(args, "--provider").ok_or_else(|| anyhow!("--provider or --cli"))?;
    Ok(match arg(args, "--base-url") {
        Some(base) => {
            let var = arg(args, "--key-env").ok_or_else(|| anyhow!("--key-env with --base-url"))?;
            build_provider(&ProviderSpec {
                custom: Default::default(),
                id: provider_id.clone(),
                kind: arg(args, "--kind").unwrap_or_else(|| "openai_compat".into()),
                base_url: Some(base),
                api_key: Some(std::env::var(&var).map_err(|_| anyhow!("{var} is not set"))?),
                max_concurrency: 2,
            })?
        }
        None => {
            let root =
                PathBuf::from(arg(args, "--root").ok_or_else(|| anyhow!("--root or --base-url"))?);
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
    })
}

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let model = arg(&args, "--model").unwrap_or_default();
    let agent = match arg(&args, "--cli").as_deref() {
        Some("claude") => Agent::Cli(CliKind::Claude),
        Some("grok") => Agent::Cli(CliKind::Grok),
        Some("codex") => Agent::Cli(CliKind::Codex),
        Some("agy") => Agent::Cli(CliKind::Agy),
        Some(other) => return Err(anyhow!("--cli {other}: claude, grok, codex or agy")),
        None if model.is_empty() => return Err(anyhow!("--model with --provider")),
        None => Agent::Api(api_provider(&args).await?),
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
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo = match arg(&args, "--repo") {
        Some(r) => PathBuf::from(r),
        None => here.join("../.."),
    }
    .canonicalize()?;
    let cases_file = arg(&args, "--cases")
        .map(PathBuf::from)
        .unwrap_or_else(|| here.join("data/find-code-cases.json"));
    let cases: Vec<Case> = serde_json::from_str(&std::fs::read_to_string(&cases_file)?)?;
    let only: Option<usize> = arg(&args, "--only").and_then(|n| n.parse().ok());
    let who = match &agent {
        Agent::Api(_) => format!("API {model}"),
        Agent::Cli(k) => format!(
            "{} {}",
            k.program(),
            if model.is_empty() {
                "(default model)"
            } else {
                &model
            }
        ),
    };
    println!(
        "## find_code A/B: {who} on {} (usual read tools vs + find_code)\n",
        repo.file_name().unwrap_or_default().to_string_lossy()
    );
    println!("| # | right (without / with) | tokens in (without / with) | out (without / with) | steps (without / with) | Jev tokens | tools used with find_code |");
    println!("|---|---|---|---|---|---|---|");
    // right without, right with, input without, input with, Jev input
    let mut tot = [0u64; 5];
    let mut answers = Vec::new();
    for (i, c) in cases.iter().enumerate().take(only.unwrap_or(cases.len())) {
        let a = agent.run(&model, &repo, c, None).await;
        let before = app.judge_activity(500).len();
        let b = agent
            .run(
                &model,
                &repo,
                c,
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
        let cut = |s: &str| s.chars().take(160).collect::<String>().replace('\n', " ");
        answers.push(format!(
            "{}. expected `{}`\n   without: {}\n   with:    {}",
            i + 1,
            c.file,
            cut(&a.answer),
            cut(&b.answer)
        ));
    }
    let n = only.unwrap_or(cases.len()).min(cases.len());
    println!(
        "\nRight: {}/{n} without, {}/{n} with find_code. Agent input tokens: {} without, {} with ({:+.1} %). Jev: {} \
         input tokens (≈ ${:.4}).",
        tot[0],
        tot[1],
        tot[2],
        tot[3],
        (tot[3] as f64 - tot[2] as f64) / tot[2].max(1) as f64 * 100.0,
        tot[4],
        tot[4] as f64 * 0.042 / 1e6
    );
    println!("\nAnswers:\n{}", answers.join("\n"));
    Ok(())
}
