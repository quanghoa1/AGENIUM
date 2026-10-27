//! Golden tasks (decision 38, M1-10): fixed coding tasks with fixture repositories, run end to end through the real
//! orchestrator (ticket -> work run -> review -> merge -> close) and graded automatically. The same tasks run with a
//! scripted model (harness regression, deterministic) or with a real model (behaviour eval; costs tokens).

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use agenium_core::forms::AssignTicket;
use agenium_core::plan::*;
use agenium_core::*;
use agenium_llm::mock::ScriptedProvider;
use agenium_llm::Provider;
use agenium_orchestrator::app::{App, AppOptions, CreateAgent, CreateProject, NoCheckFactory};
use agenium_orchestrator::runtime::{AutoApprove, DenyApprover, Runtime, StaticProviders};
use agenium_orchestrator::CallCtx;
use agenium_store::core::runs;
use serde::Serialize;
use serde_json::{json, Value};

pub mod learning;
pub mod memory_reuse;
pub mod team;

pub struct Task {
    pub id: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub acceptance: &'static [&'static str],
    pub fixture: &'static [(&'static str, &'static str)],
    /// Grades the project's `main` working tree after the run.
    pub check: fn(&Path) -> Result<(), String>,
    /// Tool calls of a correct solution (scripted mode), one turn per entry.
    pub solution: fn() -> Vec<(&'static str, Value)>,
}

fn read(root: &Path, rel: &str) -> Result<String, String> {
    std::fs::read_to_string(root.join(rel)).map_err(|e| format!("{rel}: {e}"))
}

fn walk_contains(dir: &Path, needle: &str) -> Vec<String> {
    let mut hits = vec![];
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                hits.extend(walk_contains(&p, needle));
            } else if std::fs::read_to_string(&p).is_ok_and(|s| s.contains(needle)) {
                hits.push(p.display().to_string());
            }
        }
    }
    hits
}

pub fn tasks() -> Vec<Task> {
    vec![
        Task {
            id: "hello-file",
            title: "Add a greeting file",
            description: "Create deliverables/hello.txt whose only content is the word hello.",
            acceptance: &["deliverables/hello.txt exists and contains exactly 'hello' (a trailing newline is fine)"],
            fixture: &[("README.md", "# Greeter\n")],
            check: |r| {
                let s = read(r, "deliverables/hello.txt")?;
                (s.trim() == "hello").then_some(()).ok_or_else(|| format!("content is {s:?}"))
            },
            solution: || vec![("write_file", json!({"path": "deliverables/hello.txt", "content": "hello\n"}))],
        },
        Task {
            id: "fix-config-key",
            title: "Fix the misspelled config key",
            description: "config/app.json has a misspelled key `debgu`. Rename it to `debug`, keep its value and keep every other key unchanged.",
            acceptance: &["config/app.json has \"debug\": true", "config/app.json has no key named debgu", "the name and port keys are unchanged"],
            fixture: &[("config/app.json", "{\n  \"name\": \"demo\",\n  \"debgu\": true,\n  \"port\": 8080\n}\n")],
            check: |r| {
                let v: Value = serde_json::from_str(&read(r, "config/app.json")?).map_err(|e| format!("invalid JSON: {e}"))?;
                if v["debug"] != json!(true) {
                    return Err("debug is not true".into());
                }
                if v.get("debgu").is_some() {
                    return Err("debgu is still there".into());
                }
                (v["name"] == "demo" && v["port"] == 8080).then_some(()).ok_or_else(|| "other keys changed".into())
            },
            solution: || {
                vec![
                    ("read_file", json!({"path": "config/app.json"})),
                    ("edit_file", json!({"path": "config/app.json", "old_string": "\"debgu\"", "new_string": "\"debug\""})),
                ]
            },
        },
        Task {
            id: "rename-function",
            title: "Rename a function everywhere",
            description: "Rename the Python function `old_name` (defined in src/mathlib.py) to `scale` and update every caller under src/.",
            acceptance: &["no file under src/ contains old_name", "src/mathlib.py defines scale", "src/main.py and src/report.py call scale"],
            fixture: &[
                ("src/mathlib.py", "def old_name(x):\n    return x * 2\n"),
                ("src/main.py", "from mathlib import old_name\n\nprint(old_name(21))\n"),
                ("src/report.py", "from mathlib import old_name\n\n\ndef report(v):\n    return f\"value={old_name(v)}\"\n"),
            ],
            check: |r| {
                let left = walk_contains(&r.join("src"), "old_name");
                if !left.is_empty() {
                    return Err(format!("old_name remains in {left:?}"));
                }
                if !read(r, "src/mathlib.py")?.contains("def scale(") {
                    return Err("scale is not defined".into());
                }
                for f in ["src/main.py", "src/report.py"] {
                    if !read(r, f)?.contains("scale(") {
                        return Err(format!("{f} does not call scale"));
                    }
                }
                Ok(())
            },
            solution: || {
                vec![(
                    "apply_patch",
                    json!({"patch": "*** Begin Patch\n*** Update File: src/mathlib.py\n@@\n-def old_name(x):\n+def scale(x):\n*** Update File: src/main.py\n@@\n-from mathlib import old_name\n+from mathlib import scale\n@@\n-print(old_name(21))\n+print(scale(21))\n*** Update File: src/report.py\n@@\n-from mathlib import old_name\n+from mathlib import scale\n@@\n-    return f\"value={old_name(v)}\"\n+    return f\"value={scale(v)}\"\n*** End Patch"}),
                )]
            },
        },
        Task {
            id: "readme-usage",
            title: "Document usage in the README",
            description: "Add a `## Usage` section at the end of README.md with a fenced code block that shows the command `demo --help`.",
            acceptance: &["README.md has a '## Usage' heading", "a fenced code block after it contains 'demo --help'", "the existing content is kept"],
            fixture: &[("README.md", "# Demo\n\nA demo tool.\n")],
            check: |r| {
                let s = read(r, "README.md")?;
                let i = s.find("## Usage").ok_or("no '## Usage' heading")?;
                let after = &s[i..];
                let fence = after.find("```").ok_or("no fenced block after the heading")?;
                if !after[fence..].contains("demo --help") {
                    return Err("the code block does not show demo --help".into());
                }
                s.starts_with("# Demo\n\nA demo tool.").then_some(()).ok_or_else(|| "existing content changed".into())
            },
            solution: || {
                vec![
                    ("read_file", json!({"path": "README.md"})),
                    ("edit_file", json!({"path": "README.md", "old_string": "A demo tool.\n", "new_string": "A demo tool.\n\n## Usage\n\n```sh\ndemo --help\n```\n"})),
                ]
            },
        },
        Task {
            id: "csv-total",
            title: "Compute the sales total",
            description: "Sum the amount column of data/sales.csv and write the total with exactly one decimal (e.g. 12.5) to deliverables/total.txt.",
            acceptance: &["deliverables/total.txt contains the sum of the amount column with one decimal"],
            fixture: &[("data/sales.csv", "region,amount\nnorth,120.5\nsouth,80.25\neast,99.25\nwest,200.0\n")],
            check: |r| {
                let s = read(r, "deliverables/total.txt")?;
                (s.trim() == "500.0").then_some(()).ok_or_else(|| format!("total is {:?}, expected 500.0", s.trim()))
            },
            solution: || {
                vec![
                    ("read_file", json!({"path": "data/sales.csv"})),
                    ("write_file", json!({"path": "deliverables/total.txt", "content": "500.0\n"})),
                ]
            },
        },
    ]
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskResult {
    pub id: String,
    pub passed: bool,
    pub closed: bool,
    pub detail: String,
    pub runs: usize,
    pub steps: u32,
    pub tool_errors: u32,
    /// Error codes of failed tool calls (e.g. command_denied, invalid).
    pub error_codes: std::collections::BTreeMap<String, u32>,
    /// `<tool>: <error>` of each failed call (first 300 characters).
    pub errors: Vec<String>,
    /// Outcome of each run, in order.
    pub outcomes: Vec<String>,
    pub reopened: u32,
    pub tokens_in: u64,
    pub tokens_out: u64,
    /// Input tokens served from the provider's prompt cache (part of `tokens_in`).
    #[serde(default)]
    pub tokens_cached: u64,
    pub cost_usd: f64,
    pub elapsed_ms: u64,
    /// The learning-loop scenario's observations (`learning` module); absent for the other tasks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub learning: Option<Value>,
    /// The team task's protocol checks and counts (ticket discipline, evidence, retries, nudges); absent elsewhere.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub discipline: Option<Value>,
}

pub enum Mode {
    /// Deterministic scripted model playing each task's correct solution.
    Scripted,
    Model {
        provider: Arc<dyn Provider>,
        model: String,
        effort: Effort,
        approve_commands: bool,
    },
    /// A vendor agent CLI through the CliBackend (M2.5): `cli` is its program name, `model` its model.
    Cli {
        cli: String,
        model: String,
        /// Passed to the CLI as its own effort flag (`--effort high` by default).
        effort: Effort,
        approve_commands: bool,
    },
}

/// Lets the app price the eval's runs. The model runs as provider `eval`, so a configured provider of that id and kind
/// is needed to look its price up: Anthropic's and Gemini's prices. An OpenAI-compatible provider needs its endpoint,
/// which the eval does not know, and stays unpriced.
pub fn price_as_eval(
    app: &agenium_orchestrator::App,
    provider: &dyn Provider,
) -> anyhow::Result<()> {
    let kind = match provider.kind() {
        "anthropic" => agenium_config::ProviderKind::Anthropic,
        "gemini" => agenium_config::ProviderKind::Gemini,
        _ => return Ok(()),
    };
    app.update_config(|c| {
        c.providers.retain(|p| p.id != "eval");
        c.providers.push(agenium_config::ProviderConfig {
            headers: Default::default(),
            id: "eval".into(),
            kind,
            base_url: None,
            api_key_ref: String::new(),
            max_concurrency: 8,
            models: vec![],
            model_caps: vec![],
        });
    })?;
    Ok(())
}

fn scripted(task: &Task) -> Arc<ScriptedProvider> {
    let p = Arc::new(ScriptedProvider::new("eval"));
    for (name, args) in (task.solution)() {
        p.push_calls("", vec![(name, args)]);
    }
    let acs: Vec<Value> = (0..task.acceptance.len())
        .map(|i| json!({"ac_id": format!("AC-{}", i + 1), "status": "Met", "evidence": "see diff"}))
        .collect();
    p.push_calls("", vec![("submit_report", json!({"ticket_id": "AGN-1", "summary": task.title, "acceptance": acs, "confidence": 0.9}))]);
    p.push_calls(
        "",
        vec![(
            "review_ticket",
            json!({"ticket_id": "AGN-1", "decision": "Approve"}),
        )],
    );
    p.push_calls(
        "",
        vec![(
            "close_ticket",
            json!({"ticket_id": "AGN-1", "reason": "All acceptance criteria met; merged"}),
        )],
    );
    p
}

/// Runs one task in a fresh temporary root.
pub async fn run_task(task: &Task, mode: &Mode) -> anyhow::Result<TaskResult> {
    let start = Instant::now();
    let tmp = tempfile::tempdir()?;
    let keyring = Arc::new(agenium_secrets::MemoryKeyring::default());
    let (app, _) = App::open(
        &tmp.path().join("root"),
        AppOptions {
            keyring,
            checks: Arc::new(NoCheckFactory),
        },
    )?;
    let app = Arc::new(app);
    let project = app.create_project(&CreateProject {
        name: task.id.into(),
        path: None,
        key_prefix: None,
        template: None,
        folder: None,
        repo: None,
    })?;
    let root = Path::new(&project.path).to_path_buf();
    for (rel, content) in task.fixture {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap_or(&root))?;
        std::fs::write(&p, content)?;
    }
    let repo = app.repo(&project);
    repo.commit_all(&root, "Eval fixture", ("eval", "eval@agenium.local"), &[])?;

    let (provider, model, effort, approve): (Arc<dyn Provider>, String, Effort, bool) = match mode {
        Mode::Scripted => (scripted(task), "scripted".into(), Effort::LOW, false),
        Mode::Model {
            provider,
            model,
            effort,
            approve_commands,
        } => (
            provider.clone(),
            model.clone(),
            effort.clone(),
            *approve_commands,
        ),
        Mode::Cli {
            model,
            effort,
            approve_commands,
            ..
        } => (
            Arc::new(ScriptedProvider::new("unused")) as Arc<dyn Provider>,
            model.clone(),
            effort.clone(),
            *approve_commands,
        ),
    };
    let (_, agent) = app.create_solo(
        project.id,
        &CreateAgent {
            name: "eval-agent".into(),
            role: "solo".into(),
            role_kind: RoleKind::Lead,
            tags: vec![],
            model: ModelBinding {
                provider: "eval".into(),
                model: model.clone(),
                effort: effort.clone(),
                max_tokens: None,
            },
        },
    )?;
    if let Mode::Cli { cli, model, .. } = mode {
        app.set_agent_backend(
            agent.id,
            BackendKind::Cli,
            &ModelBinding {
                provider: cli.clone(),
                model: model.clone(),
                effort,
                max_tokens: None,
            },
        )?;
    }
    let user = CallCtx::user(project.id);
    let acceptance: Vec<AcceptanceCriterion> = task
        .acceptance
        .iter()
        .enumerate()
        .map(|(i, t)| AcceptanceCriterion {
            id: format!("AC-{}", i + 1),
            text: t.to_string(),
        })
        .collect();
    app.apply(
        &user,
        Intent::ProposeSpec(RequirementSpec {
            goals: vec![task.title.into()],
            scope: vec![],
            out_of_scope: vec![],
            requirements: vec![Requirement {
                id: "REQ-1".into(),
                text: task.description.into(),
                acceptance: acceptance.clone(),
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
                title: task.title.into(),
            }],
            items: vec![PlanItem {
                id: "PI-1".into(),
                milestone: "M1".into(),
                title: task.title.into(),
                trace_to: vec!["REQ-1".into()],
                acceptance: acceptance.clone(),
                role: "solo".into(),
                depends_on: vec![],
                estimate: Estimate::S,
                scope_paths: vec![],
            }],
        }),
    )?;
    app.apply(
        &user,
        Intent::CreateTicket(agenium_core::forms::NewTicket {
            kind: TicketKind::Task,
            plan_item: "PI-1".into(),
            parent: None,
            title: task.title.into(),
            description: task.description.into(),
            acceptance,
            priority: 2,
            depends_on: vec![],
            required_role: None,
            required_tags: vec![],
            scope_paths: vec![],
            report_template: None,
        }),
    )?;
    let key: TicketKey = "AGN-1".parse()?;
    app.apply(
        &user,
        Intent::AssignTicket(AssignTicket {
            ticket_id: key.clone(),
            worker: agent.id,
            note: None,
        }),
    )?;

    let approver: Arc<dyn agenium_orchestrator::runtime::Approver> = if approve {
        Arc::new(AutoApprove)
    } else {
        Arc::new(DenyApprover)
    };
    price_as_eval(&app, provider.as_ref())?;
    let rt = Runtime::new(
        app.clone(),
        Arc::new(StaticProviders(vec![("eval".into(), provider)])),
        approver,
    );
    let summaries = rt.drive(project.id, 8).await?;
    let t = app.ticket(project.id, &key)?;
    let check = (task.check)(&root);
    let all_runs = app.store().read(|c| runs::for_ticket(c, t.id))?;
    let mut tool_errors = 0;
    let mut error_codes = std::collections::BTreeMap::new();
    let mut errors = Vec::new();
    let trace = std::env::var_os("AGENIUM_EVAL_TRACE").is_some();
    if trace {
        for op in app
            .store()
            .read(|c| agenium_store::core::ops::for_ticket(c, t.id))?
        {
            eprintln!("-- vcs op {op:?}");
        }
    }
    for r in &all_runs {
        if trace {
            eprintln!("-- run {} {:?} steps {}", r.id, r.outcome, r.steps);
        }
        for e in app.store().read(|c| runs::log_entries(c, r.id))? {
            if trace {
                let line: String = format!("   {} {}", e.kind, e.data)
                    .chars()
                    .take(400)
                    .collect();
                eprintln!("{line}");
            }
            if e.kind == "tool" && e.data["ok"] == false {
                tool_errors += 1;
                let code = e.data["result"]
                    .as_str()
                    .unwrap_or_default()
                    .split(':')
                    .next()
                    .unwrap_or_default()
                    .to_owned();
                *error_codes.entry(code).or_insert(0) += 1;
                let msg: String = format!(
                    "{}: {}",
                    e.data["tool"].as_str().unwrap_or_default(),
                    e.data["result"].as_str().unwrap_or_default()
                )
                .chars()
                .take(300)
                .collect();
                errors.push(msg);
            }
        }
    }
    let closed = t.status == TicketStatus::Closed && (t.merged || !t.has_code);
    let passed = closed && check.is_ok();
    let detail = match (&check, closed) {
        (Ok(()), true) => "ok".into(),
        (Ok(()), false) => format!("work is correct but the ticket is {}", t.status),
        (Err(e), _) => e.clone(),
    };
    Ok(TaskResult {
        id: task.id.into(),
        passed,
        closed,
        detail,
        runs: summaries.len(),
        steps: summaries.iter().map(|s| s.steps).sum(),
        tool_errors,
        error_codes,
        errors,
        outcomes: summaries
            .iter()
            .map(|s| format!("{}:{}", s.kind, s.outcome))
            .collect(),
        reopened: t.reopen_count,
        tokens_in: t.usage.tokens_in,
        tokens_cached: t.usage.cache_read,
        tokens_out: t.usage.tokens_out,
        cost_usd: t.usage.estimated_cost_usd,
        elapsed_ms: start.elapsed().as_millis() as u64,
        learning: None,
        discipline: None,
    })
}

pub fn table(results: &[TaskResult]) -> String {
    let mut s = format!(
        "{:<16} {:<6} {:>4} {:>5} {:>6} {:>7} {:>9} {:>6} {:>8} {:>8}  {}\n",
        "task",
        "result",
        "runs",
        "steps",
        "errors",
        "reopen",
        "tokens",
        "cache%",
        "cost$",
        "time_s",
        "detail"
    );
    for r in results {
        s.push_str(&format!(
            "{:<16} {:<6} {:>4} {:>5} {:>6} {:>7} {:>9} {:>6.1} {:>8.4} {:>8.1}  {}\n",
            r.id,
            if r.passed { "PASS" } else { "FAIL" },
            r.runs,
            r.steps,
            r.tool_errors,
            r.reopened,
            r.tokens_in + r.tokens_out,
            100.0 * r.tokens_cached as f64 / r.tokens_in.max(1) as f64,
            r.cost_usd,
            r.elapsed_ms as f64 / 1000.0,
            r.detail
        ));
    }
    let passed = results.iter().filter(|r| r.passed).count();
    let (cached, input) = results.iter().fold((0u64, 0u64), |(c, i), r| {
        (c + r.tokens_cached, i + r.tokens_in)
    });
    s.push_str(&format!(
        "score: {passed}/{} passed; prompt cache {:.1}% of {input} input tokens\n",
        results.len(),
        100.0 * cached as f64 / input.max(1) as f64
    ));
    s
}
