//! What a task left on record (ADR-147, ADR-148), read before its root is dropped: the checks its runs made and how
//! many carry the worktree tree they ran on, what the last reports said beside their criteria, the handover written
//! when the project completed, and the project's status.

use agenium_core::comment::CommentBody;
use agenium_core::*;
use agenium_orchestrator::app::App;
use agenium_orchestrator::judge::done_check::is_check;
use agenium_store::core::{comments, runs};
use serde_json::{json, Value};

pub fn record(app: &App, project: ProjectId, tickets: &[TicketId]) -> anyhow::Result<Value> {
    let (mut checks, mut with_tree) = (0, 0);
    let mut reports = Vec::new();
    for &t in tickets {
        for r in app.store().read(|c| runs::for_ticket(c, t))? {
            for e in app.store().read(|c| runs::log_entries(c, r.id))? {
                let shell = e.kind == "tool" && e.data["tool"] == "shell";
                if shell && e.data["args"]["command"].as_str().is_some_and(is_check) {
                    checks += 1;
                    with_tree += usize::from(e.data["tree"].is_string());
                }
            }
        }
        let last = app
            .store()
            .read(|c| comments::for_ticket(c, t))?
            .into_iter()
            .rev()
            .find_map(|c| match c.body {
                CommentBody::Report(r) => Some(r),
                _ => None,
            });
        if let Some(r) = last {
            reports.push(json!({
                "ticket": r.ticket_id,
                "checks_not_run": r.checks_not_run,
                "beyond_ticket": r.beyond_ticket,
                "tests_changed": r.tests_changed,
            }));
        }
    }
    let p = app.project(project)?;
    let dir = app.layout().root.join("reports").join(&p.name);
    let handovers: Vec<String> = std::fs::read_dir(&dir)
        .map(|d| {
            d.filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    Ok(json!({
        "checks": checks,
        "checks_with_tree": with_tree,
        "reports": reports,
        "handovers": handovers,
        "project_status": format!("{:?}", p.status),
    }))
}

/// One line for the table.
pub fn line(v: &Value) -> String {
    let lists = |field: &str| {
        v["reports"]
            .as_array()
            .map(|rs| {
                rs.iter()
                    .filter_map(|r| r[field].as_array())
                    .map(Vec::len)
                    .sum::<usize>()
            })
            .unwrap_or(0)
    };
    let handovers: Vec<&str> = v["handovers"]
        .as_array()
        .map(|h| h.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    format!(
        "  record: checks {} ({} with tree), report lists: not run {}, beyond {}, tests {}; handover [{}]; project {}\n",
        v["checks"],
        v["checks_with_tree"],
        lists("checks_not_run"),
        lists("beyond_ticket"),
        lists("tests_changed"),
        handovers.join(", "),
        v["project_status"].as_str().unwrap_or_default()
    )
}

/// The app's loop, once, when the project is done: it writes the handover and completes the project (ADR-147), which
/// the eval's own rounds (`drive`, `tick`) never do.
pub async fn complete(
    app: &App,
    rt: &agenium_orchestrator::runtime::Runtime,
    project: ProjectId,
) -> anyhow::Result<Vec<agenium_orchestrator::runtime::RunSummary>> {
    if app.run_state(project)? == agenium_orchestrator::views::RunState::Done {
        return Ok(rt.work(project).await?);
    }
    Ok(Vec::new())
}
