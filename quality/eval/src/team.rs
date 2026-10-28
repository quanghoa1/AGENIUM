//! Team golden task (M4 acceptance): a request in Vietnamese to a coding team built from the template (Lead, planner,
//! backend, frontend). The harness plays the user: it approves the spec and the plan when they wait. Graded on the
//! result files and on the team behaviour: the planner drafted, the Lead created the plan, at least two tickets by
//! two different workers, one depending on the other, all closed and merged. And on the protocol (`discipline`):
//! workers work only on tickets assigned to them, only the Lead is chatted with or messaged while it works, every
//! closed ticket carries its worker's report with evidence for each acceptance criterion and an approving review
//! before the merge. Retries, nudges, delivered messages, compactions and refused calls are counted.
//! `--worker-cli <cli> --worker-model <model>`: the workers run on a vendor CLI, the Lead and planner on the API.

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

/// The message the harness sends while the team works: it must reach the Lead, and only the Lead.
const STEER: &str =
    "While you work: when you next write a ticket comment or answer, include the tag ACK-42.";

/// Protocol checks over the finished team run; returns the problems and the counts.
fn discipline(
    app: &App,
    project: ProjectId,
    summaries: &[RunSummary],
) -> anyhow::Result<(Vec<String>, serde_json::Value)> {
    let mut problems = vec![];
    let team = app.team_of_project(project)?;
    let lead = team.as_ref().and_then(|(t, _)| t.lead);
    let by_name: std::collections::HashMap<String, agenium_store::core::agents::Agent> = team
        .map(|(_, m)| m.into_iter().map(|a| (a.name.clone(), a)).collect())
        .unwrap_or_default();
    let tickets = app.tickets(project)?;
    let mut counts = std::collections::BTreeMap::<String, u32>::new();
    let mut steered_to = vec![];
    for s in summaries {
        let agent = by_name.get(&s.agent);
        let is_lead = agent.is_some_and(|a| Some(a.id) == lead);
        match s.kind.as_str() {
            "work" => match (&s.ticket, agent) {
                (Some(k), Some(a)) => {
                    let t = tickets.iter().find(|t| &t.key == k);
                    if t.and_then(|t| t.assignee) != Some(a.id) {
                        problems.push(format!(
                            "{} worked on {k}, which is not assigned to it",
                            s.agent
                        ));
                    }
                }
                _ => problems.push(format!("work run of {} without a ticket", s.agent)),
            },
            "chat" | "inbox" | "steer" if !is_lead => problems.push(format!(
                "{} run of {}, who is not the Lead",
                s.kind, s.agent
            )),
            "review" => {
                if let (Some(k), Some(a)) = (&s.ticket, agent) {
                    if tickets
                        .iter()
                        .any(|t| &t.key == k && t.assignee == Some(a.id))
                    {
                        problems.push(format!("{} reviewed its own ticket {k}", s.agent));
                    }
                }
            }
            _ => {}
        }
        for e in app.store().read(|c| runs::log_entries(c, s.run))? {
            let what = match e.kind.as_str() {
                "retry" => "retry".to_owned(),
                "control" => e.data["what"].as_str().unwrap_or("control").to_owned(),
                _ => continue,
            };
            if what == "steer" {
                steered_to.push((s.agent.clone(), is_lead));
            }
            *counts.entry(what).or_default() += 1;
        }
    }
    if let Some((who, _)) = steered_to.iter().find(|(_, lead)| !lead) {
        problems.push(format!(
            "the user's message reached {who}, who is not the Lead"
        ));
    }
    // Evidence on the tickets: the worker's report covers every criterion, and a review approved it before the merge.
    let mut reports = 0;
    for t in tickets.iter().filter(|t| t.status == TicketStatus::Closed) {
        let cs = app.store().read(|c| comments::for_ticket(c, t.id))?;
        let report = cs.iter().rev().find_map(|c| match (&c.body, &c.author) {
            (CommentBody::Report(r), Author::Agent(a)) if Some(*a) == t.assignee => Some(r),
            _ => None,
        });
        // The approving review's own word on a criterion (a reviewer may verify what the worker could not).
        let reviewed: std::collections::HashSet<String> = cs
            .iter()
            .rev()
            .find_map(|c| match &c.body {
                CommentBody::Review(r) if r.decision == forms::ReviewVerdict::Approve => Some(
                    r.ac_feedback
                        .iter()
                        .filter(|f| f.feedback.trim().len() >= 10)
                        .map(|f| f.ac_id.clone())
                        .collect(),
                ),
                _ => None,
            })
            .unwrap_or_default();
        match report {
            None => problems.push(format!("{} closed without its worker's report", t.key)),
            Some(r) => {
                reports += 1;
                for ac in &t.acceptance {
                    match r.acceptance.iter().find(|x| x.ac_id == ac.id) {
                        Some(x)
                            if x.status == forms::AcStatus::Met
                                && x.evidence.trim().len() >= 10 => {}
                        Some(_) if reviewed.contains(&ac.id) => {
                            *counts.entry("ac_verified_by_reviewer".into()).or_default() += 1;
                        }
                        Some(x) => problems.push(format!(
                            "{} {}: {:?} with evidence {:?}, approved without the reviewer's own word on it",
                            t.key, ac.id, x.status, x.evidence
                        )),
                        None => problems.push(format!("{} report misses {}", t.key, ac.id)),
                    }
                }
            }
        }
        if t.merged {
            let approved = cs.iter().rev().find_map(|c| match &c.body {
                CommentBody::Review(r) => Some(r.decision == forms::ReviewVerdict::Approve),
                _ => None,
            });
            if approved != Some(true) {
                problems.push(format!("{} merged without an approving review", t.key));
            }
        }
    }
    let acked = tickets.iter().any(|t| {
        t.close_reason
            .as_deref()
            .is_some_and(|r| r.contains("ACK-42"))
    }) || tickets.iter().any(|t| {
        app.store()
            .read(|c| comments::for_ticket(c, t.id))
            .map(|cs| {
                cs.iter()
                    .any(|c| serde_json::to_string(&c.body).is_ok_and(|b| b.contains("ACK-42")))
            })
            .unwrap_or(false)
    }) || summaries.iter().any(|s| s.text.contains("ACK-42"));
    Ok((
        problems,
        serde_json::json!({
            "counts": counts,
            "reports_with_evidence": reports,
            "message_reached_lead": steered_to.iter().any(|(_, l)| *l),
            "lead_acknowledged_message": acked,
        }),
    ))
}

pub async fn run(mode: &Mode, worker_cli: Option<(String, String)>) -> anyhow::Result<TaskResult> {
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
        effort: effort.clone(),
        max_tokens: None,
    };
    app.create_team_from_template(project.id, "coding", &binding, None, None, "t")?;
    // Mixed team: the workers that take tickets run on a vendor CLI; the Lead and the planner stay on the API.
    if let Some((cli, cli_model)) = &worker_cli {
        let lead = app.team_of_project(project.id)?.and_then(|(t, _)| t.lead);
        for a in app
            .team_of_project(project.id)?
            .map(|(_, m)| m)
            .unwrap_or_default()
        {
            if Some(a.id) == lead || app.is_planner_role(&a.role) {
                continue;
            }
            app.set_agent_backend(
                a.id,
                BackendKind::Cli,
                &ModelBinding {
                    provider: cli.clone(),
                    model: cli_model.clone(),
                    effort: effort.clone(),
                    max_tokens: None,
                },
            )?;
        }
    }
    let approver: Arc<dyn agenium_orchestrator::runtime::Approver> = if *approve_commands {
        Arc::new(AutoApprove)
    } else {
        Arc::new(DenyApprover)
    };
    crate::price_as_eval(&app, provider.as_ref())?;
    let rt = Arc::new(Runtime::new(
        app.clone(),
        Arc::new(StaticProviders(vec![("eval".into(), provider.clone())])),
        approver,
    ));

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
            let mut steered = false;
            // Scheduler rounds; the harness approves a plan version as soon as it waits for the user.
            for _ in 0..24 {
                if let Some(p) = app.store().read(|c| plans::latest_plan(c, project.id))? {
                    if p.approved_by.is_none() {
                        app.approve_plan(project.id, p.version)?;
                    }
                }
                play_user(&app, project.id)?;
                // Once tickets exist, the user writes to the team while a round runs: delivered to the running Lead
                // (never to a worker), or kept for the Lead's next turn.
                let watcher = (!app.tickets(project.id)?.is_empty() && !steered).then(|| {
                    let rt = rt.clone();
                    let pid = project.id;
                    tokio::spawn(async move {
                        for _ in 0..600 {
                            if rt.steer(pid, STEER).unwrap_or(false) {
                                return true;
                            }
                            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                        }
                        false
                    })
                });
                let r = rt.tick(project.id).await?;
                if let Some(w) = watcher {
                    w.abort();
                    steered |= w.await.unwrap_or(false);
                }
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
    // Every ticket ends Closed (merged when it has code), or Cancelled by the Lead with a reason (a superseded or
    // duplicate ticket); at least the two workers' tickets close.
    let finished = |t: &Ticket| {
        (t.status == TicketStatus::Closed && (t.merged || !t.has_code))
            || (t.status == TicketStatus::Cancelled
                && t.close_reason
                    .as_deref()
                    .is_some_and(|r| r.trim().len() >= 5))
    };
    let closed_by: std::collections::HashSet<AgentId> = tickets
        .iter()
        .filter(|t| t.status == TicketStatus::Closed)
        .filter_map(|t| t.assignee)
        .collect();
    let all_closed = !tickets.is_empty() && tickets.iter().all(finished) && closed_by.len() >= 2;
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
    let (rules, mut discipline) = discipline(&app, project.id, &summaries)?;
    problems.extend(rules);
    discipline["tickets"] = serde_json::json!(tickets
        .iter()
        .map(|t| serde_json::json!({
            "key": t.key.to_string(), "title": t.title, "status": t.status.to_string(),
            "assignee": t.assignee.map(|a| a.to_string()), "depends_on": t.depends_on,
            "merged": t.merged, "reopened": t.reopen_count, "close_reason": t.close_reason,
        }))
        .collect::<Vec<_>>());

    let (mut tool_errors, mut error_codes, mut errors) =
        (0u32, std::collections::BTreeMap::new(), vec![]);
    let mut tools = std::collections::BTreeMap::new();
    for s in &summaries {
        for e in app.store().read(|c| runs::log_entries(c, s.run))? {
            if e.kind == "tool" {
                *tools
                    .entry(format!(
                        "{}/{}",
                        s.kind,
                        e.data["tool"].as_str().unwrap_or_default()
                    ))
                    .or_insert(0) += 1;
            }
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
        tools,
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
        tokens_cached: usage.cache_read,
        tokens_out: usage.tokens_out,
        cost_usd: usage.estimated_cost_usd,
        elapsed_ms: start.elapsed().as_millis() as u64,
        learning: None,
        discipline: Some(discipline),
    })
}
