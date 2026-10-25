//! Team golden task (M4 acceptance): a request in Vietnamese to a coding team built from the template (Lead, planner,
//! backend, frontend). The harness plays the user: it approves the spec and the plan when they wait. Graded on the
//! result files and on the team behaviour: the planner drafted, the Lead created the plan, at least two tickets by
//! two different workers, one depending on the other, all closed and merged.

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use agenium_core::*;
use agenium_orchestrator::app::{App, AppOptions, CreateProject, NoCheckFactory};
use agenium_orchestrator::runtime::{
    AutoApprove, DenyApprover, RunSummary, Runtime, StaticProviders,
};
use agenium_store::core::{comments, events, plans, runs};

use crate::{Mode, TaskResult};

pub const ID: &str = "team-greeting";

const REQUEST: &str = "Tôi cần một trang chào hỏi nhỏ, làm thành hai phần cho hai người trong team: \
(1) backend: tạo file api/greeting.json chứa đúng JSON {\"greeting\": \"xin chào\"}; \
(2) frontend: tạo file web/index.html hiển thị lời chào, lấy nó từ api/greeting.json bằng fetch. \
Phần frontend chỉ làm sau khi phần backend xong. Yêu cầu đã đủ rõ, không cần hỏi lại: hãy đề xuất spec.";

fn check(root: &Path) -> Result<(), String> {
    let api = std::fs::read_to_string(root.join("api/greeting.json"))
        .map_err(|e| format!("api/greeting.json: {e}"))?;
    let v: serde_json::Value =
        serde_json::from_str(&api).map_err(|e| format!("api/greeting.json is not JSON: {e}"))?;
    if v["greeting"] != "xin chào" {
        return Err(format!(
            "api/greeting.json is {}",
            api.chars().take(200).collect::<String>()
        ));
    }
    let page = std::fs::read_to_string(root.join("web/index.html"))
        .map_err(|e| format!("web/index.html: {e}"))?;
    if !page.contains("greeting.json") || !page.contains("fetch") {
        return Err("web/index.html does not fetch api/greeting.json".into());
    }
    Ok(())
}

/// The harness as the user for what reaches the user during the work: permission requests outside the allowlist
/// are denied (work without them) and questions waiting for the user get "decide within the spec".
fn play_user(app: &App, project: ProjectId) -> anyhow::Result<()> {
    for t in app.tickets(project)? {
        for c in app.store().read(|c| comments::for_ticket(c, t.id))? {
            match &c.body {
                CommentBody::Permission { outcome: None, .. } => {
                    app.decide_permission(
                        project,
                        &t.key,
                        c.id,
                        agenium_core::forms::PermissionDecision::Deny,
                        false,
                        Some("Not granted in this evaluation: finish the work without it.".into()),
                    )?;
                }
                CommentBody::Question(_)
                    if !c.resolved
                        && app
                            .lead_inbox(project)?
                            .iter()
                            .all(|i| i.comment != Some(c.id)) =>
                {
                    app.user_answer(
                        project,
                        &t.key,
                        c.id,
                        "Decide yourself within the approved spec.",
                        None,
                    )?;
                }
                _ => {}
            }
        }
    }
    Ok(())
}

pub async fn run(mode: &Mode) -> anyhow::Result<TaskResult> {
    let start = Instant::now();
    let Mode::Model {
        provider,
        model,
        effort,
        approve_commands,
    } = mode
    else {
        anyhow::bail!("the team task needs a real model (--provider/--model)");
    };
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
        name: ID.into(),
        path: None,
        key_prefix: None,
        template: None,
        folder: None,
        repo: None,
    })?;
    let root = Path::new(&project.path).to_path_buf();
    std::fs::write(
        root.join("README.md"),
        "# Greeting\nA tiny greeting page.\n",
    )?;
    app.repo(&project)
        .commit_all(&root, "Eval fixture", ("eval", "eval@agenium.local"), &[])?;
    let binding = ModelBinding {
        provider: "eval".into(),
        model: model.clone(),
        effort: *effort,
        max_tokens: None,
    };
    app.create_team_from_template(project.id, "coding", &binding, None, None, "t")?;
    let approver: Arc<dyn agenium_orchestrator::runtime::Approver> = if *approve_commands {
        Arc::new(AutoApprove)
    } else {
        Arc::new(DenyApprover)
    };
    let rt = Runtime::new(
        app.clone(),
        Arc::new(StaticProviders(vec![("eval".into(), provider.clone())])),
        approver,
    );

    let mut summaries: Vec<RunSummary> = Vec::new();
    // Discovery: the Lead proposes the spec (asked again once if it only talks).
    for msg in [REQUEST, "Hãy đề xuất spec ngay bây giờ (propose_spec)."] {
        summaries.push(rt.chat(project.id, msg).await?);
        if app
            .store()
            .read(|c| plans::latest_spec(c, project.id))?
            .is_some()
        {
            break;
        }
    }
    let mut detail = String::new();
    let spec = app.store().read(|c| plans::latest_spec(c, project.id))?;
    match spec {
        None => detail = "the Lead did not propose a spec".into(),
        Some(s) => {
            app.approve_spec(project.id, s.version)?;
            // Scheduler rounds; the harness approves a plan version as soon as it waits for the user.
            for _ in 0..24 {
                if let Some(p) = app.store().read(|c| plans::latest_plan(c, project.id))? {
                    if p.approved_by.is_none() {
                        app.approve_plan(project.id, p.version)?;
                    }
                }
                play_user(&app, project.id)?;
                let r = rt.tick(project.id).await?;
                let idle = r.is_empty();
                summaries.extend(r);
                if idle {
                    break;
                }
            }
        }
    }

    // Grading.
    let tickets = app.tickets(project.id)?;
    let drafted = !app
        .store()
        .read(|c| events::recent_of_kind(c, "PlanDrafted", 10))?
        .is_empty();
    let plan = app.store().read(|c| plans::current_plan(c, project.id))?;
    let workers: std::collections::HashSet<AgentId> =
        tickets.iter().filter_map(|t| t.assignee).collect();
    let all_closed = !tickets.is_empty()
        && tickets
            .iter()
            .all(|t| t.status == TicketStatus::Closed && (t.merged || !t.has_code));
    let mut problems = vec![];
    if !detail.is_empty() {
        problems.push(detail.clone());
    }
    if !drafted {
        problems.push("the planner did not draft".into());
    }
    if plan
        .as_ref()
        .is_none_or(|p| !p.proposed_by.starts_with("agent:"))
    {
        problems.push("no plan created by the Lead".into());
    }
    if tickets.len() < 2 || workers.len() < 2 {
        problems.push(format!(
            "{} ticket(s) by {} worker(s)",
            tickets.len(),
            workers.len()
        ));
    }
    if !tickets.iter().any(|t| !t.depends_on.is_empty()) {
        problems.push("no ticket depends on another".into());
    }
    if !all_closed {
        problems.push(
            tickets
                .iter()
                .map(|t| format!("{}:{}", t.key, t.status))
                .collect::<Vec<_>>()
                .join(" "),
        );
    }
    if let Err(e) = check(&root) {
        problems.push(e);
    }

    let (mut tool_errors, mut error_codes, mut errors) =
        (0u32, std::collections::BTreeMap::new(), vec![]);
    for s in &summaries {
        for e in app.store().read(|c| runs::log_entries(c, s.run))? {
            if e.kind == "tool" && e.data["ok"] == false {
                tool_errors += 1;
                let res = e.data["result"].as_str().unwrap_or_default();
                *error_codes
                    .entry(res.split(':').next().unwrap_or_default().to_owned())
                    .or_insert(0) += 1;
                errors.push(
                    format!(
                        "{}/{}: {res}",
                        s.kind,
                        e.data["tool"].as_str().unwrap_or_default()
                    )
                    .chars()
                    .take(300)
                    .collect(),
                );
            }
        }
    }
    let usage = summaries.iter().fold(UsageSummary::default(), |mut u, s| {
        u.add(&s.usage);
        u
    });
    Ok(TaskResult {
        id: ID.into(),
        passed: problems.is_empty(),
        closed: all_closed,
        detail: if problems.is_empty() {
            "ok".into()
        } else {
            problems.join("; ")
        },
        runs: summaries.len(),
        steps: summaries.iter().map(|s| s.steps).sum(),
        tool_errors,
        error_codes,
        errors,
        outcomes: summaries
            .iter()
            .map(|s| {
                format!(
                    "{}:{}:{}:{}",
                    s.kind,
                    s.agent,
                    s.ticket.as_ref().map(|k| k.to_string()).unwrap_or_default(),
                    s.outcome
                )
            })
            .collect(),
        reopened: tickets.iter().map(|t| t.reopen_count).sum(),
        tokens_in: usage.tokens_in,
        tokens_out: usage.tokens_out,
        cost_usd: usage.estimated_cost_usd,
        elapsed_ms: start.elapsed().as_millis() as u64,
        learning: None,
    })
}
