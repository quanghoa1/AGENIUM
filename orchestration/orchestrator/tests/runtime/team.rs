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

/// ADR-98: the chat with the Lead is learned from once it has gone quiet (by the scheduler's round), each part once,
/// facts and preferences only, without the project state appended to each user message.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_chat_with_the_lead_is_learned_once_it_goes_quiet() {
    let f = fixture();
    let p = Arc::new(ScriptedProvider::new("mock"));
    let (app, rt) = runtime(&f, p.clone(), Arc::new(AutoApprove));
    let (project, agent) = solo(&app);
    let e = app.embedder();
    let mem = app.agent_store(agent, e.dim(), e.model_id()).unwrap();
    let quiet = || {
        mem.set_meta(&format!("chat-last:{project}"), "2020-01-01T00:00:00Z")
            .unwrap()
    };

    p.push_text("Noted: Vietnamese from now on, and `make ship` deploys.");
    rt.chat(
        project,
        "Always answer me in Vietnamese. We deploy this project with `make ship`.",
    )
    .await
    .unwrap();
    // Not quiet yet: nothing happens (no model call is scripted).
    assert!(rt.learn_from_chat(project, false).await.unwrap().is_none());

    quiet();
    p.push(Box::new(|req| {
        let t = req.messages[0].text();
        assert!(t.contains("Episode chat:greet"), "{t}");
        assert!(t.contains("SIGNALS: none"), "{t}");
        assert!(t.contains("[User] Always answer me in Vietnamese."), "{t}");
        assert!(t.contains("[Assistant] Noted"), "{t}");
        assert!(!t.contains("project_state"), "{t}");
        Ok(calls_response(
            "{\"facts\": [{\"text\": \"The user wants answers in Vietnamese.\", \"kind\": \"preference\"}, \
             {\"text\": \"The greet project deploys with make ship.\", \"kind\": \"fact\"}]}",
            vec![],
        ))
    }));
    // The scheduler's round learns from it.
    rt.tick(project).await.unwrap();
    assert_eq!(p.remaining(), 0, "one extraction call");
    let facts = mem.live(10).unwrap();
    assert_eq!(facts.len(), 2, "{facts:?}");
    assert!(facts.iter().all(|f| f.source == "chat:greet"));
    assert!(facts
        .iter()
        .any(|f| f.kind == agenium_store::agent::FactKind::Preference));
    // Each part once.
    assert!(rt.learn_from_chat(project, true).await.unwrap().is_none());

    // The next part of the chat: only it is sent.
    p.push_text("Sure.");
    rt.chat(project, "Please keep commit messages short.")
        .await
        .unwrap();
    p.push(Box::new(|req| {
        let t = req.messages[0].text();
        assert!(
            t.contains("[User] Please keep commit messages short."),
            "{t}"
        );
        assert!(!t.contains("Vietnamese"), "{t}");
        Ok(calls_response("{\"facts\": []}", vec![]))
    }));
    assert!(rt.learn_from_chat(project, true).await.unwrap().is_some());
    assert_eq!(p.remaining(), 0);

    // Off: never.
    p.push_text("Ok.");
    rt.chat(project, "One more thing.").await.unwrap();
    quiet();
    app.update_config(|c| c.memory.chat_idle_minutes = 0)
        .unwrap();
    assert!(rt.learn_from_chat(project, false).await.unwrap().is_none());
}

/// ADR-98: a failed chat extraction is not retried on every scheduler round.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_chat_extraction_waits_before_the_next_try() {
    let f = fixture();
    let p = Arc::new(ScriptedProvider::new("mock"));
    let (app, rt) = runtime(&f, p.clone(), Arc::new(AutoApprove));
    let (project, agent) = solo(&app);
    let e = app.embedder();
    let mem = app.agent_store(agent, e.dim(), e.model_id()).unwrap();
    p.push_text("Ok.");
    rt.chat(project, "We use pnpm, not npm.").await.unwrap();
    mem.set_meta(&format!("chat-last:{project}"), "2020-01-01T00:00:00Z")
        .unwrap();
    p.push_error(agenium_llm::LlmError::Unsupported("provider down".into()));
    assert!(rt.learn_from_chat(project, false).await.unwrap().is_none());
    assert_eq!(p.remaining(), 0);
    // The next rounds make no model call.
    let asked = p.requests().len();
    rt.tick(project).await.unwrap();
    assert!(rt.learn_from_chat(project, false).await.unwrap().is_none());
    assert_eq!(p.requests().len(), asked);
    // Forced, it tries again and learns.
    p.push(Box::new(|_| Ok(calls_response("{\"facts\": []}", vec![]))));
    assert!(rt.learn_from_chat(project, true).await.unwrap().is_some());
    assert_eq!(p.remaining(), 0);
}

/// ADR-149: a message to a running Lead waits in its queue (it does not reach the run going on), where the user can
/// change it; the Lead's next chat turn reads every queued message; a worker is never messaged (ADR-104).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_user_messages_a_running_lead_and_never_a_worker() {
    let f = fixture();
    let p = Arc::new(ScriptedProvider::new("mock"));
    let (app, rt) = runtime(&f, p.clone(), Arc::new(AutoApprove));
    let rt = Arc::new(rt);
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
            &[agent_spec("api-1", "api", RoleKind::Worker, &["rust"])],
        )
        .unwrap();
    let (lead, api) = (members[0].id, members[1].id);
    assert!(!rt.steer(project, "nobody runs").unwrap());

    // The Lead's chat turn: the user writes twice while it works; both wait in the queue, and one is changed.
    let r2 = rt.clone();
    let a2 = app.clone();
    p.push(Box::new(move |_| {
        assert!(r2.steer(project, "Also keep the API in Rust.").unwrap());
        let busy = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(r2.chat(project, "And add a README."))
        })
        .unwrap();
        assert_eq!(busy.kind, "queued");
        let q = a2.undelivered_lead_messages(project).unwrap();
        assert_eq!(q.len(), 2);
        a2.edit_lead_message(project, q[0].id, "Also keep the API in Go.")
            .unwrap();
        Ok(calls_response("", vec![("list_dir", json!({}))]))
    }));
    p.push(Box::new(move |req| {
        let all: String = req.messages.iter().map(|m| m.text()).collect();
        assert!(
            !all.contains("keep the API") && !all.contains("README"),
            "{all}"
        );
        Ok(calls_response("Planning.", vec![]))
    }));
    let s = rt.chat(project, "Plan an API.").await.unwrap();
    assert_eq!(s.outcome, "Completed");
    // The next turn reads the queue, as changed, with the user's new message after it.
    p.push(Box::new(|req| {
        let last = req.messages.last().unwrap().text();
        let go = last.find("Also keep the API in Go.").unwrap();
        let readme = last.find("And add a README.").unwrap();
        let thanks = last.find("Thanks.").unwrap();
        assert!(go < readme && readme < thanks, "{last}");
        assert!(!last.contains("in Rust"), "{last}");
        Ok(calls_response("Will do.", vec![]))
    }));
    rt.chat(project, "Thanks.").await.unwrap();
    assert!(app.undelivered_lead_messages(project).unwrap().is_empty());
    // Read by the Lead: no longer changed or taken back.
    assert!(app.drop_lead_message(project, 1).is_err());

    // A worker's run: the user cannot reach it (only its ticket and the Lead's plan give it work).
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
    app.apply(
        &as_lead,
        Intent::CreateTicket(new_ticket("PI-1", "API file", &[], &[])),
    )
    .unwrap();
    app.apply(
        &as_lead,
        Intent::AssignTicket(agenium_core::forms::AssignTicket {
            ticket_id: key("AGN-1"),
            worker: api,
            note: None,
        }),
    )
    .unwrap();
    let r4 = rt.clone();
    p.push(Box::new(move |req| {
        assert!(
            !r4.steer(project, "worker, listen").unwrap(),
            "workers are not messaged"
        );
        assert!(!format!("{:?}", req.messages).contains("while you work"));
        Err(agenium_llm::LlmError::Unsupported("stop here".into()))
    }));
    let _ = rt.run_ticket(project, &key("AGN-1")).await;
    assert_eq!(p.remaining(), 0);
}

/// ADR-106: a long chat with the Lead is seen as a summary of its older part plus the recent messages; the summary is
/// made once and kept, and the stored conversation stays whole.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_long_chat_is_summarised_once_and_kept() {
    let f = fixture();
    let p = Arc::new(ScriptedProvider::new("mock"));
    let (app, rt) = runtime(&f, p.clone(), Arc::new(AutoApprove));
    let (project, agent) = solo(&app);
    let e = app.embedder();
    let mem = app.agent_store(agent, e.dim(), e.model_id()).unwrap();
    let session = format!("chat:{project}");
    for i in 0..20 {
        for m in [
            agenium_llm::Message::user_text(format!("turn {i}: {}", "x".repeat(12_000))),
            agenium_llm::Message::assistant_text(format!("ok {i}")),
        ] {
            mem.append_message(&session, &serde_json::to_value(m).unwrap())
                .unwrap();
        }
    }
    p.push(Box::new(|req| {
        assert!(req.tools.is_empty());
        let t = req.messages[0].text();
        assert!(
            t.contains("turn 0:") && !t.contains("turn 19:"),
            "only the older part"
        );
        Ok(calls_response(
            "## Goal\nsummary of the early turns",
            vec![],
        ))
    }));
    p.push(Box::new(|req| {
        let first = req.messages[0].text();
        assert!(
            first.starts_with("[Earlier conversation, summarised]")
                && first.contains("summary of the early turns"),
            "{first}"
        );
        let all: String = req.messages.iter().map(|m| m.text()).collect();
        assert!(all.contains("turn 19:") && !all.contains("turn 0:"));
        Ok(calls_response("ok", vec![]))
    }));
    let first = rt.chat(project, "next").await.unwrap();
    // The summary's model call is a helper call of its own (ADR-155: written by the Lead's own model, API or CLI),
    // counted once in Usage, not again in the turn's.
    assert_eq!(first.usage.tokens_in, 100);
    let calls = app
        .store()
        .read(|c| agenium_store::core::helper_calls::recent(c, 10))
        .unwrap();
    assert!(
        calls
            .iter()
            .any(|c| c.kind == "chat-summary" && c.usage.tokens_in == 100),
        "{:?}",
        calls.iter().map(|c| &c.kind).collect::<Vec<_>>()
    );
    // The next turn reuses the summary: no new summary call.
    p.push(Box::new(|req| {
        assert!(req.messages[0]
            .text()
            .starts_with("[Earlier conversation, summarised]"));
        Ok(calls_response("ok again", vec![]))
    }));
    let next = rt.chat(project, "and next").await.unwrap();
    assert_eq!(next.usage.tokens_in, 100);
    assert_eq!(p.remaining(), 0);
    assert_eq!(
        mem.load_messages(&session).unwrap().len(),
        44,
        "the stored conversation stays whole"
    );
}

/// A Lead with a run going (a chat turn marks no one Working) is not given an inbox run beside it: the two planned the
/// same work at once and rewrote each other's plan (2026-11-05). Once the run ends, the inbox runs.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn no_inbox_run_while_the_lead_has_a_run_going() {
    let f = fixture();
    let p = Arc::new(ScriptedProvider::new("mock"));
    let (app, rt) = runtime(&f, p.clone(), Arc::new(AutoApprove));
    app.update_config(|c| c.memory.learn = false).unwrap();
    let project = app
        .create_project(&agenium_orchestrator::CreateProject {
            name: "busy".into(),
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
            "busy team",
            &agent_spec("lead", "lead", RoleKind::Lead, &[]),
            &[agent_spec("api-1", "api", RoleKind::Worker, &["rust"])],
        )
        .unwrap();
    let lead = members[0].id;
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
    assert_eq!(
        app.lead_inbox(project).unwrap().len(),
        1,
        "a plan item waits for its ticket"
    );
    // The Lead's chat turn, still going.
    let chat = RunId::new();
    app.store()
        .commit(None, "system", |w| {
            runs::insert(
                w.tx,
                &runs::Run {
                    id: chat,
                    ticket: None,
                    agent: lead,
                    provider: "mock".into(),
                    model: "m".into(),
                    started_at: agenium_core::now_iso(),
                    ended_at: None,
                    outcome: None,
                    steps: 0,
                    usage: UsageSummary::default(),
                    heartbeat: None,
                },
            )
        })
        .unwrap();
    let ran = rt.tick(project).await.unwrap();
    assert!(ran.is_empty(), "no inbox run beside the chat turn: {ran:?}");
    assert_eq!(p.requests().len(), 0);
    // The turn ends: the inbox runs (here it answers with nothing and stops).
    app.store()
        .commit(None, "system", |w| {
            runs::finish(w.tx, chat, &agenium_core::now_iso(), "Completed")
        })
        .unwrap();
    p.push_text("nothing to do");
    let _ = rt.tick(project).await.unwrap();
    assert!(
        !p.requests().is_empty(),
        "the inbox ran once the turn ended"
    );
}

/// An inbox run the Lead ends with words and no call (here: the user said to wait) is not run again for the same items,
/// and its words reach the user once, not a "could not handle its inbox" after two runs (2026-11-05).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_inbox_the_lead_leaves_as_it_is_is_not_run_again() {
    let f = fixture();
    let p = Arc::new(ScriptedProvider::new("mock"));
    let (app, rt) = runtime(&f, p.clone(), Arc::new(AutoApprove));
    app.update_config(|c| c.memory.learn = false).unwrap();
    let project = app
        .create_project(&agenium_orchestrator::CreateProject {
            name: "hold".into(),
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
            "hold team",
            &agent_spec("lead", "lead", RoleKind::Lead, &[]),
            &[agent_spec("api-1", "api", RoleKind::Worker, &["rust"])],
        )
        .unwrap();
    let lead = members[0].id;
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
    for _ in 0..6 {
        p.push_text(
            "The user asked me to wait for their go, so I leave PI-1 without a ticket for now.",
        );
    }
    let _ = rt.tick(project).await.unwrap();
    let asked = p.requests().len();
    assert!(asked >= 1, "the inbox ran");
    let _ = rt.tick(project).await.unwrap();
    assert_eq!(
        p.requests().len(),
        asked,
        "the same items are not run again"
    );
    let needs = app
        .store()
        .read(|c| events::recent_of_kind(c, "NeedsUser", 10))
        .unwrap();
    let texts: Vec<String> = needs.iter().map(|e| e.payload.to_string()).collect();
    assert_eq!(
        texts
            .iter()
            .filter(|t| t.contains("left 1 inbox item"))
            .count(),
        1,
        "{texts:?}"
    );
    assert!(
        texts.iter().all(|t| !t.contains("could not handle")),
        "{texts:?}"
    );
}
