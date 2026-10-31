//! `loop`: the whole app with real workers (see the module above).

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use agenium_core::plan::*;
use agenium_core::*;
use agenium_llm::Provider;
use agenium_memory::MemoryRetriever;
use agenium_orchestrator::app::{App, AppOptions, CreateAgent, CreateProject, NoCheckFactory};
use agenium_orchestrator::runtime::{AutoApprove, DenyApprover, Runtime, StaticProviders};
use agenium_orchestrator::CallCtx;
use agenium_store::core::{runs, workspaces};
use agenium_store::{FactKind, NewFact};
use serde_json::{json, Value};

use super::ID;
use crate::{Mode, TaskResult};

/// A provider that keeps what the memory extraction (an `Episode …` first message) was asked and answered, so a run's
/// result shows whether the lesson was listed to the learning and what the model said of it.
struct Recording {
    inner: Arc<dyn Provider>,
    seen: Arc<std::sync::Mutex<Vec<(String, String)>>>,
}

#[async_trait::async_trait]
impl Provider for Recording {
    fn id(&self) -> &str {
        self.inner.id()
    }
    fn kind(&self) -> &'static str {
        self.inner.kind()
    }
    async fn list_models(&self) -> Result<Vec<agenium_llm::ModelInfo>, agenium_llm::LlmError> {
        self.inner.list_models().await
    }
    async fn chat(
        &self,
        req: &agenium_llm::ChatRequest,
        emit: &(dyn Fn(agenium_llm::ChatEvent) + Send + Sync),
    ) -> Result<agenium_llm::ChatResponse, agenium_llm::LlmError> {
        let r = self.inner.chat(req, emit).await;
        let first = req.messages.first().map(|m| m.text()).unwrap_or_default();
        if first.starts_with("Episode ") {
            let answer = match &r {
                Ok(x) => x.message.text(),
                Err(e) => format!("error: {e}"),
            };
            self.seen
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push((first, answer));
        }
        r
    }
}

/// The "MEMORIES SHOWN" part of an extraction prompt, and the `misleading` the model answered.
fn shown_and_answer(seen: &[(String, String)], key: &str) -> Value {
    let Some((prompt, answer)) = seen
        .iter()
        .rev()
        .find(|(p, _)| p.starts_with(&format!("Episode ticket:{key} ")))
    else {
        return Value::Null;
    };
    let shown: Vec<&str> = prompt
        .split("## MEMORIES SHOWN to this run\n")
        .nth(1)
        .map(|s| s.lines().collect())
        .unwrap_or_default();
    let misleading = agenium_memory::pipeline::json_object(answer)
        .map(|v| v["misleading"].clone())
        .unwrap_or(Value::Null);
    json!({ "shown": shown, "misleading_answered": misleading })
}

/// The reopened tickets keep their worktrees, whose commands' temp folders lie outside the root (under the system
/// temp): they go with the root however the scenario ends, unless the root is kept.
struct RunTemps {
    dirs: Vec<std::path::PathBuf>,
    keep: bool,
}

impl Drop for RunTemps {
    fn drop(&mut self) {
        if !self.keep {
            for w in &self.dirs {
                agenium_sandbox::remove_run_temp_dir(w);
            }
        }
    }
}

/// The confidence of the live fact `id` became (following what superseded it), or -1 when it was retired.
fn live_end_confidence(
    mem: &agenium_store::AgentStore,
    id: i64,
) -> Result<f64, agenium_store::StoreError> {
    let mut f = mem.get(id)?;
    for _ in 0..100 {
        match f.superseded_by {
            None => return Ok(f.confidence),
            Some(next) if next == f.id => return Ok(-1.0),
            Some(next) => f = mem.get(next)?,
        }
    }
    Ok(-1.0)
}

/// The pool lesson of the loop: right in another agent's project, wrong here.
const POOLED: &str =
    "Every team document in this project starts with the line 'Owner: platform-team'.";
const POOLED_CUE: &str = "when writing a team document";
const OWN: &str = "Text files in this project end with a newline.";

/// `loop`: two tickets through the app with a real worker, each reopened by the user and learned from.
pub async fn full_loop(
    mode: &Mode,
    extract: Option<(Arc<dyn Provider>, String)>,
    models: Option<&Path>,
    keep: Option<&Path>,
) -> anyhow::Result<TaskResult> {
    let start = Instant::now();
    let tmp = tempfile::tempdir()?;
    // `--keep <dir>`: the root stays there, to be opened in the desktop app (a UI-test root: its vault key is not kept).
    let root_dir = match keep {
        Some(dir) => {
            anyhow::ensure!(
                !dir.exists(),
                "{} exists: --keep needs a new folder",
                dir.display()
            );
            dir.to_path_buf()
        }
        None => tmp.path().join("root"),
    };
    let (app, _) = App::open(
        &root_dir,
        AppOptions {
            keyring: Arc::new(agenium_secrets::MemoryKeyring::default()),
            checks: Arc::new(NoCheckFactory),
        },
    )?;
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
    let app = Arc::new(app);
    let (provider, model, effort, approve): (Arc<dyn Provider>, String, Effort, bool) = match mode {
        Mode::Scripted => {
            anyhow::bail!("the shared-pool loop needs a real model (--provider or --cli)")
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
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let record = |inner: Arc<dyn Provider>| -> Arc<dyn Provider> {
        Arc::new(Recording {
            inner,
            seen: seen.clone(),
        })
    };
    let mut providers = vec![("eval".to_owned(), record(provider))];
    if let Some((p, m)) = &extract {
        providers.push(("extract".to_owned(), record(p.clone())));
        let m = m.clone();
        app.update_config(move |c| {
            c.cli.fallback_provider = "extract".into();
            c.cli.fallback_model = m;
        })?;
    }
    app.update_config(|c| {
        c.memory.learn = true;
        c.memory.share = true;
    })?;

    let e = app.embedder();
    let vector = |t: &str| e.embed(&[t.to_owned()]).ok().and_then(|mut v| v.pop());
    let lesson = |text: &str, cue: &str, source: &str, confidence: f64| NewFact {
        kind: FactKind::Lesson,
        cue: Some(cue.into()),
        confidence,
        ..NewFact::fact(text, source)
    };
    let ac = |file: &str, word: &str| {
        vec![AcceptanceCriterion {
            id: "AC-1".into(),
            text: format!("{file} is a team document: it names its owning team on its first line and contains the word {word}"),
        }]
    };
    // Two projects, each with its own Solo agent and one ticket: the pool's lesson reaches two different agents, as it
    // does in use (one agent that learned the right rule on its first reopen does not follow the lesson again).
    let seats = [
        ("docs", "eval-a", "deliverables/hello.txt", "hello"),
        ("notes", "eval-b", "reports/weekly.txt", "weekly"),
    ];
    let mut placed = Vec::new();
    for (name, agent_name, file, word) in seats {
        let project = app.create_project(&CreateProject {
            name: name.into(),
            path: None,
            key_prefix: None,
            template: None,
            folder: None,
            repo: None,
        })?;
        let root = Path::new(&project.path).to_path_buf();
        std::fs::write(
            root.join("README.md"),
            "# Docs\n\nTeam documents live in deliverables/ and reports/. Each names its owning team on its first \
             line, as `Owner: <team>`.\n",
        )?;
        app.repo(&project).commit_all(
            &root,
            "Eval fixture",
            ("eval", "eval@agenium.local"),
            &[],
        )?;
        let (_, agent) = app.create_solo(
            project.id,
            &CreateAgent {
                name: agent_name.into(),
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
                    effort: effort.clone(),
                    max_tokens: None,
                },
            )?;
        }
        // The agent's own lesson, shown beside the pool's: the pool's blame must never reach it.
        let mem = app.agent_store(agent.id, e.dim(), e.model_id())?;
        let own = mem.add(
            &lesson(OWN, "when writing a text file", "ticket:DOC-1", 0.7),
            vector(OWN).as_deref(),
        )?;
        let user = CallCtx::user(project.id);
        app.apply(
            &user,
            Intent::ProposeSpec(RequirementSpec {
                goals: vec!["A short team document".into()],
                scope: vec![],
                out_of_scope: vec![],
                requirements: vec![Requirement {
                    id: "REQ-1".into(),
                    text: format!("A short team document, {file}"),
                    acceptance: ac(file, word),
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
                    title: "Notes".into(),
                }],
                items: vec![PlanItem {
                    id: "PI-1".into(),
                    milestone: "M1".into(),
                    title: format!("Add {file}"),
                    trace_to: vec!["REQ-1".into()],
                    acceptance: ac(file, word),
                    role: "solo".into(),
                    depends_on: vec![],
                    estimate: Estimate::S,
                    scope_paths: vec![],
                }],
            }),
        )?;
        placed.push((project, agent, mem, own, file, word));
    }

    // Another agent's lesson, copied into the pool as sharing copies it.
    let owner = AgentId::new();
    let owner_mem = app.agent_store(owner, e.dim(), e.model_id())?;
    let original = owner_mem.add(
        &lesson(POOLED, POOLED_CUE, "ticket:OPS-4", 0.9),
        vector(POOLED).as_deref(),
    )?;
    let pool = app.pool_memory()?;
    let copy = pool.add(
        &lesson(
            POOLED,
            POOLED_CUE,
            &format!("shared:{owner}:{original}"),
            0.6,
        ),
        vector(POOLED).as_deref(),
    )?;

    let approver: Arc<dyn agenium_orchestrator::runtime::Approver> = if approve {
        Arc::new(AutoApprove)
    } else {
        Arc::new(DenyApprover)
    };
    crate::price_as_eval(&app, providers[0].1.as_ref())?;
    let rt = Runtime::new(app.clone(), Arc::new(StaticProviders(providers)), approver);

    let mut steps = Vec::new();
    let mut temps = RunTemps {
        dirs: Vec::new(),
        keep: keep.is_some(),
    };
    let mut blamed = 0;
    let mut all_ok = true;
    for (project, agent, _, own, file, word) in &placed {
        let (file, word, own) = (*file, *word, *own);
        let user = CallCtx::user(project.id);
        let out = app.apply(
            &user,
            Intent::CreateTicket(agenium_core::forms::NewTicket {
                kind: TicketKind::Task,
                plan_item: "PI-1".into(),
                parent: None,
                title: format!("Add {file}"),
                description: format!("Create {file}, a team document containing the word {word}."),
                acceptance: ac(file, word),
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
            Intent::AssignTicket(agenium_core::forms::AssignTicket {
                ticket_id: key.clone(),
                worker: agent.id,
                note: None,
            }),
        )?;
        temps
            .dirs
            .push(app.repo(project).worktree_path(key.as_str()));
        let work = rt.run_ticket(project.id, &key).await?;
        let t = app.ticket(project.id, &key)?;
        // What the work wrote, in its worktree (not merged: the user reviews it now).
        let ws = app.store().read(|c| workspaces::get(c, t.id))?;
        let content = ws
            .as_ref()
            .and_then(|w| std::fs::read_to_string(Path::new(&w.worktree).join(file)).ok())
            .unwrap_or_default();
        let first = content.lines().next().unwrap_or_default().trim().to_owned();
        let followed = first.contains("platform-team");
        // Shown to the work run, and logged in the pool's usage.
        let shown_shared = app
            .store()
            .read(|c| runs::log_entries(c, work.run))?
            .iter()
            .filter(|e| e.kind == "memory")
            .flat_map(|e| e.data["recalled"].as_array().cloned().unwrap_or_default())
            .any(|r| r["shared"] == true && r["text"] == POOLED);
        let in_pool_usage = pool
            .usage_for_run(&work.run.to_string())?
            .iter()
            .any(|(f, _, _)| *f == copy);
        let resolved = t.status == TicketStatus::Resolved;
        let mut report = Value::Null;
        if resolved {
            // The user reopens: on the pool's lesson when the work followed it, else on something no lesson made.
            let feedback = if followed {
                "The first line says 'Owner: platform-team'; this project's documents belong to the docs team: \
                 'Owner: docs-team'."
            } else {
                "The word must be written in capital letters in this note."
            };
            app.apply(
                &user,
                Intent::ReviewTicket(agenium_core::forms::ReviewDecision {
                    ticket_id: key.clone(),
                    decision: agenium_core::forms::ReviewVerdict::Reopen,
                    ac_feedback: vec![agenium_core::forms::AcFeedback {
                        ac_id: "AC-1".into(),
                        feedback: feedback.into(),
                    }],
                    required_changes: vec![],
                }),
            )?;
            report = match rt.learn_from_ticket(project.id, &key, work.run).await? {
                Some(r) => serde_json::to_value(&r)?,
                None => Value::Null,
            };
        }
        let named_shared = report["misleading_shared"]
            .as_array()
            .is_some_and(|a| a.contains(&json!(copy)));
        let named_own = report["misleading"]
            .as_array()
            .is_some_and(|a| a.contains(&json!(own)));
        if named_shared {
            blamed += 1;
        }
        // The code's part: the lesson reached the run and the pool's usage. The model's: the blame follows the work.
        let plumbing = shown_shared && in_pool_usage;
        let judged = resolved && !report.is_null() && named_shared == followed && !named_own;
        all_ok &= plumbing && judged;
        steps.push(json!({
            "project": project.name,
            "ticket": key.to_string(),
            "work_outcome": work.outcome,
            "resolved": resolved,
            "first_line": first,
            "followed_the_pool_lesson": followed,
            "shown_to_the_run": shown_shared,
            "in_pool_usage": in_pool_usage,
            "named_shared": named_shared,
            "named_own": named_own,
            "learning": report,
            "extraction": shown_and_answer(&seen.lock().unwrap_or_else(|p| p.into_inner()), key.as_str()),
            "copy_confidence_after": pool.get(copy).map(|f| f.confidence).ok(),
            "copy_live_after": pool.get(copy).map(|f| f.is_live()).ok(),
        }));
    }
    let copy_live = pool.get(copy)?.is_live();
    let dropped_mark = pool
        .meta(&format!("pool-dropped:{}", POOLED.to_lowercase()))?
        .is_some();
    // The copy leaves exactly when it misled two tickets; then recall no longer finds it.
    let drop_right = (blamed >= 2) == (!copy_live && dropped_mark);
    let (project, _, mem, _, _, _) = &placed[0];
    let recalled_after = MemoryRetriever::new(mem.clone(), e.clone())
        .with_pool(Some(pool.clone()))
        .recall(
            "Add minutes/standup.txt, a team document naming its owning team on its first line",
            Some(&project.id.to_string()),
            8,
        )?
        .iter()
        .any(|r| r.shared && r.text == POOLED);
    // Only the pool changed: the lesson it was copied from and each agent's own lesson are as they were (a clean close
    // may raise an own lesson, never lower it here).
    let o = owner_mem.get(original)?;
    // An agent may refine its own lesson while it learns (an update supersedes it): followed to its live end. Retired,
    // or less confident, it was blamed.
    let own_after: Vec<f64> = placed
        .iter()
        .map(|(_, _, mem, own, _, _)| live_end_confidence(mem, *own))
        .collect::<Result<_, _>>()?;
    let untouched = o.is_live()
        && (o.confidence - 0.9).abs() < 1e-9
        && own_after.iter().all(|c| *c >= 0.7 - 1e-9);
    let passed = all_ok && drop_right && untouched && (copy_live || !recalled_after);
    let mut agent_runs = Vec::new();
    for (_, agent, _, _, _, _) in &placed {
        agent_runs.extend(app.store().read(|c| runs::for_agent(c, agent.id, 100))?);
    }
    Ok(TaskResult {
        id: format!("{ID}-loop"),
        passed,
        closed: true,
        detail: format!(
            "{} ticket(s), the pool lesson followed in {}, named in {blamed}; copy {}",
            steps.len(),
            steps
                .iter()
                .filter(|s| s["followed_the_pool_lesson"] == true)
                .count(),
            if copy_live { "kept" } else { "dropped" }
        ),
        runs: steps.len(),
        steps: 0,
        tool_errors: 0,
        error_codes: Default::default(),
        errors: vec![],
        tools: Default::default(),
        outcomes: steps
            .iter()
            .map(|s| s["work_outcome"].as_str().unwrap_or_default().to_owned())
            .collect(),
        reopened: steps.iter().filter(|s| s["resolved"] == true).count() as u32,
        tokens_in: agent_runs.iter().map(|r| r.usage.tokens_in).sum(),
        tokens_cached: agent_runs.iter().map(|r| r.usage.cache_read).sum(),
        tokens_out: agent_runs.iter().map(|r| r.usage.tokens_out).sum(),
        cost_usd: agent_runs.iter().map(|r| r.usage.estimated_cost_usd).sum(),
        elapsed_ms: start.elapsed().as_millis() as u64,
        record: None,
        learning: Some(json!({
            "embedder": e.model_id(),
            "tickets": steps,
            "copy_live": copy_live,
            "dropped_mark": dropped_mark,
            "drop_right": drop_right,
            "recalled_after": recalled_after,
            "original_and_own_untouched": untouched,
            "own_confidence_after": own_after,
        })),
        discipline: None,
    })
}
