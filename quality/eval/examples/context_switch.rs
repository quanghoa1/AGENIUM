//! A measured scenario for ADR-155, run unchanged on an Agenium from before it and one after, with real models.
//!
//! `work`: a Solo agent audited log files on a 1M-token model (`beam-free`): its conversation of the ticket (38 files
//! read, its notes on them) is in place, as a run the app's restart interrupted. The user moves the agent to a model
//! with a 256K window (`iceman-contributor-free`) and the ticket goes on: two files are left, then
//! `deliverables/errors.md` with every error code of the 40 files. `--history tools` keeps the files in the tool
//! results (the agent read them); `--history text` has the agent's notes quote them (a history of long writing, as a
//! documentation ticket builds), which clearing tool results cannot shrink.
//!
//! `chat`: the Lead remembers that the user's services use PostgreSQL and Fly.io (a preference it learned), and the
//! user asks four questions in the chat: is the preference used, and what do the turns cost, the prompt cache
//! included.
//!
//!   cargo run -p agenium-eval --example context_switch -- work|chat [--history tools|text] --key-env VAR
//!     [--catalogue models-dev.json] [--models <dir with bge-small-en-v1.5>] --out report.json
//!
//! The OpenCode Go key is read from the environment variable named by `--key-env`, never stored.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use agenium_core::forms;
use agenium_core::plan::*;
use agenium_core::*;
use agenium_llm::{Message, Part, Role};
use agenium_orchestrator::app::{App, AppOptions, CreateAgent, CreateProject, NoCheckFactory};
use agenium_orchestrator::runtime::{AutoApprove, Runtime, StaticProviders};
use agenium_orchestrator::CallCtx;
use anyhow::{anyhow, Result};
use serde_json::json;

const PROVIDER: &str = "opencode-go";
const BIG: &str = "beam-free";
const SMALL: &str = "iceman-contributor-free";
const FILES: usize = 50;
const READ_BEFORE: usize = 48;

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

/// The error codes of log file `i` (1-based): two per file, all distinct.
fn codes(i: usize) -> [String; 2] {
    [format!("E{}", 100 + i * 2), format!("E{}", 101 + i * 2)]
}

/// A log file of about 27 000 characters, its two error lines among ordinary ones (under read_file's 30 000).
fn log(i: usize) -> String {
    let [a, b] = codes(i);
    let mut s = String::new();
    for n in 0..235 {
        let line = match n {
            70 => format!("2026-10-30T10:{:02}:{:02}Z ERROR svc-{i:02} payment gateway timed out code={a} order=O-{}", n % 60, (n * 7) % 60, 4000 + n),
            180 => format!("2026-10-30T11:{:02}:{:02}Z ERROR svc-{i:02} inventory lock lost code={b} sku=S-{}", n % 60, (n * 7) % 60, 9000 + n),
            _ => format!(
                "2026-10-30T10:{:02}:{:02}Z INFO svc-{i:02} request handled path=/api/v1/orders/{} status=200 took={}ms",
                n % 60,
                (n * 7) % 60,
                10_000 + n * 13,
                20 + (n * 17) % 300
            ),
        };
        s.push_str(&line);
        s.push('\n');
    }
    s
}

fn app_at(root: &Path, catalogue: Option<&Path>, models: Option<&Path>) -> Result<App> {
    let (app, _) = App::open(
        root,
        AppOptions {
            keyring: Arc::new(agenium_secrets::MemoryKeyring::default()),
            checks: Arc::new(NoCheckFactory),
        },
    )?;
    if let Some(c) = catalogue {
        std::fs::copy(c, root.join("models-dev.json"))?;
    }
    if let Some(dir) = models {
        let spec = agenium_memory::local::BGE_SMALL_EN;
        let (from, to) = (spec.dir(dir), spec.dir(&app.layout().models()));
        std::fs::create_dir_all(&to)?;
        for e in std::fs::read_dir(&from)? {
            let e = e?;
            if e.path().is_file() {
                std::fs::copy(e.path(), to.join(e.file_name()))?;
            }
        }
    }
    Ok(app)
}

fn binding(model: &str) -> ModelBinding {
    ModelBinding {
        provider: PROVIDER.into(),
        model: model.into(),
        effort: Effort::MEDIUM,
        max_tokens: None,
    }
}

fn usage_json(
    runs: &[agenium_store::core::runs::Run],
    helpers: &[agenium_store::core::helper_calls::HelperCall],
) -> serde_json::Value {
    let mut u = UsageSummary::default();
    for r in runs {
        u.add(&r.usage);
    }
    let mut h = UsageSummary::default();
    for c in helpers {
        h.add(&c.usage);
    }
    json!({
        "runs": runs.iter().map(|r| json!({"model": r.model, "outcome": r.outcome, "steps": r.steps,
            "tokens_in": r.usage.tokens_in, "tokens_out": r.usage.tokens_out, "cached": r.usage.cache_read})).collect::<Vec<_>>(),
        "run_tokens_in": u.tokens_in, "run_tokens_out": u.tokens_out, "run_cached": u.cache_read,
        "helper_calls": helpers.iter().map(|c| json!({"kind": c.kind, "tokens_in": c.usage.tokens_in, "tokens_out": c.usage.tokens_out})).collect::<Vec<_>>(),
        "helper_tokens_in": h.tokens_in, "helper_tokens_out": h.tokens_out,
        "total_tokens": u.tokens_in + u.tokens_out + h.tokens_in + h.tokens_out,
        "total_uncached_in": u.tokens_in.saturating_sub(u.cache_read) + h.tokens_in.saturating_sub(h.cache_read),
    })
}

async fn work(
    provider: Arc<dyn agenium_llm::Provider>,
    history: &str,
    catalogue: Option<&Path>,
    models: Option<&Path>,
) -> Result<serde_json::Value> {
    let tmp = tempfile::tempdir()?;
    // `AGENIUM_EVAL_KEEP=<dir>`: the root stays there for its run logs.
    let base = std::env::var("AGENIUM_EVAL_KEEP")
        .map(PathBuf::from)
        .unwrap_or_else(|_| tmp.path().to_path_buf());
    let app = Arc::new(app_at(&base.join("root"), catalogue, models)?);
    let project = app.create_project(&CreateProject {
        name: "logs".into(),
        path: None,
        key_prefix: None,
        template: None,
        folder: None,
        repo: None,
    })?;
    let root = PathBuf::from(&project.path);
    std::fs::create_dir_all(root.join("logs"))?;
    for i in 1..=FILES {
        std::fs::write(root.join(format!("logs/svc-{i:02}.log")), log(i))?;
    }
    app.repo(&project)
        .commit_all(&root, "The logs", ("eval", "eval@agenium.local"), &[])?;
    let (_, agent) = app.create_solo(
        project.id,
        &CreateAgent {
            name: "auditor".into(),
            role: "solo".into(),
            role_kind: RoleKind::Lead,
            tags: vec![],
            model: binding(BIG),
        },
    )?;
    let user = CallCtx::user(project.id);
    let title = "List every error code in the service logs";
    let description = format!(
        "There are {FILES} log files, logs/svc-01.log to logs/svc-{FILES:02}.log. Read each one and note every error code \
         (code=E followed by three digits). Then write deliverables/errors.md with one line per code, sorted by code: \
         `E123 svc-07.log`."
    );
    let ac = vec![
        AcceptanceCriterion {
            id: "AC-1".into(),
            text: format!("deliverables/errors.md lists all {} error codes of the {FILES} files, each with its file", FILES * 2),
        },
        AcceptanceCriterion {
            id: "AC-2".into(),
            text: "No code is listed that is not in the logs".into(),
        },
    ];
    app.apply(
        &user,
        Intent::ProposeSpec(RequirementSpec {
            goals: vec![title.into()],
            scope: vec![],
            out_of_scope: vec![],
            requirements: vec![Requirement {
                id: "REQ-1".into(),
                text: description.clone(),
                acceptance: ac.clone(),
            }],
            constraints: vec![],
        }),
    )?;
    app.apply(
        &user,
        Intent::CreatePlan(TicketPlan {
            spec_version: 1,
            milestones: vec![Milestone {
                id: "M1".into(),
                title: title.into(),
            }],
            items: vec![PlanItem {
                id: "PI-1".into(),
                milestone: "M1".into(),
                title: title.into(),
                trace_to: vec!["REQ-1".into()],
                acceptance: ac.clone(),
                role: "solo".into(),
                depends_on: vec![],
                estimate: Estimate::S,
                scope_paths: vec![],
            }],
        }),
    )?;
    let out = app.apply(
        &user,
        Intent::CreateTicket(forms::NewTicket {
            kind: TicketKind::Task,
            plan_item: "PI-1".into(),
            parent: None,
            title: title.into(),
            description: description.clone(),
            acceptance: ac,
            priority: 2,
            depends_on: vec![],
            required_role: None,
            required_tags: vec![],
            scope_paths: vec![],
            report_template: None,
            follows: None,
        }),
    )?;
    let key: TicketKey = serde_json::from_value(out.result["ticket_id"].clone())?;
    app.apply(
        &user,
        Intent::AssignTicket(forms::AssignTicket {
            ticket_id: key.clone(),
            worker: agent.id,
            note: None,
        }),
    )?;

    // The conversation the agent built on the 1M model: the work order, the files read one by one, its notes every
    // five files; ended by the app's restart (Interrupted), so the next run resumes it.
    let t = app.ticket(project.id, &key)?;
    let mut msgs: Vec<Message> = vec![Message::user_text(format!(
        "# Ticket {key}: {title}\nStatus: InProgress\nPlan item: PI-1\n\n{description}\n\nAcceptance criteria:\n- AC-1: {}\n- AC-2: {}\n",
        t.acceptance[0].text, t.acceptance[1].text
    ))];
    let mut noted: Vec<String> = Vec::new();
    for i in 1..=READ_BEFORE {
        let path = format!("logs/svc-{i:02}.log");
        let [a, b] = codes(i);
        noted.push(format!("{a} svc-{i:02}.log"));
        noted.push(format!("{b} svc-{i:02}.log"));
        let id = format!("call_{i}");
        let say = if history == "text" {
            // The agent quotes what it read in its own words (not a tool result: clearing cannot free it).
            format!(
                "Reading {path}. For the record, its content as I go through it:\n{}",
                log(i)
            )
        } else {
            format!("Reading {path}.")
        };
        msgs.push(Message {
            role: Role::Assistant,
            parts: vec![
                Part::Text { text: say },
                Part::ToolCall {
                    id: id.clone(),
                    name: "read_file".into(),
                    args: json!({"path": path}),
                    signature: None,
                },
            ],
        });
        let content = if history == "text" {
            format!("(read {path}: {} lines)", 235)
        } else {
            log(i)
        };
        msgs.push(Message {
            role: Role::User,
            parts: vec![Part::ToolResult {
                call_id: id,
                name: "read_file".into(),
                content,
                is_error: false,
            }],
        });
        if i % 5 == 0 || i == READ_BEFORE {
            msgs.push(Message {
                role: Role::Assistant,
                parts: vec![Part::Text {
                    text: format!("Codes so far ({i} files): {}", noted.join(", ")),
                }],
            });
            msgs.push(Message::user_text("Continue."));
        }
    }
    let chars: usize = msgs
        .iter()
        .map(|m| {
            m.text().len()
                + m.parts
                    .iter()
                    .map(|p| match p {
                        Part::ToolResult { content, .. } => content.len(),
                        _ => 0,
                    })
                    .sum::<usize>()
        })
        .sum();
    let run = RunId::new();
    let started = now_iso();
    app.store()
        .commit(None, "system", |w| -> agenium_orchestrator::Result<()> {
            agenium_store::core::runs::insert(
                w.tx,
                &agenium_store::core::runs::Run {
                    id: run,
                    ticket: Some(t.id),
                    agent: agent.id,
                    provider: PROVIDER.into(),
                    model: BIG.into(),
                    started_at: started.clone(),
                    ended_at: Some(started.clone()),
                    outcome: Some("Interrupted".into()),
                    steps: (READ_BEFORE * 2) as u32,
                    usage: UsageSummary::default(),
                    heartbeat: None,
                },
            )?;
            let mut tk = agenium_store::core::tickets::get(w.tx, t.id)?;
            tk.status = TicketStatus::InProgress;
            agenium_store::core::tickets::update(w.tx, &tk)?;
            Ok(())
        })?;
    let e = app.embedder();
    let mem = app.agent_store(agent.id, e.dim(), e.model_id())?;
    for m in &msgs {
        mem.append_message(&run.to_string(), &serde_json::to_value(m)?)?;
    }
    // The user moves the agent to the 256K model.
    app.set_agent_backend(agent.id, BackendKind::Api, &binding(SMALL))?;

    let rt = Runtime::new(
        app.clone(),
        Arc::new(StaticProviders(vec![(PROVIDER.into(), provider)])),
        Arc::new(AutoApprove),
    );
    let started = std::time::Instant::now();
    let summaries = rt.drive(project.id, 4).await?;
    let runs: Vec<_> = app
        .store()
        .read(|c| agenium_store::core::runs::for_ticket(c, t.id))?
        .into_iter()
        .filter(|r| r.id != run)
        .collect();
    let helpers = app
        .store()
        .read(|c| agenium_store::core::helper_calls::recent(c, 50))?;
    let ws = app
        .store()
        .read(|c| agenium_store::core::workspaces::get(c, t.id))?;
    // The file as merged into main (a closed ticket's worktree is gone), else as its worktree holds it.
    let merged = std::process::Command::new("git")
        .args(["show", "main:deliverables/errors.md"])
        .current_dir(&root)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned());
    let listed = merged
        .or_else(|| {
            ws.and_then(|w| {
                std::fs::read_to_string(Path::new(&w.worktree).join("deliverables/errors.md")).ok()
            })
        })
        .unwrap_or_default();
    let expected: Vec<String> = (1..=FILES).flat_map(|i| codes(i).to_vec()).collect();
    let found = expected
        .iter()
        .filter(|c| listed.contains(c.as_str()))
        .count();
    let early = (1..=READ_BEFORE)
        .flat_map(|i| codes(i).to_vec())
        .filter(|c| listed.contains(c.as_str()))
        .count();
    let wrong = listed
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|w| {
            w.len() == 4 && w.starts_with('E') && w[1..].chars().all(|c| c.is_ascii_digit())
        })
        .filter(|w| !expected.iter().any(|e| e == w))
        .count();
    let ticket = app.ticket(project.id, &key)?;
    let mut report = usage_json(&runs, &helpers);
    report["scenario"] = json!(format!("work/{history}"));
    report["resumed_conversation_chars"] = json!(chars);
    report["resumed_conversation_tokens_est"] = json!(chars / 4);
    report["outcomes"] = json!(summaries
        .iter()
        .map(|s| format!("{} {}", s.kind, s.outcome))
        .collect::<Vec<_>>());
    report["ticket_status"] = json!(format!("{:?}", ticket.status));
    report["codes_found"] = json!(format!("{found}/{}", expected.len()));
    report["codes_from_resumed_history"] = json!(format!("{early}/{}", READ_BEFORE * 2));
    report["codes_not_in_logs"] = json!(wrong);
    report["elapsed_s"] = json!(started.elapsed().as_secs());
    Ok(report)
}

async fn chat(
    provider: Arc<dyn agenium_llm::Provider>,
    catalogue: Option<&Path>,
    models: Option<&Path>,
) -> Result<serde_json::Value> {
    let tmp = tempfile::tempdir()?;
    let app = Arc::new(app_at(&tmp.path().join("root"), catalogue, models)?);
    let project = app.create_project(&CreateProject {
        name: "inventory".into(),
        path: None,
        key_prefix: None,
        template: None,
        folder: None,
        repo: None,
    })?;
    let (_, lead) = app.create_solo(
        project.id,
        &CreateAgent {
            name: "lead".into(),
            role: "solo".into(),
            role_kind: RoleKind::Lead,
            tags: vec![],
            model: binding(BIG),
        },
    )?;
    // What the Lead learned from earlier chats (ADR-98): the user's standard for new services.
    let e = app.embedder();
    let mem = app.agent_store(lead.id, e.dim(), e.model_id())?;
    mem.add(
        &agenium_store::NewFact {
            kind: agenium_store::FactKind::Preference,
            cue: Some("when choosing a database or hosting for a new service".into()),
            ..agenium_store::NewFact::fact(
                "The user's standard for every new service: the database is PostgreSQL 17 and it is deployed on Fly.io.",
                "chat:earlier",
            )
        },
        None,
    )?;
    let rt = Runtime::new(
        app.clone(),
        Arc::new(StaticProviders(vec![(PROVIDER.into(), provider)])),
        Arc::new(AutoApprove),
    );
    let asks = [
        "We are starting a new inventory service. Which database and which hosting would you pick for it? Answer in three lines, no tickets yet.",
        "List three risks of building it, one line each. No tickets.",
        "How would you split the work between two developers? Two lines, no tickets.",
        "Sum up the database and hosting you chose, in one line. No tickets.",
    ];
    let mut turns = Vec::new();
    for a in asks {
        let s = rt.chat(project.id, a).await?;
        turns.push(json!({"ask": a, "answer": s.text, "outcome": s.outcome,
            "tokens_in": s.usage.tokens_in, "cached": s.usage.cache_read, "tokens_out": s.usage.tokens_out}));
    }
    let lead_runs = app
        .store()
        .read(|c| agenium_store::core::runs::for_agent(c, lead.id, 50))?;
    let helpers = app
        .store()
        .read(|c| agenium_store::core::helper_calls::recent(c, 50))?;
    let first = turns[0]["answer"]
        .as_str()
        .unwrap_or_default()
        .to_lowercase();
    let last = turns[3]["answer"]
        .as_str()
        .unwrap_or_default()
        .to_lowercase();
    let mut report = usage_json(&lead_runs, &helpers);
    report["scenario"] = json!("chat");
    report["turns"] = json!(turns);
    report["first_answer_uses_preference"] =
        json!(first.contains("postgres") && first.contains("fly"));
    report["last_answer_uses_preference"] =
        json!(last.contains("postgres") && last.contains("fly"));
    Ok(report)
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
    let which = args.first().cloned().unwrap_or_default();
    let var = arg(&args, "--key-env").ok_or_else(|| anyhow!("--key-env is required"))?;
    let key = std::env::var(&var).map_err(|_| anyhow!("{var} is not set"))?;
    let provider = agenium_llm::build_provider(&agenium_llm::ProviderSpec {
        custom: Default::default(),
        id: PROVIDER.into(),
        kind: "openai_compat".into(),
        base_url: Some("https://opencode.ai/zen/go/v1".into()),
        api_key: Some(key),
        max_concurrency: 2,
    })?;
    let catalogue = arg(&args, "--catalogue").map(PathBuf::from);
    let models = arg(&args, "--models").map(PathBuf::from);
    let report = match which.as_str() {
        "work" => {
            work(
                provider,
                &arg(&args, "--history").unwrap_or_else(|| "tools".into()),
                catalogue.as_deref(),
                models.as_deref(),
            )
            .await?
        }
        "chat" => chat(provider, catalogue.as_deref(), models.as_deref()).await?,
        other => anyhow::bail!("work or chat, not {other:?}"),
    };
    let text = serde_json::to_string_pretty(&report)?;
    println!("{text}");
    if let Some(out) = arg(&args, "--out") {
        std::fs::write(out, &text)?;
    }
    Ok(())
}
