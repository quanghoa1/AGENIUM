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
    app.set_agent_backend(agent, BackendKind::Cli, &devin())
        .unwrap();
    for err in [
        rt.run_ticket(project, &key("AGN-1")).await.unwrap_err(),
        rt.chat(project, "hello").await.unwrap_err(),
    ] {
        assert!(err.to_string().contains("does not drive devin"), "{err}");
        assert!(open(&app).is_empty(), "the failed run was ended");
        assert_eq!(app.agent(agent).unwrap().status, AgentStatus::Idle);
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

/// A planner or Lead inbox run that fails before its loop (here: its CLI cannot run) counts as a failed run: after
/// MAX_FAILED_RUNS the user is asked once, instead of a failing run every round.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failing_planner_and_inbox_runs_go_to_the_user() {
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
    app.set_agent_backend(lead, BackendKind::Cli, &devin())
        .unwrap();
    for _ in 0..4 {
        assert!(rt.tick(project).await.unwrap().is_empty());
    }
    let lead_runs = app.store().read(|c| runs::for_agent(c, lead, 50)).unwrap();
    assert_eq!(lead_runs.len(), 2, "two failed inbox runs, then none");
    assert!(lead_runs.iter().all(|r| r.ended_at.is_some()));
    assert_eq!(needs_user(&app, "could not handle its inbox"), 1);

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
    app.set_agent_backend(planner, BackendKind::Cli, &devin())
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
    assert_eq!(planner_runs.len(), 2, "two failed planner runs, then none");
    assert_eq!(needs_user(&app, "could not draft plan v1"), 1);
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
/// the user's comment on the ticket gives the Lead another try.
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
    app.user_comment(project, &key("AGN-1"), "Only check AC-1.")
        .unwrap();
    review(&p);
    assert_eq!(rt.tick(project).await.unwrap().len(), 1, "another try");
    assert_eq!(p.remaining(), 0);
}
