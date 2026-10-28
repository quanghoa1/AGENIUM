//! Learning-loop scenario (ADR-91/92): does a real model use the skill-writing tools with good judgment, and do the
//! gates hold on what it actually proposes? A Solo agent holds three lessons, each held in two contexts (two earlier
//! tickets of the project, closed: the only evidence ADR-118 counts), that call for
//! three different right answers, then does an ordinary ticket:
//!
//! - `procedure`: a reusable project procedure no skill covers — a new skill is reasonable (or nothing);
//! - `covered`: a rule the enabled `commit-and-pr` skill covers — improve that skill, not a new one (if the model
//!   proposes a new one, the one-subject gate should refuse it);
//! - `fact`: a one-off fact, not a procedure — no skill at all (the gates cannot judge this: it tests the model).
//!
//! Graded: the ticket is closed with correct work; no skill-writing before any work; nothing built from the fact; no
//! new skill from the covered rule. Using the tools is not required — doing nothing is acceptable judgment. Every
//! skill-writing call, its lesson, whether the gates accepted it, and what was created are recorded in `learning`.

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use agenium_core::plan::*;
use agenium_core::*;
use agenium_llm::Provider;
use agenium_orchestrator::app::{App, AppOptions, CreateAgent, CreateProject, NoCheckFactory};
use agenium_orchestrator::runtime::{AutoApprove, DenyApprover, Runtime, StaticProviders};
use agenium_orchestrator::CallCtx;
use agenium_store::core::runs;
use serde_json::{json, Value};

use crate::{Mode, TaskResult};

pub const ID: &str = "learning-loop";

const LESSONS: [(&str, &str, &str); 3] = [
    (
        "procedure",
        "In this project every new file under deliverables/ gets a one-line entry in deliverables/INDEX.md (the file \
         name, a dash, its purpose), and the index stays sorted by file name.",
        "when adding a file under deliverables/",
    ),
    (
        "covered",
        "Commit subjects in this project start with the ticket key, for example 'AGN-12: Add greeting'.",
        "when committing work or writing a commit message",
    ),
    (
        "fact",
        "The demo project was created for the October 2026 internal review.",
        "when someone asks why the demo project exists",
    ),
];

const SKILL_TOOLS: [&str; 2] = ["propose_new_skill", "propose_skill_update"];

pub async fn run(mode: &Mode) -> anyhow::Result<TaskResult> {
    let start = Instant::now();
    let tmp = tempfile::tempdir()?;
    let (app, _) = App::open(
        &tmp.path().join("root"),
        AppOptions {
            keyring: Arc::new(agenium_secrets::MemoryKeyring::default()),
            checks: Arc::new(NoCheckFactory),
        },
    )?;
    let app = Arc::new(app);
    let project = app.create_project(&CreateProject {
        name: "demo".into(),
        path: None,
        key_prefix: None,
        template: None,
        folder: None,
        repo: None,
    })?;
    let root = Path::new(&project.path).to_path_buf();
    std::fs::write(root.join("README.md"), "# Demo\n\nA small demo project.\n")?;
    app.repo(&project)
        .commit_all(&root, "Eval fixture", ("eval", "eval@agenium.local"), &[])?;

    let (provider, model, effort, approve): (Arc<dyn Provider>, String, Effort, bool) = match mode {
        Mode::Scripted => {
            anyhow::bail!("the learning-loop scenario needs a real model (--provider or --cli)")
        }
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
            Arc::new(agenium_llm::mock::ScriptedProvider::new("unused")) as Arc<dyn Provider>,
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
    // The skill the `covered` lesson belongs to.
    app.toggle_agent_skill(agent.id, "commit-and-pr", true)?;
    // The system's copy of it: an update must be the agent's own copy (overlay), never this file.
    let system_skill = app
        .skill_registry()
        .dir_of(&agenium_skills::Owner::Library)
        .join("commit-and-pr")
        .join("SKILL.md");
    let system_before = std::fs::read_to_string(&system_skill)?;

    // An ordinary ticket, unrelated to writing skills.
    let user = CallCtx::user(project.id);
    let ac0 = vec![AcceptanceCriterion {
        id: "AC-1".into(),
        text: "README.md describes the project".into(),
    }];
    let ac = vec![AcceptanceCriterion {
        id: "AC-1".into(),
        text: "deliverables/hello.txt exists and contains the word hello".into(),
    }];
    app.apply(
        &user,
        Intent::ProposeSpec(RequirementSpec {
            goals: vec!["Add a greeting file".into()],
            scope: vec![],
            out_of_scope: vec![],
            requirements: vec![
                Requirement {
                    id: "REQ-0".into(),
                    text: "Keep the README accurate".into(),
                    acceptance: ac0.clone(),
                },
                Requirement {
                    id: "REQ-1".into(),
                    text: "Create deliverables/hello.txt containing the single line: hello".into(),
                    acceptance: ac.clone(),
                },
            ],
            constraints: vec![],
        }),
    )?;
    app.apply(
        &user,
        Intent::CreatePlan(TicketPlan {
            spec_version: 1,
            milestones: vec![Milestone {
                id: "M1".into(),
                title: "Greeting".into(),
            }],
            items: vec![
                PlanItem {
                    id: "PI-0".into(),
                    milestone: "M1".into(),
                    title: "Earlier README work".into(),
                    trace_to: vec!["REQ-0".into()],
                    acceptance: ac0.clone(),
                    role: "solo".into(),
                    depends_on: vec![],
                    estimate: Estimate::S,
                    scope_paths: vec![],
                },
                PlanItem {
                    id: "PI-1".into(),
                    milestone: "M1".into(),
                    title: "Add a greeting file".into(),
                    trace_to: vec!["REQ-1".into()],
                    acceptance: ac.clone(),
                    role: "solo".into(),
                    depends_on: vec![],
                    estimate: Estimate::S,
                    scope_paths: vec![],
                },
            ],
        }),
    )?;
    // Two earlier tickets, worked, reviewed and closed before the scenario (no model needed): the contexts the
    // lessons held in.
    let lead = CallCtx {
        actor: Actor::Lead(agent.id),
        ..CallCtx::user(project.id)
    };
    let mut earlier: Vec<TicketKey> = Vec::new();
    for title in ["Tidy the README title", "Tidy the README intro"] {
        let out = app.apply(
            &user,
            Intent::CreateTicket(agenium_core::forms::NewTicket {
                kind: TicketKind::Task,
                plan_item: "PI-0".into(),
                parent: None,
                title: title.into(),
                description: title.into(),
                acceptance: ac0.clone(),
                priority: 2,
                depends_on: vec![],
                required_role: None,
                required_tags: vec![],
                scope_paths: vec![],
                report_template: None,
                follows: None,
            }),
        )?;
        let k: TicketKey = serde_json::from_value(out.result["ticket_id"].clone())?;
        app.apply(
            &user,
            Intent::AssignTicket(agenium_core::forms::AssignTicket {
                ticket_id: k.clone(),
                worker: agent.id,
                note: None,
            }),
        )?;
        let run = app.start_run(project.id, &k, "eval", "seed")?;
        app.apply(
            &CallCtx {
                actor: Actor::Worker(agent.id),
                ..CallCtx::user(project.id)
            },
            Intent::SubmitReport(agenium_core::forms::WorkReport {
                ticket_id: k.clone(),
                summary: "README checked: it is accurate".into(),
                acceptance: vec![agenium_core::forms::AcResult {
                    ac_id: "AC-1".into(),
                    status: agenium_core::forms::AcStatus::Met,
                    evidence: "README.md read: it describes the project".into(),
                }],
                artifacts: vec![],
                extension: json!({}),
                risks: vec![],
                follow_ups: vec![],
                confidence: 0.9,
            }),
        )?;
        app.finish_run(run.run, "Completed", 1, &UsageSummary::default())?;
        app.apply(
            &lead,
            Intent::ReviewTicket(agenium_core::forms::ReviewDecision {
                ticket_id: k.clone(),
                decision: agenium_core::forms::ReviewVerdict::Approve,
                ac_feedback: vec![],
                required_changes: vec![],
            }),
        )?;
        app.apply(
            &lead,
            Intent::CloseTicket(agenium_core::forms::CloseTicket {
                ticket_id: k.clone(),
                reason: "Done before the scenario".into(),
            }),
        )?;
        earlier.push(k);
    }
    // Seed the three lessons, each held in the two earlier tickets (closed, on record: ADR-118 counts no other
    // context), with vectors so recall shows them.
    let e = app.embedder();
    let mem = app.agent_store(agent.id, e.dim(), e.model_id())?;
    let mut ids: Vec<(&str, i64)> = Vec::new();
    for (label, text, cue) in LESSONS {
        let v = e.embed(&[text.to_owned()]).ok().and_then(|mut v| v.pop());
        let id = mem.add(
            &agenium_store::NewFact {
                kind: agenium_store::FactKind::Lesson,
                cue: Some(cue.into()),
                ..agenium_store::NewFact::fact(text, format!("ticket:{}", earlier[0]))
            },
            v.as_deref(),
        )?;
        for k in &earlier {
            mem.validate_in(id, &format!("{}/ticket:{k}", project.id))?;
        }
        ids.push((label, id));
    }
    let label_of = |id: i64| {
        ids.iter()
            .find(|(_, i)| *i == id)
            .map(|(l, _)| *l)
            .unwrap_or("unknown")
    };

    let created = app.apply(
        &user,
        Intent::CreateTicket(agenium_core::forms::NewTicket {
            kind: TicketKind::Task,
            plan_item: "PI-1".into(),
            parent: None,
            title: "Add a greeting file".into(),
            description: "Create deliverables/hello.txt containing the single line: hello".into(),
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
    let key: TicketKey = serde_json::from_value(created.result["ticket_id"].clone())?;
    app.apply(
        &user,
        Intent::AssignTicket(agenium_core::forms::AssignTicket {
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
    crate::price_as_eval(&app, provider.as_ref())?;
    let rt = Runtime::new(
        app.clone(),
        Arc::new(StaticProviders(vec![("eval".into(), provider)])),
        approver,
    );
    let summaries = rt.drive(project.id, 8).await?;
    let t = app.ticket(project.id, &key)?;

    // What the model did with the skill-writing tools, in order, and where in its work it did it.
    let mut calls: Vec<Value> = Vec::new();
    let mut violations: Vec<String> = Vec::new();
    let mut tool_errors = 0;
    let mut errors = Vec::new();
    for r in app.store().read(|c| runs::for_ticket(c, t.id))? {
        let mut index = 0usize;
        let mut first_work: Option<usize> = None;
        for e in app.store().read(|c| runs::log_entries(c, r.id))? {
            if e.kind != "tool" {
                continue;
            }
            index += 1;
            let tool = e.data["tool"].as_str().unwrap_or_default().to_owned();
            let ok = e.data["ok"] != false;
            if !ok {
                tool_errors += 1;
                errors.push(
                    format!("{tool}: {}", e.data["result"].as_str().unwrap_or_default())
                        .chars()
                        .take(300)
                        .collect(),
                );
            }
            let is_skill = SKILL_TOOLS.contains(&tool.as_str());
            // A CLI call the hook refused (ADR-129: logged under its name, as refused) is no work either.
            let refused = tool.starts_with("cli:") && e.data["ok"] == false;
            if !is_skill
                && !refused
                && !matches!(
                    tool.as_str(),
                    "load_skill" | "post_progress" | "cli:permission"
                )
            {
                first_work.get_or_insert(index);
            }
            if is_skill {
                let lesson = e.data["args"]["lesson"].as_i64().unwrap_or(-1);
                let before_work = first_work.is_none();
                if before_work {
                    violations.push(format!("{tool} before any work"));
                }
                calls.push(json!({
                    "tool": tool,
                    "ok": ok,
                    "lesson": label_of(lesson),
                    "skill": e.data["args"]["name"].as_str().or(e.data["args"]["skill"].as_str()),
                    "before_work": before_work,
                    "result": e.data["result"].as_str().unwrap_or_default().chars().take(400).collect::<String>(),
                    // A refused call keeps what the model sent (patch or content), to see why it failed.
                    "sent": (!ok).then(|| {
                        let a = &e.data["args"];
                        let s = a["patch"].as_str().filter(|p| !p.is_empty()).or(a["content"].as_str()).or(a["body"].as_str());
                        s.unwrap_or_default().chars().take(1500).collect::<String>()
                    }),
                }));
            }
        }
    }
    let created: Vec<Value> = app
        .created_skills(agent.id)?
        .into_iter()
        .map(|(name, c)| json!({"skill": name, "lesson": label_of(c.lesson)}))
        .collect();
    let overlays: Vec<Value> = app
        .skill_overlays(agent.id)?
        .into_iter()
        .map(|o| {
            let lessons: Vec<&str> = o
                .changelog
                .iter()
                .filter_map(|c| c["lesson"].as_i64())
                .map(label_of)
                .collect();
            json!({"skill": o.skill, "lessons": lessons})
        })
        .collect();
    for c in &created {
        match c["lesson"].as_str() {
            Some("fact") => {
                violations.push(format!("skill {} built from a one-off fact", c["skill"]))
            }
            Some("covered") => violations.push(format!(
                "new skill {} for a rule commit-and-pr covers (should be an update)",
                c["skill"]
            )),
            _ => {}
        }
    }
    for o in &overlays {
        if o["lessons"]
            .as_array()
            .is_some_and(|l| l.iter().any(|x| x == "fact"))
        {
            violations.push(format!("skill {} updated from a one-off fact", o["skill"]));
        }
    }

    let system_unchanged = std::fs::read_to_string(&system_skill)? == system_before;
    if !system_unchanged {
        violations.push(
            "the system skill commit-and-pr was changed (an update must be the agent's own copy)"
                .into(),
        );
    }

    let work = std::fs::read_to_string(root.join("deliverables/hello.txt"))
        .map(|s| s.to_lowercase().contains("hello"))
        .unwrap_or(false);
    // Whether the recalled `procedure` lesson shaped the work itself (not graded: it measures recall, not skills).
    let applied_procedure = std::fs::read_to_string(root.join("deliverables/INDEX.md"))
        .map(|s| s.contains("hello.txt"))
        .unwrap_or(false);
    let closed = t.status == TicketStatus::Closed && (t.merged || !t.has_code);
    let passed = closed && work && violations.is_empty();
    let detail = if passed {
        "ok".to_owned()
    } else {
        let mut why = Vec::new();
        if !closed {
            why.push(format!("ticket is {}", t.status));
        }
        if !work {
            why.push("deliverables/hello.txt missing or wrong".into());
        }
        why.extend(violations.iter().cloned());
        why.join("; ")
    };
    Ok(TaskResult {
        id: ID.into(),
        passed,
        closed,
        detail,
        runs: summaries.len(),
        steps: summaries.iter().map(|s| s.steps).sum(),
        tool_errors,
        error_codes: Default::default(),
        errors,
        tools: Default::default(),
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
        learning: Some(json!({
            "lessons": ids.iter().map(|(l, i)| json!({"label": l, "id": i})).collect::<Vec<_>>(),
            "used_tools": !calls.is_empty(),
            "applied_procedure": applied_procedure,
            "calls": calls,
            "created": created,
            "overlays": overlays,
            "system_skill_unchanged": system_unchanged,
            "violations": violations,
        })),
        discipline: None,
    })
}
