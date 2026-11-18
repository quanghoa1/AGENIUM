//! A team project: a CLI that cannot run, tool calling required, the Lead's inbox, the planner, the chat learned.

use super::*;

/// ADR-132: a CLI agent whose CLI cannot run waits for the user with the reason (set aside, told in Needs you); its work
/// never moves to another model by itself, not even to the API model `[cli]` names for CLI agents' memory. The user
/// gives it another model and it works again.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cli_agent_whose_cli_cannot_run_waits_for_the_user() {
    let f = fixture();
    let p = Arc::new(ScriptedProvider::new("mock"));
    let (app, rt) = runtime(&f, p.clone(), Arc::new(AutoApprove));
    let (project, agent) = solo(&app);
    app.update_config(|c| {
        c.cli.fallback_provider = "mock".into();
        c.cli.fallback_model = "memory-model".into();
    })
    .unwrap();
    // An agent CLI that is not installed: the agent cannot run.
    let cli_model = super::no_cli(&rt);
    app.set_agent_backend(agent, BackendKind::Cli, &cli_model)
        .unwrap();
    let err = rt.chat(project, "hello").await.unwrap_err().to_string();
    assert!(err.contains("devin is not installed"), "{err}");
    assert_eq!(p.requests().len(), 0, "no other model takes the work");
    assert_eq!(app.agent(agent).unwrap().status, AgentStatus::Offline);
    let told = app
        .store()
        .read(|c| events::recent_of_kind(c, "NeedsUser", 5))
        .unwrap();
    assert!(
        told.iter().any(|e| e.payload["reason"]
            .as_str()
            .unwrap_or_default()
            .contains("cannot use devin")),
        "{told:?}"
    );

    // The user gives it another model: it works again at once.
    let mut api = cli_model.clone();
    api.provider = "mock".into();
    app.set_agent_backend(agent, BackendKind::Api, &api)
        .unwrap();
    assert_eq!(app.agent(agent).unwrap().status, AgentStatus::Idle);
    p.push_text("hi from the API");
    let r = rt.chat(project, "hello").await.unwrap();
    assert!(r.text.contains("hi from the API"), "{}", r.text);
}

/// ADR-31: a model known not to call tools is refused before any request; hand-set flags win over the endpoint.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn model_without_tool_calling_is_refused() {
    let f = fixture();
    let p = Arc::new(ScriptedProvider::new("mock"));
    let (app, rt) = runtime(&f, p.clone(), Arc::new(AutoApprove));
    let (project, _) = solo(&app);
    app.update_config(|c| {
        c.providers.push(agenium_config::ProviderConfig {
            body: vec![],
            effort_field: None,
            headers: Default::default(),
            id: "mock".into(),
            kind: agenium_config::ProviderKind::Mock,
            base_url: None,
            api_key_ref: String::new(),
            max_concurrency: 1,
            models: vec![],
            model_caps: vec![agenium_config::ModelCaps {
                efforts: None,
                default_effort: None,
                model: "scripted".into(),
                tools: Some(false),
                vision: None,
                context_window: None,
            }],
        })
    })
    .unwrap();
    let err = rt.chat(project, "hello").await.unwrap_err().to_string();
    assert!(err.contains("does not support tool calling"), "{err}");
    assert_eq!(p.requests().len(), 0);
}

/// M4 acceptance (scripted model): a team (Lead + 2 workers) takes an approved plan with a dependency to two
/// closed, merged tickets. The Lead's inbox turns the plan into assigned tickets and answers a worker's blocking
/// question; the dependent ticket starts only after the first one is approved and merged.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn team_lead_inbox_takes_a_plan_to_closed_tickets() {
    let f = fixture();
    let p = Arc::new(ScriptedProvider::new("mock"));
    let (app, rt) = runtime(&f, p.clone(), Arc::new(AutoApprove));
    // Memory extraction would take turns from the shared script.
    app.update_config(|c| c.memory.learn = false).unwrap();
    let project = app
        .create_project(&agenium_orchestrator::CreateProject {
            name: "duo".into(),
            path: None,
            key_prefix: None,
            template: None,
            folder: None,
            repo: None,
        })
        .unwrap()
        .id;
    let (_, members) = app
        .create_team(
            project,
            "duo team",
            &agent_spec("lead", "lead", RoleKind::Lead, &[]),
            &[
                agent_spec("api-1", "api", RoleKind::Worker, &["rust"]),
                agent_spec("web-1", "web", RoleKind::Worker, &["web"]),
            ],
        )
        .unwrap();
    let (lead, api, web) = (members[0].id, members[1].id, members[2].id);
    let as_lead = agenium_orchestrator::CallCtx {
        project,
        actor: Actor::Lead(lead),
        intent_id: None,
        run: None,
    };
    app.apply(&as_lead, Intent::ProposeSpec(spec())).unwrap();
    app.approve_spec(project, 1).unwrap();
    app.apply(
        &as_lead,
        Intent::CreatePlan(plan(&[("PI-1", &[]), ("PI-2", &["PI-1"])])),
    )
    .unwrap();
    let inbox = app.lead_inbox(project).unwrap();
    assert_eq!(inbox.len(), 1);
    assert!(inbox[0].text.contains("PI-1") && inbox[0].text.contains("PI-2"));

    let ticket = |item: &str, title: &str, deps: Vec<&str>| {
        json!({"kind": "Task", "plan_item": item, "title": title, "description": "Write the file",
               "acceptance": [{"id": "AC-1", "text": "the file exists"}], "depends_on": deps})
    };
    // R1 Lead inbox: plan -> tickets -> assignments (the run ends when the inbox is empty).
    p.push_calls(
        "",
        vec![
            ("create_ticket", ticket("PI-1", "API file", vec![])),
            ("create_ticket", ticket("PI-2", "Web file", vec!["AGN-1"])),
            (
                "assign_ticket",
                json!({"ticket_id": "AGN-1", "worker": api.to_string()}),
            ),
            (
                "assign_ticket",
                json!({"ticket_id": "AGN-2", "worker": web.to_string()}),
            ),
        ],
    );
    // R2 api-1 on AGN-1: blocking question -> parked.
    p.push_calls(
        "",
        vec![(
            "ask_question",
            json!({"ticket_id": "AGN-1", "category": "Requirement", "question": "Which file name should the API use?",
                   "context": "", "options": [], "blocking": true, "urgency": "High"}),
        )],
    );
    // R3 Lead inbox: the blocking question (its id comes from the task text).
    p.push(Box::new(|req| {
        let task = req.messages[0].text();
        let id = task
            .split("Blocking question ")
            .nth(1)
            .and_then(|s| s.split_whitespace().next())
            .expect("the inbox lists the question with its id")
            .to_owned();
        assert!(task.contains("api-1"), "the asker is named");
        assert!(
            task.contains("- REQ-1: A greeting file exists")
                && task.contains("  - AC-1: hello.txt says hello"),
            "the Lead answers from the spec text, not just its size"
        );
        Ok(calls_response(
            "",
            vec![(
                "answer_question",
                json!({"ticket_id": "AGN-1", "reply_to": id, "answer": "Use deliverables/api.txt"}),
            )],
        ))
    }));
    // R4 api-1 resumes with the answer in context.
    p.push(Box::new(|req| {
        let all = serde_json::to_string(&req.messages).unwrap();
        assert!(
            all.contains("Use deliverables/api.txt"),
            "the answer reaches the worker"
        );
        Ok(calls_response(
            "",
            vec![
                (
                    "write_file",
                    json!({"path": "deliverables/api.txt", "content": "api\n"}),
                ),
                (
                    "submit_report",
                    serde_json::to_value(report("AGN-1")).unwrap(),
                ),
            ],
        ))
    }));
    // R5, R6 Lead review of AGN-1.
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
            json!({"ticket_id": "AGN-1", "reason": "AC-1 met, merged"}),
        )],
    );
    // R7 web-1 on AGN-2 (unlocked now), R8, R9 review.
    p.push_calls(
        "",
        vec![
            (
                "write_file",
                json!({"path": "deliverables/web.txt", "content": "web\n"}),
            ),
            (
                "submit_report",
                serde_json::to_value(report("AGN-2")).unwrap(),
            ),
        ],
    );
    p.push_calls(
        "",
        vec![(
            "review_ticket",
            json!({"ticket_id": "AGN-2", "decision": "Approve"}),
        )],
    );
    p.push_calls(
        "",
        vec![(
            "close_ticket",
            json!({"ticket_id": "AGN-2", "reason": "AC-1 met, merged"}),
        )],
    );

    let runs = rt.drive(project, 8).await.unwrap();
    let kinds: Vec<(String, String, Option<String>)> = runs
        .iter()
        .map(|r| {
            (
                r.kind.clone(),
                r.agent.clone(),
                r.ticket.as_ref().map(|k| k.to_string()),
            )
        })
        .collect();
    let k = |kind: &str, agent: &str, t: Option<&str>| {
        (kind.to_string(), agent.to_string(), t.map(str::to_string))
    };
    assert_eq!(
        kinds,
        vec![
            k("inbox", "lead", None),
            k("work", "api-1", Some("AGN-1")),
            k("inbox", "lead", None),
            k("work", "api-1", Some("AGN-1")),
            k("review", "lead", Some("AGN-1")),
            k("work", "web-1", Some("AGN-2")),
            k("review", "lead", Some("AGN-2")),
        ]
    );
    assert_eq!(p.remaining(), 0, "every scripted turn was used");
    for key in ["AGN-1", "AGN-2"] {
        let t = app.ticket(project, &key.parse().unwrap()).unwrap();
        assert_eq!(t.status, TicketStatus::Closed, "{key}");
        assert!(t.merged, "{key}");
    }
    assert_eq!(
        app.ticket(project, &"AGN-1".parse().unwrap())
            .unwrap()
            .assignee,
        Some(api)
    );
    assert_eq!(
        app.ticket(project, &"AGN-2".parse().unwrap())
            .unwrap()
            .assignee,
        Some(web)
    );
    let pv = app.plan_view(project).unwrap();
    assert!(pv.items.iter().all(|i| i.done), "both plan items ticked");
    assert!(app.lead_inbox(project).unwrap().is_empty());
}

/// ADR-59 through the scheduler: approved spec -> planner run (draft) -> Lead inbox run (create_plan from the
/// draft) -> plan waits for the user; the planner does nothing more.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn planner_drafts_then_the_lead_creates_the_plan() {
    let f = fixture();
    let p = Arc::new(ScriptedProvider::new("mock"));
    let (app, rt) = runtime(&f, p.clone(), Arc::new(AutoApprove));
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
    let (team, _) = app.team_of_project(project).unwrap().unwrap();
    let as_lead = agenium_orchestrator::CallCtx {
        project,
        actor: Actor::Lead(team.lead.unwrap()),
        intent_id: None,
        run: None,
    };
    app.apply(&as_lead, Intent::ProposeSpec(spec())).unwrap();
    app.approve_spec(project, 1).unwrap();

    let plan_json = serde_json::to_value(plan(&[("PI-1", &[]), ("PI-2", &["PI-1"])])).unwrap();
    let draft = {
        let mut d = plan_json.clone();
        d["rationale"] = json!("Backend first, the frontend item depends on its API.");
        d
    };
    p.push(Box::new(move |req| {
        let task = req.messages[0].text();
        assert!(
            task.contains("Draft plan v1")
                && task.contains("coding-backend")
                && !task.contains("pl-planner")
                // Where each worker writes, and the split for parallel work.
                && task.contains("writes ")
                && task.contains("Split for parallel work"),
            "{task}"
        );
        let tools: Vec<&str> = req.tools.iter().map(|t| t.name.as_str()).collect();
        assert!(
            tools.contains(&"submit_plan_draft")
                && !tools.contains(&"submit_report")
                && !tools.contains(&"write_file")
        );
        Ok(calls_response(
            "",
            vec![("submit_plan_draft", draft.clone())],
        ))
    }));
    p.push(Box::new(move |req| {
        let task = req.messages[0].text();
        assert!(
            task.contains("Backend first"),
            "the Lead sees the planner's rationale"
        );
        Ok(calls_response("", vec![("create_plan", plan_json.clone())]))
    }));
    // One round: the planner's draft, then the Lead's plan (approved by the Lead, ADR-63; the next round would
    // have the Lead create tickets).
    let runs = rt.drive(project, 1).await.unwrap();
    let kinds: Vec<(String, String)> = runs
        .iter()
        .map(|r| (r.kind.clone(), r.agent.clone()))
        .collect();
    assert_eq!(
        kinds,
        vec![
            ("plan".to_string(), "pl-planner".to_string()),
            ("inbox".to_string(), "pl-lead".to_string())
        ]
    );
    assert_eq!(p.remaining(), 0);
    let pv = app.plan_view(project).unwrap();
    let plan = pv.plan.unwrap();
    assert_eq!(
        (plan.version, plan.approved_by.is_some()),
        (1, true),
        "the agreed spec's plan is the Lead's"
    );
    assert!(
        plan.proposed_by.starts_with("agent:"),
        "proposed by the Lead"
    );
}
