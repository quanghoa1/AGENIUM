//! When runs cannot go on: quota, credit, repeated failures, pauses, checkpoints.

use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn quota_exhaustion_parks_the_agent_and_repeated_failures_escalate() {
    let f = fixture();
    let p = Arc::new(ScriptedProvider::new("mock"));
    let (app, rt) = runtime(&f, p.clone(), Arc::new(AutoApprove));
    let (project, agent) = solo(&app);
    let lead = Actor::Lead(agent);
    let ctxp = |a: Actor| agenium_orchestrator::CallCtx {
        project,
        actor: a,
        intent_id: None,
        run: None,
    };
    app.apply(&ctxp(lead), Intent::ProposeSpec(spec())).unwrap();
    app.apply(&ctxp(lead), Intent::CreatePlan(plan(&[("PI-1", &[])])))
        .unwrap();
    app.apply(
        &ctxp(lead),
        Intent::CreateTicket(new_ticket("PI-1", "t", &[], &[])),
    )
    .unwrap();
    app.apply(
        &ctxp(lead),
        Intent::AssignTicket(agenium_core::forms::AssignTicket {
            ticket_id: key("AGN-1"),
            worker: agent,
            note: None,
        }),
    )
    .unwrap();

    p.push_error(agenium_llm::LlmError::Http {
        status: 429,
        kind: "insufficient_quota".into(),
        message: "quota".into(),
        quota: true,
        retry_after: None,
    });
    let s = rt.run_ticket(project, &key("AGN-1")).await.unwrap();
    assert_eq!(s.outcome, "QuotaExhausted");
    assert_eq!(
        app.agent(agent).unwrap().status,
        AgentStatus::QuotaExhausted
    );
    assert_eq!(
        app.ticket(project, &key("AGN-1")).unwrap().status,
        TicketStatus::InProgress,
        "the ticket keeps its state"
    );
    assert!(
        app.ready_tickets(project).unwrap().is_empty(),
        "no new work for a QuotaExhausted agent"
    );

    // The user switches the model / adds credit: the agent is Idle again and the ticket continues; two stopped runs
    // in a row stop auto-restarts and ask the user.
    app.set_agent_status(agent, AgentStatus::Idle).unwrap();
    for _ in 0..2 {
        for _ in 0..3 {
            p.push_text("thinking out loud");
        }
        let s = rt.run_ticket(project, &key("AGN-1")).await.unwrap();
        assert_eq!(s.outcome, "Stopped:no_tool_call");
    }
    assert!(
        rt.tick(project).await.unwrap().is_empty(),
        "auto-restart stops after repeated failures (the stall-nudge waits stall_minutes)"
    );
    let stopping = |app: &agenium_orchestrator::App| {
        app.store()
            .read(|c| events::recent_of_kind(c, "NeedsUser", 10))
            .unwrap()
            .iter()
            .filter(|e| {
                e.payload["reason"]
                    .as_str()
                    .unwrap_or_default()
                    .contains("runs keep stopping")
            })
            .count()
    };
    assert_eq!(stopping(&app), 0, "not escalated before the nudges");

    // Stall-nudge (decision 28): after stall_minutes one more run with a System nudge, then the user.
    app.update_config(|c| {
        c.runtime.stall_minutes = 0;
        c.runtime.stall_nudges = 1;
    })
    .unwrap();
    p.push(Box::new(|req| {
        let all = serde_json::to_string(&req.messages).unwrap();
        assert!(
            all.contains("Stall nudge 1/1"),
            "the nudge reaches the agent"
        );
        Ok(calls_response("still thinking", vec![]))
    }));
    p.push_text("thinking out loud");
    p.push_text("thinking out loud");
    let runs = rt.tick(project).await.unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].outcome, "Stopped:no_tool_call");
    assert!(rt.tick(project).await.unwrap().is_empty(), "nudges used up");
    assert!(rt.tick(project).await.unwrap().is_empty());
    assert_eq!(stopping(&app), 1, "escalated to the user once");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chat_with_exhausted_credit_marks_the_agent_and_explains() {
    let f = fixture();
    let p = Arc::new(ScriptedProvider::new("mock"));
    let (app, rt) = runtime(&f, p.clone(), Arc::new(AutoApprove));
    let (project, agent) = solo(&app);
    // The real OpenCode Zen answer of an unfunded account (2026-10-23).
    p.push_error(agenium_llm::LlmError::Http {
        status: 402,
        kind: "server_error".into(),
        message: "Upstream request failed: Insufficient account funds".into(),
        quota: true,
        retry_after: None,
    });
    let r = rt.chat(project, "hello").await.unwrap();
    assert_eq!(r.outcome, "QuotaExhausted");
    assert!(r.text.contains("Insufficient account funds"), "{}", r.text);
    assert_eq!(
        app.agent(agent).unwrap().status,
        AgentStatus::QuotaExhausted
    );
}

/// A run that fails between its start and its outcome is ended with an Error, and a ticket without its worktree
/// starts no run: neither leaves a run open, the ticket InProgress or the agent Working until a restart.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_run_that_cannot_go_on_leaves_nothing_open() {
    use agenium_store::core::workspaces;
    let f = fixture();
    let p = Arc::new(ScriptedProvider::new("mock"));
    let (app, rt) = runtime(&f, p.clone(), Arc::new(AutoApprove));
    let (project, agent) = solo_with_tickets(&app, &["PI-1"]);
    let open = |app: &agenium_orchestrator::App| app.store().read(runs::unfinished).unwrap();

    // The worktree's git operation has not succeeded yet: no run starts.
    let t = app.ticket(project, &key("AGN-1")).unwrap();
    let ws = app
        .store()
        .read(|c| workspaces::get(c, t.id))
        .unwrap()
        .unwrap();
    let set_removed = |removed: bool| {
        app.store()
            .commit(None, "test", |w| {
                workspaces::upsert(
                    w.tx,
                    &workspaces::TicketWorkspace {
                        removed,
                        ..ws.clone()
                    },
                )
            })
            .unwrap();
    };
    set_removed(true);
    let err = rt.run_ticket(project, &key("AGN-1")).await.unwrap_err();
    assert!(err.to_string().contains("has no worktree"), "{err}");
    assert!(open(&app).is_empty(), "no run was left open");
    assert_eq!(
        app.ticket(project, &key("AGN-1")).unwrap().status,
        TicketStatus::Assigned
    );
    assert_eq!(app.agent(agent).unwrap().status, AgentStatus::Idle);
    set_removed(false);

    // A run that fails after its start (here: its CLI cannot run) is ended with the error.
    app.set_agent_backend(agent, BackendKind::Cli, &no_cli(&rt))
        .unwrap();
    for err in [
        rt.run_ticket(project, &key("AGN-1")).await.unwrap_err(),
        rt.chat(project, "hello").await.unwrap_err(),
    ] {
        assert!(err.to_string().contains("devin is not installed"), "{err}");
        assert!(open(&app).is_empty(), "the failed run was ended");
        // ADR-132: set aside until the user fixes the CLI or gives it another model.
        assert_eq!(app.agent(agent).unwrap().status, AgentStatus::Offline);
    }
    let rs = app.store().read(|c| runs::for_ticket(c, t.id)).unwrap();
    assert_eq!(rs.len(), 1);
    assert!(
        rs[0]
            .outcome
            .as_deref()
            .is_some_and(|o| o.starts_with("Error:")),
        "{:?}",
        rs[0].outcome
    );
    assert_eq!(p.requests().len(), 0);
}

/// ADR-132: a Lead inbox or planner run whose CLI cannot run sets the agent aside and tells the user at once, instead
/// of a failing run every round (it failed two rounds, then asked).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_lead_or_planner_whose_cli_cannot_run_waits_for_the_user() {
    let f = fixture();
    let p = Arc::new(ScriptedProvider::new("mock"));
    let (app, rt) = runtime(&f, p.clone(), Arc::new(AutoApprove));
    app.update_config(|c| c.memory.learn = false).unwrap();
    let (project, lead, _) = team_project(&app, "duo");
    let as_lead = agenium_orchestrator::CallCtx {
        project,
        actor: Actor::Lead(lead),
        intent_id: None,
        run: None,
    };
    app.apply(&as_lead, Intent::ProposeSpec(spec())).unwrap();
    app.approve_spec(project, 1).unwrap();
    app.apply(&as_lead, Intent::CreatePlan(plan(&[("PI-1", &[])])))
        .unwrap();
    assert!(!app.lead_inbox(project).unwrap().is_empty());
    app.set_agent_backend(lead, BackendKind::Cli, &no_cli(&rt))
        .unwrap();
    for _ in 0..4 {
        assert!(rt.tick(project).await.unwrap().is_empty());
    }
    let lead_runs = app.store().read(|c| runs::for_agent(c, lead, 50)).unwrap();
    assert_eq!(lead_runs.len(), 1, "one failed inbox run, then none");
    assert!(lead_runs.iter().all(|r| r.ended_at.is_some()));
    assert_eq!(app.agent(lead).unwrap().status, AgentStatus::Offline);
    assert_eq!(needs_user(&app, "cannot use devin"), 1);

    // The planner (coding template) the same way.
    let project = app
        .create_project(&agenium_orchestrator::CreateProject {
            name: "planned".into(),
            path: None,
            key_prefix: None,
            template: None,
            folder: None,
            repo: None,
        })
        .unwrap()
        .id;
    app.create_team_from_template(project, "coding", &model(), None, None, "pl")
        .unwrap();
    let (team, members) = app.team_of_project(project).unwrap().unwrap();
    let planner = members.iter().find(|m| m.name == "pl-planner").unwrap().id;
    app.set_agent_backend(planner, BackendKind::Cli, &no_cli(&rt))
        .unwrap();
    let as_lead = agenium_orchestrator::CallCtx {
        project,
        actor: Actor::Lead(team.lead.unwrap()),
        intent_id: None,
        run: None,
    };
    app.apply(&as_lead, Intent::ProposeSpec(spec())).unwrap();
    app.approve_spec(project, 1).unwrap();
    for _ in 0..4 {
        rt.tick(project).await.unwrap();
    }
    let planner_runs = app
        .store()
        .read(|c| runs::for_agent(c, planner, 50))
        .unwrap();
    assert_eq!(planner_runs.len(), 1, "one failed planner run, then none");
    assert_eq!(app.agent(planner).unwrap().status, AgentStatus::Offline);
    assert_eq!(needs_user(&app, "pl-planner cannot use devin"), 1);
    assert_eq!(p.requests().len(), 0);
}

/// ADR-63: a run the user stopped (paused project) is not a failed run, and a pause during a round starts nothing
/// after it: no later phase, no further review.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_pause_is_not_a_failure_and_ends_the_round() {
    let f = fixture();
    let p = Arc::new(ScriptedProvider::new("mock"));
    let (app, rt) = runtime(&f, p.clone(), Arc::new(AutoApprove));
    let rt = Arc::new(rt);
    app.update_config(|c| c.memory.learn = false).unwrap();
    let (project, agent) = solo_with_tickets(&app, &["PI-1", "PI-2", "PI-3"]);
    resolve(&app, project, agent, "AGN-1");
    resolve(&app, project, agent, "AGN-2");
    let pausing = |rt: &Arc<Runtime>| -> agenium_llm::mock::Responder {
        let r = rt.clone();
        Box::new(move |_| {
            r.pause_project(project).unwrap();
            Ok(calls_response("", vec![("list_dir", json!({}))]))
        })
    };
    let kinds = |runs: Vec<agenium_orchestrator::runtime::RunSummary>| -> Vec<(String, String)> {
        runs.iter()
            .map(|r| (r.kind.clone(), r.outcome.clone()))
            .collect()
    };
    // Paused during the work on AGN-3: the round reviews nothing after it.
    p.push(pausing(&rt));
    let runs = rt.tick(project).await.unwrap();
    assert_eq!(kinds(runs), vec![("work".into(), "Cancelled".into())]);
    // Paused again: two stopped runs in a row on AGN-3.
    rt.resume_project(project).unwrap();
    p.push(pausing(&rt));
    let s = rt.run_ticket(project, &key("AGN-3")).await.unwrap();
    assert_eq!(s.outcome, "Cancelled");
    // Resumed, AGN-3 goes on at once (a pause is no failure: no stall wait); a pause during the review of AGN-1
    // leaves AGN-2's review for later.
    rt.resume_project(project).unwrap();
    p.push_calls(
        "",
        vec![(
            "submit_report",
            serde_json::to_value(report("AGN-3")).unwrap(),
        )],
    );
    p.push(pausing(&rt));
    let runs = rt.tick(project).await.unwrap();
    assert_eq!(
        kinds(runs),
        vec![
            ("work".into(), "Completed".into()),
            ("review".into(), "Cancelled".into())
        ]
    );
    assert_eq!(p.remaining(), 0);
    assert_eq!(needs_user(&app, ""), 0);
}

/// A review that leaves the Lead out of quota ends the round's reviews (each would fail the same way).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_lead_out_of_quota_reviews_nothing_more() {
    let f = fixture();
    let p = Arc::new(ScriptedProvider::new("mock"));
    let (app, rt) = runtime(&f, p.clone(), Arc::new(AutoApprove));
    let (project, agent) = solo_with_tickets(&app, &["PI-1", "PI-2"]);
    resolve(&app, project, agent, "AGN-1");
    resolve(&app, project, agent, "AGN-2");
    p.push_error(agenium_llm::LlmError::Http {
        status: 429,
        kind: "insufficient_quota".into(),
        message: "quota".into(),
        quota: true,
        retry_after: None,
    });
    let runs = rt.tick(project).await.unwrap();
    let outcomes: Vec<&str> = runs.iter().map(|r| r.outcome.as_str()).collect();
    assert_eq!(outcomes, vec!["QuotaExhausted"]);
    assert_eq!(p.requests().len(), 1, "no second review was tried");
}

/// Reviews that keep ending at a checkpoint (each starts fresh) stop after MAX_FAILED_RUNS and the user is asked once;
/// the user's message to the Lead gives it another try.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reviews_stopping_at_a_checkpoint_go_to_the_user() {
    let f = fixture();
    let p = Arc::new(ScriptedProvider::new("mock"));
    let (app, rt) = runtime(&f, p.clone(), Arc::new(AutoApprove));
    let (project, agent) = solo_with_tickets(&app, &["PI-1"]);
    resolve(&app, project, agent, "AGN-1");
    app.update_config(|c| c.runtime.max_steps_per_run = 1)
        .unwrap();
    let review = |p: &ScriptedProvider| {
        p.push_calls("", vec![("list_dir", json!({}))]);
        p.push_text("Checked the report; the diff is next.");
    };
    for _ in 0..2 {
        review(&p);
        let runs = rt.tick(project).await.unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].outcome, "Checkpoint:max_steps");
    }
    assert!(
        rt.tick(project).await.unwrap().is_empty(),
        "no third fresh review"
    );
    assert!(rt.tick(project).await.unwrap().is_empty());
    assert_eq!(
        needs_user(&app, "reviews keep stopping"),
        1,
        "the user is asked once"
    );
    p.push_text("I will check AC-1 only.");
    rt.chat(project, "Only check AC-1.").await.unwrap();
    review(&p);
    assert_eq!(rt.tick(project).await.unwrap().len(), 1, "another try");
    assert_eq!(p.remaining(), 0);
}

/// The user's stop ends a command the run waits on at once (it ran on to its timeout, up to 10 minutes, before the stop
/// was seen at the next step), and the run ends cancelled without another model call.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_users_stop_ends_a_running_command_at_once() {
    let f = fixture();
    let p = Arc::new(ScriptedProvider::new("mock"));
    let (app, rt) = runtime(&f, p.clone(), Arc::new(AutoApprove));
    let rt = Arc::new(rt);
    app.update_config(|c| c.memory.learn = false).unwrap();
    let (project, _) = solo_with_tickets(&app, &["PI-1"]);
    let slow = match agenium_sandbox::Shell::detect() {
        agenium_sandbox::Shell::Bash(_) => "sleep 120",
        agenium_sandbox::Shell::PowerShell(_) => "Start-Sleep -Seconds 120",
        agenium_sandbox::Shell::Cmd => "powershell -NoProfile -Command Start-Sleep -Seconds 120",
    };
    p.push_calls(
        "",
        vec![("shell", json!({"command": slow, "timeout_ms": 300000}))],
    );
    p.push_calls("", vec![("list_dir", json!({}))]);
    let stopper = rt.clone();
    let calls = p.clone();
    tokio::spawn(async move {
        // Once the model asked for the command (a loaded machine may take a while to get there), and it runs.
        while calls.remaining() > 1 {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
        stopper.pause_project(project).unwrap();
    });
    let started = std::time::Instant::now();
    let s = rt.run_ticket(project, &key("AGN-1")).await.unwrap();
    assert_eq!(s.outcome, "Cancelled");
    assert!(
        started.elapsed() < std::time::Duration::from_secs(30),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(p.remaining(), 1, "no model call after the stop");
}

/// Quota used up in the middle of the work: the work so far stays (the worktree, the conversation), giving the agent
/// another model frees it at once (it stayed set aside until set back to Idle by hand), and the next run goes on from
/// where it stopped, the new model seeing what the old one did.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn quota_used_up_mid_work_goes_on_with_another_model() {
    let f = fixture();
    let p = Arc::new(ScriptedProvider::new("mock"));
    let (app, rt) = runtime(&f, p.clone(), Arc::new(AutoApprove));
    app.update_config(|c| c.memory.learn = false).unwrap();
    let (project, agent) = solo_with_tickets(&app, &["PI-1"]);
    p.push_calls(
        "",
        vec![(
            "write_file",
            json!({"path": "part1.txt", "content": "first half"}),
        )],
    );
    p.push_error(agenium_llm::LlmError::Http {
        status: 429,
        kind: "insufficient_quota".into(),
        message: "quota".into(),
        quota: true,
        retry_after: None,
    });
    let s = rt.run_ticket(project, &key("AGN-1")).await.unwrap();
    assert_eq!(s.outcome, "QuotaExhausted");
    assert_eq!(
        app.agent(agent).unwrap().status,
        AgentStatus::QuotaExhausted
    );
    let t = app.ticket(project, &key("AGN-1")).unwrap();
    let wt = std::path::PathBuf::from(
        app.store()
            .read(|c| agenium_store::core::workspaces::get(c, t.id))
            .unwrap()
            .unwrap()
            .worktree,
    );
    assert!(wt.join("part1.txt").exists(), "the work so far stays");

    let mut other = app.agent(agent).unwrap().model;
    other.model = "another-model".into();
    app.set_agent_model(agent, &other).unwrap();
    assert_eq!(
        app.agent(agent).unwrap().status,
        AgentStatus::Idle,
        "a new model frees the agent"
    );
    p.push(Box::new(|req| {
        let all = serde_json::to_string(&req.messages).unwrap();
        assert!(
            all.contains("first half") && all.contains("Your run resumes"),
            "the new model sees what the old one did"
        );
        Ok(calls_response(
            "",
            vec![(
                "write_file",
                json!({"path": "part2.txt", "content": "second half"}),
            )],
        ))
    }));
    p.push_calls(
        "",
        vec![(
            "submit_report",
            serde_json::to_value(report("AGN-1")).unwrap(),
        )],
    );
    let s = rt.run_ticket(project, &key("AGN-1")).await.unwrap();
    assert_eq!(s.outcome, "Completed");
    assert!(wt.join("part1.txt").exists() && wt.join("part2.txt").exists());
}

/// ADR-135: an agent set aside for its quota goes back to work with the same model once the quota should be back: when
/// its vendor's used-up window resets, or tried again from 30 minutes on (then 60, 120...); its open ticket is told.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_agent_out_of_quota_goes_back_to_work_when_the_quota_is_back() {
    use std::time::{Duration, SystemTime};
    let f = fixture();
    let p = Arc::new(ScriptedProvider::new("mock"));
    let (app, rt) = runtime(&f, p.clone(), Arc::new(AutoApprove));
    let (project, agent) = solo_with_tickets(&app, &["PI-1"]);
    let model = app.agent(agent).unwrap().model;
    let set_aside = || {
        app.set_agent_status(agent, AgentStatus::QuotaExhausted)
            .unwrap()
    };
    let min = |m: u64| Duration::from_secs(m * 60);
    let t0 = SystemTime::now();
    set_aside();
    assert!(rt.quota_back_round(t0).unwrap().is_empty(), "not yet");
    assert!(rt.quota_back_round(t0 + min(29)).unwrap().is_empty());
    assert_eq!(rt.quota_back_round(t0 + min(31)).unwrap(), vec![agent]);
    assert_eq!(app.agent(agent).unwrap().status, AgentStatus::Idle);
    assert_eq!(app.agent(agent).unwrap().model, model, "the same model");
    let t = app.ticket(project, &key("AGN-1")).unwrap();
    let said = app
        .store()
        .read(|c| agenium_store::core::comments::for_ticket(c, t.id))
        .unwrap()
        .iter()
        .any(|c| matches!(&c.body, CommentBody::System { text } if text.contains("quota on") && text.contains("should be back")));
    assert!(said, "its ticket is told");
    // Another project's round sees it at work meanwhile: the backoff is kept.
    assert!(rt.quota_back_round(t0 + min(40)).unwrap().is_empty());
    // Still used up: set aside again, tried an hour later (not 30 minutes).
    set_aside();
    assert!(rt.quota_back_round(t0 + min(75)).unwrap().is_empty());
    assert!(rt.quota_back_round(t0 + min(80)).unwrap().is_empty());
    assert_eq!(rt.quota_back_round(t0 + min(92)).unwrap(), vec![agent]);
    // Its vendor said when the window resets: it goes back then.
    let mut cli = model.clone();
    cli.provider = "codex".into();
    app.set_agent_backend(agent, BackendKind::Cli, &cli)
        .unwrap();
    // A round while it works ends its wait; the next time it starts afresh.
    assert!(rt.quota_back_round(t0 + min(199)).unwrap().is_empty());
    let reset = chrono::DateTime::<chrono::Utc>::from(t0 + min(215));
    app.record_vendor_usage(agenium_llm::vendor_usage::VendorUsage {
        windows: vec![agenium_llm::vendor_usage::UsageWindow {
            window: "5h".into(),
            used_percent: 100.0,
            resets_at: Some(reset.to_rfc3339()),
        }],
        ..agenium_llm::vendor_usage::VendorUsage::new("codex", "")
    })
    .unwrap();
    set_aside();
    assert!(rt.quota_back_round(t0 + min(200)).unwrap().is_empty());
    assert!(
        rt.quota_back_round(t0 + min(212)).unwrap().is_empty(),
        "the window has not reset (no 30-minute try meanwhile)"
    );
    assert_eq!(rt.quota_back_round(t0 + min(217)).unwrap(), vec![agent]);
}

/// A provider whose model never answers: its run waits until its future is dropped.
struct Silent;

#[async_trait]
impl Provider for Silent {
    fn id(&self) -> &str {
        "mock"
    }
    fn kind(&self) -> &'static str {
        "mock"
    }
    async fn list_models(&self) -> Result<Vec<agenium_llm::ModelInfo>, agenium_llm::LlmError> {
        Ok(vec![])
    }
    async fn chat(
        &self,
        _req: &agenium_llm::ChatRequest,
        _emit: &(dyn Fn(agenium_llm::ChatEvent) + Send + Sync),
    ) -> Result<agenium_llm::ChatResponse, agenium_llm::LlmError> {
        std::future::pending().await
    }
}

/// ADR-147: a run whose task panics, or whose future is dropped, is ended as Interrupted at once: its agent is Idle
/// and its ticket ready for a continuation run, without a restart.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_panicking_or_dropped_run_is_ended_without_a_restart() {
    let f = fixture();
    let p = Arc::new(ScriptedProvider::new("mock"));
    let (app, rt) = runtime(&f, p.clone(), Arc::new(AutoApprove));
    let (project, agent) = solo_with_tickets(&app, &["PI-1"]);
    let open = |app: &agenium_orchestrator::App| app.store().read(runs::unfinished).unwrap();
    let ready = |app: &agenium_orchestrator::App| {
        app.ready_tickets(project)
            .unwrap()
            .iter()
            .any(|r| r.ticket.key == key("AGN-1"))
    };

    // A panic in the run (here: the provider's).
    p.push(Box::new(|_| panic!("a bug in the provider")));
    let rt = Arc::new(rt);
    let task = tokio::spawn({
        let rt = rt.clone();
        async move { rt.run_ticket(project, &key("AGN-1")).await }
    });
    assert!(task.await.unwrap_err().is_panic());
    assert!(open(&app).is_empty(), "the panicked run was ended");
    assert_eq!(app.agent(agent).unwrap().status, AgentStatus::Idle);
    assert!(ready(&app), "the ticket goes on without a restart");

    // A run whose future is dropped (a timeout, a task aborted).
    let silent = Runtime::new(
        app.clone(),
        Arc::new(StaticProviders(vec![(
            "mock".into(),
            Arc::new(Silent) as Arc<dyn Provider>,
        )])),
        Arc::new(AutoApprove),
    );
    let r = tokio::time::timeout(
        std::time::Duration::from_millis(1500),
        silent.run_ticket(project, &key("AGN-1")),
    )
    .await;
    assert!(r.is_err(), "the model never answered");
    assert!(open(&app).is_empty(), "the dropped run was ended");
    assert_eq!(app.agent(agent).unwrap().status, AgentStatus::Idle);
    assert!(ready(&app));
    let t = app.ticket(project, &key("AGN-1")).unwrap();
    let rs = app.store().read(|c| runs::for_ticket(c, t.id)).unwrap();
    assert_eq!(rs.len(), 2);
    assert!(rs
        .iter()
        .all(|r| r.outcome.as_deref() == Some("Interrupted")));
}

/// An agent whose API provider cannot be built (its key not set, or not configured) waits for the user, told once,
/// like a CLI that cannot run; it was retried as a plain error and nobody was told (seen after a vault reset).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_agent_whose_provider_has_no_key_waits_for_the_user() {
    let f = fixture();
    let p = Arc::new(ScriptedProvider::new("mock"));
    let (app, rt) = runtime(&f, p.clone(), Arc::new(AutoApprove));
    app.update_config(|c| c.memory.learn = false).unwrap();
    let (project, agent) = solo_with_tickets(&app, &["PI-1"]);
    let nokey = ModelBinding {
        provider: "nokey".into(),
        model: "m".into(),
        effort: Default::default(),
        max_tokens: None,
    };
    app.set_agent_backend(agent, BackendKind::Api, &nokey)
        .unwrap();
    assert!(rt.run_ticket(project, &key("AGN-1")).await.is_err());
    assert_eq!(app.agent(agent).unwrap().status, AgentStatus::Offline);
    assert_eq!(needs_user(&app, "cannot use nokey"), 1);
    assert_eq!(p.requests().len(), 0, "no model was called");
}

/// ADR-158: an agent whose provider could not be reached is tried again by itself, two minutes later and then twice as
/// long each time; after six tries it waits for the user.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unreachable_agent_is_tried_again_by_itself() {
    use std::time::{Duration, SystemTime};
    let f = fixture();
    let p = Arc::new(ScriptedProvider::new("mock"));
    let (app, rt) = runtime(&f, p.clone(), Arc::new(AutoApprove));
    let (_, agent) = solo_with_tickets(&app, &["PI-1"]);
    let min = |m: u64| Duration::from_secs(m * 60);
    let t0 = SystemTime::now();
    let offline = |at: SystemTime| {
        app.set_agent_status(agent, AgentStatus::Offline).unwrap();
        rt.offline_later(agent, at)
    };
    assert_eq!(offline(t0), Some(min(2)));
    assert!(
        rt.offline_back_round(t0 + min(1)).unwrap().is_empty(),
        "not yet"
    );
    assert_eq!(rt.offline_back_round(t0 + min(3)).unwrap(), vec![agent]);
    assert_eq!(app.agent(agent).unwrap().status, AgentStatus::Idle);
    // Still unreachable: twice as long, then the tries are spent and the user is left to it.
    assert_eq!(offline(t0 + min(3)), Some(min(4)));
    assert!(rt.offline_back_round(t0 + min(6)).unwrap().is_empty());
    assert_eq!(rt.offline_back_round(t0 + min(7)).unwrap(), vec![agent]);
    for _ in 0..4 {
        assert!(offline(t0 + min(8)).is_some());
    }
    assert_eq!(offline(t0 + min(8)), None);
    assert!(
        rt.offline_back_round(t0 + min(200)).unwrap().is_empty(),
        "left to the user"
    );
    assert_eq!(app.agent(agent).unwrap().status, AgentStatus::Offline);
    // An agent the user set back to work meanwhile is not touched.
    app.set_agent_status(agent, AgentStatus::Idle).unwrap();
    assert!(rt.offline_back_round(t0 + min(200)).unwrap().is_empty());
}
