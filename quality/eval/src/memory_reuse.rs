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
//!
//! `--chain` (memory on) continues the same agent with tickets 3 and 4 in two more folders the rule covers
//! (`handbook/`, `minutes/`): the whole learning chain. A lesson shown to a cleanly closed ticket holds in that
//! context; after two contexts the skill-writing tools open (ADR-91), and ticket 4 shows whether the agent turns the
//! proven lesson into a skill update or a new skill, and whether the gates accept it. Recorded under `chain`.

use std::path::Path;
use std::sync::{Arc, Mutex};
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

/// The folders whose files are team documents (the chain adds `handbook/` and `minutes/`).
const TEAM_DIRS: [&str; 2] = ["deliverables", "reports"];
const CHAIN_DIRS: [&str; 4] = ["deliverables", "reports", "handbook", "minutes"];

/// The project's pre-merge check: every team document names its owning team on its first line. `seen` keeps, for the
/// result file only, each rejected first line as written (a UTF-8 BOM or a joined line shows there); the agent sees the
/// check's log alone.
struct OwnerRule {
    seen: Arc<Mutex<Vec<String>>>,
    dirs: &'static [&'static str],
}
impl CheckRunner for OwnerRule {
    fn run(&self, worktree: &Path) -> CheckOutcome {
        let mut bad = Vec::new();
        let wt = worktree.file_name().unwrap_or_default().to_string_lossy();
        for dir in self.dirs {
            let Ok(rd) = std::fs::read_dir(worktree.join(dir)) else {
                continue;
            };
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') || !e.path().is_file() {
                    continue;
                }
                let content = std::fs::read_to_string(e.path()).unwrap_or_default();
                let line = content.lines().next().unwrap_or_default();
                if line.trim() != OWNER_LINE {
                    let shown: String = line.chars().take(80).collect();
                    self.seen
                        .lock()
                        .unwrap()
                        .push(format!("{wt}: {dir}/{name}: {shown:?}"));
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

struct OwnerRuleFactory(Arc<Mutex<Vec<String>>>, &'static [&'static str]);
impl CheckFactory for OwnerRuleFactory {
    fn for_project(
        &self,
        _: &agenium_store::core::projects::Project,
        _: &agenium_orchestrator::manifest::ProjectManifest,
    ) -> Arc<dyn CheckRunner> {
        Arc::new(OwnerRule {
            seen: self.0.clone(),
            dirs: self.1,
        })
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

/// The agent's live memories: where each came from and where it holds (a scope the recall context conflicts with
/// hides it, and so do the untrusted mark and a weight below the hiding threshold).
fn held(mem: &agenium_store::AgentStore) -> anyhow::Result<Vec<String>> {
    Ok(mem
        .live(200)?
        .into_iter()
        .map(|f| {
            let untrusted = if f.untrusted { " untrusted" } else { "" };
            format!(
                "[{:?}{untrusted} w={:.2}] ({} {}) {}",
                f.kind, f.weight, f.source, f.scope, f.text
            )
        })
        .collect())
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
        "tokens_cached": t.usage.cache_read,
        "tokens_out": t.usage.tokens_out,
        "cost_usd": t.usage.estimated_cost_usd,
        "files_after": files(root),
        "tickets_in_project": app.store().read(|c| agenium_store::core::tickets::list(c, project, &Default::default()))?.len(),
    }))
}

/// The agent's lessons with the contexts each held in (`text` shortened).
fn lessons(mem: &agenium_store::AgentStore) -> anyhow::Result<Vec<Value>> {
    Ok(mem
        .live(500)?
        .into_iter()
        .filter(|f| f.kind == agenium_store::FactKind::Lesson)
        .map(|f| {
            json!({ "id": f.id, "held_in": f.validated, "untrusted": f.untrusted,
                    "text": f.text.chars().take(160).collect::<String>() })
        })
        .collect())
}

/// The skill-writing calls of a ticket's runs: tool, accepted or refused, and the answer.
fn skill_calls(app: &App, project: ProjectId, key: &TicketKey) -> anyhow::Result<Vec<Value>> {
    let t = app.ticket(project, key)?;
    let mut out = vec![];
    for r in app.store().read(|c| runs::for_ticket(c, t.id))? {
        for e in app.store().read(|c| runs::log_entries(c, r.id))? {
            let tool = e.data["tool"].as_str().unwrap_or_default();
            if e.kind == "tool" && matches!(tool, "propose_skill_update" | "propose_new_skill") {
                out.push(json!({ "tool": tool, "accepted": e.data["ok"], "args": e.data["args"],
                                 "answer": e.data["result"].as_str().unwrap_or_default().chars().take(400).collect::<String>() }));
            }
        }
    }
    Ok(out)
}

pub async fn run(
    mode: &Mode,
    memory_on: bool,
    extract: Option<(Arc<dyn Provider>, String)>,
    models: Option<&Path>,
    chain: bool,
    skills: &[String],
) -> anyhow::Result<TaskResult> {
    let start = Instant::now();
    let tmp = tempfile::tempdir()?;
    let rejected = Arc::new(Mutex::new(Vec::new()));
    let (app, _) = App::open(
        &tmp.path().join("root"),
        AppOptions {
            keyring: Arc::new(agenium_secrets::MemoryKeyring::default()),
            checks: Arc::new(OwnerRuleFactory(
                rejected.clone(),
                if chain { &CHAIN_DIRS } else { &TEAM_DIRS },
            )),
        },
    )?;
    // The local embedding model, as `agenium models download` installs it; without it memory uses the hashing embedder.
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
        if chain {
            "# Docs\n\nTeam documents live in deliverables/, reports/, handbook/ and minutes/.\n"
        } else {
            "# Docs\n\nTeam documents live in deliverables/ and reports/.\n"
        },
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
                effort: effort.clone(),
                max_tokens: None,
            },
        },
    )?;
    // `--skill`: library skills the agent has, whose own copy it may improve from what it learns (ADR-146); the
    // system's files are read before and after to show they are never changed.
    let library_before: Vec<(String, String)> = skills
        .iter()
        .map(|s| {
            app.toggle_agent_skill(agent.id, s, true)?;
            Ok((s.clone(), app.library_skill_body(s).unwrap_or_default()))
        })
        .collect::<anyhow::Result<_>>()?;
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
    crate::price_as_eval(&app, providers[0].1.as_ref())?;
    let rt = Runtime::new(app.clone(), Arc::new(StaticProviders(providers)), approver);

    let memory = || {
        let e = app.embedder();
        app.agent_store(agent.id, e.dim(), e.model_id())
    };
    let mut tickets = Vec::new();
    let mut before_t2 = Vec::new();
    let mut chain_steps = Vec::new();
    let all: [(&str, &str, &str, &str); 4] = [
        ("PI-1", "deliverables/hello.txt", "hello", "Hello note"),
        ("PI-2", "reports/weekly.txt", "weekly", "Weekly report"),
        (
            "PI-3",
            "handbook/onboarding.txt",
            "onboarding",
            "Onboarding page",
        ),
        ("PI-4", "minutes/standup.txt", "standup", "Standup minutes"),
    ];
    let count = if chain { 4 } else { 2 };
    for (n, (item, file, word, _)) in all.iter().copied().enumerate().take(count) {
        // The weekly report enters the plan only now: with it in plan v1 the Lead ticketed it on its own after ticket 1
        // (as reports/weekly.md), so the measured ticket was its third and its first merge could copy a reports/ file
        // already on main (the 2026-10-25 v2 runs).
        if n > 0 {
            if item == "PI-2" {
                // What recall could draw on: ticket 2's own learning may rewrite these texts afterwards.
                before_t2 = held(memory()?.as_ref())?;
            }
            app.apply(
                &user,
                Intent::RevisePlan(PlanRevision {
                    base_version: n as u32,
                    reason: format!("Add {file}"),
                    milestones: milestones(),
                    items: all[..=n]
                        .iter()
                        .map(|(i, f, w, title)| plan_item(i, title, &ac(f, w)))
                        .collect(),
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
        rt.drive(project.id, 12).await?;
        tickets.push(measure(&app, project.id, &key, file, word, &root)?);
        if chain {
            // What the memory pipeline decided after this ticket (added / updated / deleted, what it refused).
            // And what recall put in this ticket's work orders.
            let t = app.ticket(project.id, &key)?;
            let mem = memory()?;
            let mut memory_ops = Vec::new();
            let mut shown = Vec::new();
            let mut run_list = Vec::new();
            for r in app.store().read(|c| runs::for_ticket(c, t.id))? {
                run_list.push(json!({
                    "run": r.id.to_string(),
                    "agent": r.agent.to_string(),
                    "outcome": r.outcome,
                    "steps": r.steps,
                    "shown": mem.usage_for_run(&r.id.to_string())?.len(),
                }));
                for e in app.store().read(|c| runs::log_entries(c, r.id))? {
                    if e.kind == "memory" {
                        memory_ops.push(e.data);
                    }
                }
                for (fact, rank, via) in mem.usage_for_run(&r.id.to_string())? {
                    let f = mem.get(fact).ok();
                    shown.push(format!(
                        "run {} #{rank} {via} {:?}: {}",
                        r.id,
                        f.as_ref().map(|f| f.kind),
                        f.map(|f| f.text).unwrap_or_default()
                    ));
                }
            }
            chain_steps.push(json!({
                "ticket": key.to_string(),
                "skill_calls": skill_calls(&app, project.id, &key)?,
                "lessons_after": lessons(memory()?.as_ref())?,
                "memory_ops": memory_ops,
                "shown": shown,
                "runs": run_list,
                "memories_after": held(memory()?.as_ref())?,
            }));
        }
    }

    // What memory held after ticket 1, and what recall showed ticket 2.
    let mem = memory()?;
    let lessons = held(&mem)?;
    let t2 = app.ticket(
        project.id,
        &tickets[1]["ticket"].as_str().unwrap_or("AGN-2").parse()?,
    )?;
    let mut shown = 0usize;
    let mut shown_texts: Vec<String> = Vec::new();
    for r in app.store().read(|c| runs::for_ticket(c, t2.id))? {
        for (fact, rank, via) in mem.usage_for_run(&r.id.to_string())? {
            shown += 1;
            let text = mem.get(fact).map(|f| f.text).unwrap_or_default();
            shown_texts.push(format!("run {} #{rank} {via}: {text}", r.id));
        }
    }

    let t2m = &tickets[1];
    let library_unchanged = library_before
        .iter()
        .all(|(s, before)| app.library_skill_body(s).unwrap_or_default() == *before);
    let closed = tickets.iter().all(|t| t["closed"] == true);
    Ok(TaskResult {
        id: format!("{ID}-{}", if memory_on { "on" } else { "off" }),
        passed: verdict(&tickets, library_unchanged),
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
        tools: Default::default(),
        outcomes: vec![],
        reopened: tickets.iter().map(|t| t["reopened"].as_u64().unwrap_or(0) as u32).sum(),
        tokens_in: tickets.iter().map(|t| t["tokens_in"].as_u64().unwrap_or(0)).sum(),
        tokens_cached: tickets.iter().map(|t| t["tokens_cached"].as_u64().unwrap_or(0)).sum(),
        tokens_out: tickets.iter().map(|t| t["tokens_out"].as_u64().unwrap_or(0)).sum(),
        cost_usd: tickets.iter().map(|t| t["cost_usd"].as_f64().unwrap_or(0.0)).sum(),
        elapsed_ms: start.elapsed().as_millis() as u64,
        record: None,
        learning: Some(json!({
            "memory": memory_on,
            "embedder": app.embedder().model_id(),
            "ticket1": tickets[0],
            "ticket2": tickets[1],
            "memories_before_ticket2": before_t2,
            "memories_after": lessons,
            "ticket2_facts_shown": shown,
            "ticket2_shown": shown_texts,
            "check_rejected": rejected.lock().unwrap().clone(),
            "chain": chain.then(|| json!({
                "steps": chain_steps,
                "tickets": tickets.iter().skip(2).cloned().collect::<Vec<_>>(),
                "agent_skills": app.agent(agent.id).ok().map(|a| rt.skills(&a).skills.iter()
                    .map(|k| format!("{} ({:?})", k.meta.name, k.meta.origin)).collect::<Vec<_>>()),
                "overlays": app.skill_overlays(agent.id).ok().map(|o| o.iter().map(|v| json!({
                    "skill": v.skill, "base": v.base, "diff": v.diff, "changelog": v.changelog,
                })).collect::<Vec<_>>()),
                "library_unchanged": library_unchanged,
            })),
        })),
        discipline: None,
    })
}

/// The scenario passes only when every ticket it ran (both of the pair, and every ticket of a chain) closed with
/// correct work, and no library skill changed: a later ticket of a chain that fails, or a lesson written into the
/// system's skill instead of the agent's own copy, fails it.
fn verdict(tickets: &[Value], library_unchanged: bool) -> bool {
    !tickets.is_empty()
        && tickets
            .iter()
            .all(|t| t["closed"] == true && t["correct"] == true)
        && library_unchanged
}

#[cfg(test)]
mod tests {
    use super::verdict;
    use serde_json::json;

    #[test]
    fn a_chain_fails_on_any_ticket_or_a_changed_library_skill() {
        let ok = json!({ "closed": true, "correct": true });
        let wrong = json!({ "closed": true, "correct": false });
        let open = json!({ "closed": false, "correct": true });
        assert!(verdict(&[ok.clone(), ok.clone()], true));
        assert!(verdict(
            &[ok.clone(), ok.clone(), ok.clone(), ok.clone()],
            true
        ));
        // Ticket 3 of a chain is wrong, or ticket 4 never closed: red, though tickets 1 and 2 passed.
        assert!(!verdict(&[ok.clone(), ok.clone(), wrong, ok.clone()], true));
        assert!(!verdict(&[ok.clone(), ok.clone(), ok.clone(), open], true));
        // The base skill changed: red.
        assert!(!verdict(&[ok.clone(), ok.clone(), ok.clone()], false));
        assert!(!verdict(&[], true));
    }
}
