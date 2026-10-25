//! Memory-reuse benchmark (does memory make the agent better on the next ticket?). A pre-merge check enforces a project
//! rule the agent cannot know upfront — every team document, under `deliverables/` or `reports/`, starts with
//! `Owner: docs-team`. Ticket 1 (create `deliverables/hello.txt`) meets it the hard way: the merge fails with a message
//! stating the rule, the agent fixes the file, and closing the ticket runs the memory pipeline (the "check failed"
//! signal). Ticket 2 (create `reports/weekly.txt`) by the same agent is the measurement, with memory **on** (learn +
//! recall) or **off** (`[memory] learn = false`): did its first merge pass, how many runs, steps and tokens did it take,
//! and how many facts did recall show it. Ticket 2 writes into a folder ticket 1 never touched, so copying the shape of
//! `hello.txt` does not pass by itself; only the general rule does (the first design used `deliverables/bye.txt`, and
//! both conditions passed by imitating the file next to it).
//!
//! A CLI agent extracts memories with the `[cli]` fallback API model (ADR-56); pass `--extract <provider> --extract-model
//! <model>` with `--root` for those runs (DeepSeek on the OpenCode key).

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use agenium_core::comment::CommentBody;
use agenium_core::plan::*;
use agenium_core::*;
use agenium_llm::Provider;
use agenium_orchestrator::app::{App, AppOptions, CheckFactory, CreateAgent, CreateProject};
use agenium_orchestrator::runtime::{AutoApprove, DenyApprover, Runtime, StaticProviders};
use agenium_orchestrator::CallCtx;
use agenium_store::core::{comments, runs};
use agenium_vcs::{CheckOutcome, CheckRunner};
use serde_json::{json, Value};

use crate::{Mode, TaskResult};

pub const ID: &str = "memory-reuse";

const OWNER_LINE: &str = "Owner: docs-team";

/// The folders whose files are team documents.
const TEAM_DIRS: [&str; 2] = ["deliverables", "reports"];

/// The project's pre-merge check: every team document names its owning team on its first line.
struct OwnerRule;
impl CheckRunner for OwnerRule {
    fn run(&self, worktree: &Path) -> CheckOutcome {
        let mut bad = Vec::new();
        for dir in TEAM_DIRS {
            let Ok(rd) = std::fs::read_dir(worktree.join(dir)) else {
                continue;
            };
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') || !e.path().is_file() {
                    continue;
                }
                let first = std::fs::read_to_string(e.path())
                    .unwrap_or_default()
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .to_owned();
                if first != OWNER_LINE {
                    bad.push(format!("{dir}/{name}"));
                }
            }
        }
        if bad.is_empty() {
            CheckOutcome {
                ok: true,
                log: String::new(),
            }
        } else {
            CheckOutcome {
                ok: false,
                log: format!(
                    "pre-merge check failed: {}: the first line must be exactly '{OWNER_LINE}' (project rule: \
                     every team document, in deliverables/ or reports/, names its owning team on its first line)",
                    bad.join(", ")
                ),
            }
        }
    }
}

struct OwnerRuleFactory;
impl CheckFactory for OwnerRuleFactory {
    fn for_project(
        &self,
        _: &agenium_store::core::projects::Project,
        _: &agenium_orchestrator::manifest::ProjectManifest,
    ) -> Arc<dyn CheckRunner> {
        Arc::new(OwnerRule)
    }
}

fn milestones() -> Vec<Milestone> {
    vec![Milestone {
        id: "M1".into(),
        title: "Notes".into(),
    }]
}

fn plan_item(id: &str, title: &str, ac: &[AcceptanceCriterion]) -> PlanItem {
    PlanItem {
        id: id.into(),
        milestone: "M1".into(),
        title: title.into(),
        trace_to: vec!["REQ-1".into()],
        acceptance: ac.to_vec(),
        role: "solo".into(),
        depends_on: vec![],
        estimate: Estimate::S,
        scope_paths: vec![],
    }
}

/// Files on the project's main checkout, relative and sorted (dot entries skipped).
fn files(root: &Path) -> Vec<String> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            if e.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            let p = e.path();
            if p.is_dir() {
                walk(root, &p, out);
            } else if let Ok(rel) = p.strip_prefix(root) {
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out.sort();
    out
}

/// Per-ticket measurements.
fn measure(
    app: &App,
    project: ProjectId,
    key: &TicketKey,
    file: &str,
    word: &str,
    root: &Path,
) -> anyhow::Result<Value> {
    let t = app.ticket(project, key)?;
    let rs = app.store().read(|c| runs::for_ticket(c, t.id))?;
    let merge_failures = app
        .store()
        .read(|c| comments::for_ticket(c, t.id))?
        .iter()
        .filter(
            |c| matches!(&c.body, CommentBody::System { text } if text.starts_with("Merge failed")),
        )
        .count();
    let content = std::fs::read_to_string(root.join(file)).unwrap_or_default();
    Ok(json!({
        "ticket": key.to_string(),
        "status": t.status.to_string(),
        "closed": t.status == TicketStatus::Closed,
        "correct": content.lines().next().map(str::trim) == Some(OWNER_LINE) && content.to_lowercase().contains(word),
        "merge_failures": merge_failures,
        "first_merge_ok": merge_failures == 0,
        "reopened": t.reopen_count,
        "runs": rs.len(),
        "steps": rs.iter().map(|r| r.steps).sum::<u32>(),
        "tokens_in": t.usage.tokens_in,
        "tokens_out": t.usage.tokens_out,
        "files_after": files(root),
        "tickets_in_project": app.store().read(|c| agenium_store::core::tickets::list(c, project, &Default::default()))?.len(),
    }))
}

pub async fn run(
    mode: &Mode,
    memory_on: bool,
    extract: Option<(Arc<dyn Provider>, String)>,
) -> anyhow::Result<TaskResult> {
    let start = Instant::now();
    let tmp = tempfile::tempdir()?;
    let (app, _) = App::open(
        &tmp.path().join("root"),
        AppOptions {
            keyring: Arc::new(agenium_secrets::MemoryKeyring::default()),
            checks: Arc::new(OwnerRuleFactory),
        },
    )?;
    let app = Arc::new(app);
    let project = app.create_project(&CreateProject {
        name: "docs".into(),
        path: None,
        key_prefix: None,
        template: None,
        folder: None,
        repo: None,
    })?;
    let root = Path::new(&project.path).to_path_buf();
    std::fs::write(
        root.join("README.md"),
        "# Docs\n\nTeam documents live in deliverables/ and reports/.\n",
    )?;
    app.repo(&project)
        .commit_all(&root, "Eval fixture", ("eval", "eval@agenium.local"), &[])?;

    let (provider, model, effort, approve): (Arc<dyn Provider>, String, Effort, bool) = match mode {
        Mode::Scripted => {
            anyhow::bail!("the memory-reuse benchmark needs a real model (--provider or --cli)")
        }
        Mode::Model {
            provider,
            model,
            effort,
            approve_commands,
        } => (provider.clone(), model.clone(), *effort, *approve_commands),
        Mode::Cli {
            model,
            approve_commands,
            ..
        } => (
            Arc::new(agenium_llm::mock::ScriptedProvider::new("unused")) as Arc<dyn Provider>,
            model.clone(),
            Effort::High,
            *approve_commands,
        ),
    };
    let mut providers = vec![("eval".to_owned(), provider)];
    if let Some((p, m)) = &extract {
        providers.push(("extract".to_owned(), p.clone()));
        let m = m.clone();
        app.update_config(move |c| {
            c.cli.fallback_provider = "extract".into();
            c.cli.fallback_model = m;
        })?;
    }
    app.update_config(move |c| {
        c.memory.learn = memory_on;
        c.memory.share = false;
    })?;
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
                effort,
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

    let ac = |file: &str, word: &str| {
        vec![AcceptanceCriterion {
            id: "AC-1".into(),
            text: format!("{file} exists and contains the word {word}"),
        }]
    };
    let user = CallCtx::user(project.id);
    app.apply(
        &user,
        Intent::ProposeSpec(RequirementSpec {
            goals: vec!["Two short team documents".into()],
            scope: vec![],
            out_of_scope: vec![],
            requirements: vec![Requirement {
                id: "REQ-1".into(),
                text: "Short team documents in deliverables/ and reports/".into(),
                acceptance: ac("deliverables/hello.txt", "hello"),
            }],
            constraints: vec![],
        }),
    )?;
    app.apply(
        &user,
        Intent::CreatePlan(TicketPlan {
            spec_version: 1,
            milestones: milestones(),
            items: vec![plan_item(
                "PI-1",
                "Hello note",
                &ac("deliverables/hello.txt", "hello"),
            )],
        }),
    )?;
    let approver: Arc<dyn agenium_orchestrator::runtime::Approver> = if approve {
        Arc::new(AutoApprove)
    } else {
        Arc::new(DenyApprover)
    };
    let rt = Runtime::new(app.clone(), Arc::new(StaticProviders(providers)), approver);

    let mut tickets = Vec::new();
    for (item, file, word) in [
        ("PI-1", "deliverables/hello.txt", "hello"),
        ("PI-2", "reports/weekly.txt", "weekly"),
    ] {
        // The weekly report enters the plan only now: with it in plan v1 the Lead ticketed it on its own after ticket 1
        // (as reports/weekly.md), so the measured ticket was its third and its first merge could copy a reports/ file
        // already on main (the 2026-10-25 v2 runs).
        if item == "PI-2" {
            app.apply(
                &user,
                Intent::RevisePlan(PlanRevision {
                    base_version: 1,
                    reason: "Add the weekly report".into(),
                    milestones: milestones(),
                    items: vec![
                        plan_item("PI-1", "Hello note", &ac("deliverables/hello.txt", "hello")),
                        plan_item("PI-2", "Weekly report", &ac(file, word)),
                    ],
                }),
            )?;
        }
        let out = app.apply(
            &user,
            Intent::CreateTicket(agenium_core::forms::NewTicket {
                kind: TicketKind::Task,
                plan_item: item.into(),
                parent: None,
                title: format!("Add {file}"),
                description: format!("Create {file} containing the word {word}."),
                acceptance: ac(file, word),
                priority: 2,
                depends_on: vec![],
                required_role: None,
                required_tags: vec![],
                scope_paths: vec![],
                report_template: None,
            }),
        )?;
        let key: TicketKey = serde_json::from_value(out.result["ticket_id"].clone())?;
        app.apply(
            &user,
            Intent::AssignTicket(agenium_core::forms::AssignTicket {
                ticket_id: key.clone(),
                worker: agent.id,
                note: None,
            }),
        )?;
        rt.drive(project.id, 12).await?;
        tickets.push(measure(&app, project.id, &key, file, word, &root)?);
    }

    // What memory held after ticket 1, and what recall showed ticket 2.
    let e = app.embedder();
    let mem = app.agent_store(agent.id, e.dim(), e.model_id())?;
    let lessons: Vec<String> = mem
        .live(200)?
        .into_iter()
        .map(|f| format!("[{:?}] {}", f.kind, f.text))
        .collect();
    let t2 = app.ticket(
        project.id,
        &tickets[1]["ticket"].as_str().unwrap_or("AGN-2").parse()?,
    )?;
    let mut shown = 0usize;
    for r in app.store().read(|c| runs::for_ticket(c, t2.id))? {
        shown += mem.usage_for_run(&r.id.to_string())?.len();
    }

    let t1 = &tickets[0];
    let t2m = &tickets[1];
    let closed = t1["closed"] == true && t2m["closed"] == true;
    let correct = t1["correct"] == true && t2m["correct"] == true;
    Ok(TaskResult {
        id: format!("{ID}-{}", if memory_on { "on" } else { "off" }),
        passed: closed && correct,
        closed,
        detail: format!(
            "ticket 2: first merge {}, {} merge failure(s), {} steps; recall showed {shown} fact(s)",
            if t2m["first_merge_ok"] == true { "passed" } else { "failed" },
            t2m["merge_failures"],
            t2m["steps"]
        ),
        runs: tickets.iter().map(|t| t["runs"].as_u64().unwrap_or(0) as usize).sum(),
        steps: tickets.iter().map(|t| t["steps"].as_u64().unwrap_or(0) as u32).sum(),
        tool_errors: 0,
        error_codes: Default::default(),
        errors: vec![],
        outcomes: vec![],
        reopened: tickets.iter().map(|t| t["reopened"].as_u64().unwrap_or(0) as u32).sum(),
        tokens_in: tickets.iter().map(|t| t["tokens_in"].as_u64().unwrap_or(0)).sum(),
        tokens_out: tickets.iter().map(|t| t["tokens_out"].as_u64().unwrap_or(0)).sum(),
        cost_usd: 0.0,
        elapsed_ms: start.elapsed().as_millis() as u64,
        learning: Some(json!({
            "memory": memory_on,
            "ticket1": tickets[0],
            "ticket2": tickets[1],
            "memories_after": lessons,
            "ticket2_facts_shown": shown,
        })),
    })
}
