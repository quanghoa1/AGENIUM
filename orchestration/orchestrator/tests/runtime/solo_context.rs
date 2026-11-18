//! What a Solo agent's run is shown: a context that fits its window, the screenshots of an accepted ticket, a
//! closed ticket's review.

use super::*;

/// ADR-155: before a model change the user is told when the agent's context outgrows the new model's window. The
/// context is the latest run's (the last request it read), the window the catalogue's.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_smaller_window_than_the_context_is_found_before_the_change() {
    let f = fixture();
    let p = Arc::new(ScriptedProvider::new("mock"));
    let (app, rt) = runtime(&f, p, Arc::new(AutoApprove));
    let (_, agent) = solo(&app);
    assert_eq!(
        app.context_now(agent).unwrap(),
        None,
        "no run measured one yet"
    );
    std::fs::write(
        app.layout().models_dev(),
        serde_json::json!({"opencode-go": {"models": {
            "beam-free": {"limit": {"context": 1048576}},
            "iceman-contributor-free": {"limit": {"context": 262144}}}}})
        .to_string(),
    )
    .unwrap();
    let run = agenium_core::RunId::new();
    app.store()
        .commit(None, "system", |w| -> agenium_orchestrator::Result<()> {
            runs::insert(
                w.tx,
                &runs::Run {
                    id: run,
                    ticket: None,
                    agent,
                    provider: "opencode-go".into(),
                    model: "beam-free".into(),
                    started_at: agenium_core::now_iso(),
                    ended_at: Some(agenium_core::now_iso()),
                    outcome: Some("Completed".into()),
                    steps: 9,
                    usage: agenium_core::UsageSummary {
                        context_used: 550_000,
                        context_window: 1_048_576,
                        ..Default::default()
                    },
                    heartbeat: None,
                },
            )?;
            Ok(())
        })
        .unwrap();
    assert_eq!(app.context_now(agent).unwrap(), Some((550_000, 1_048_576)));
    let small = rt
        .window_for(BackendKind::Cli, "opencode-go", "iceman-contributor-free")
        .await;
    assert_eq!(small, 262_144);
    assert!(550_000 > small * 85 / 100, "the change is warned about");
    let big = rt
        .window_for(BackendKind::Cli, "opencode-go", "beam-free")
        .await;
    assert!(550_000 <= big * 85 / 100, "a larger window is not");
}

/// ADR-158: a ticket closed with screenshots in its report shows them in the chat once, without the user asking.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_accepted_tickets_screenshots_go_into_the_chat_once() {
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
        Intent::CreateTicket(new_ticket("PI-1", "Home page", &[], &[])),
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
    let root = std::path::PathBuf::from(app.project(project).unwrap().path);
    let wt = root.join(".agenium").join("worktrees").join("AGN-1");
    let png = [&[0x89, b'P', b'N', b'G', 13, 10, 26, 10][..], &[0u8; 32]].concat();
    // The work order says what the report's template asks of its extension.
    p.push(Box::new(|req| {
        let t = req.messages[0].text();
        assert!(t.contains("submit_report.extension"), "{t}");
        Ok(calls_response(
            "",
            vec![(
                "write_file",
                json!({"path": "deliverables/index.html", "content": "<h1>Hi</h1>\n"}),
            )],
        ))
    }));
    // The worker took its screenshot (as `browser` saves it), then reports with it.
    let shot = png.clone();
    p.push(Box::new(move |_req| {
        std::fs::create_dir_all(wt.join(".agenium/shots")).unwrap();
        std::fs::write(wt.join(".agenium/shots/home.png"), &shot).unwrap();
        let mut r = report("AGN-1");
        r.artifacts = vec![agenium_core::forms::ArtifactRef {
            kind: "screenshot".into(),
            reference: ".agenium/shots/home.png".into(),
        }];
        Ok(calls_response(
            "",
            vec![("submit_report", serde_json::to_value(r).unwrap())],
        ))
    }));
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
            json!({"ticket_id": "AGN-1", "reason": "All AC met, merged into main"}),
        )],
    );
    rt.run_ticket(project, &key("AGN-1")).await.unwrap();
    rt.review(project, &key("AGN-1")).await.unwrap();
    assert_eq!(
        app.ticket(project, &key("AGN-1")).unwrap().status,
        TicketStatus::Closed
    );
    let e = app.embedder();
    let mem = app.agent_store(agent, e.dim(), e.model_id()).unwrap();
    let said = |mem: &agenium_store::AgentStore| -> Vec<String> {
        mem.load_messages(&format!("chat:{project}"))
            .unwrap()
            .into_iter()
            .filter_map(|v| serde_json::from_value::<agenium_llm::Message>(v).ok())
            .map(|m| m.text())
            .filter(|t| t.contains("Screenshot of AGN-1: home.png"))
            .collect()
    };
    let lines = said(&mem);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(
        lines[0].starts_with("AGN-1 is done and accepted: Home page."),
        "{}",
        lines[0]
    );
    // Once: another look at the ticket does not say it again.
    let _ = rt.review(project, &key("AGN-1")).await;
    assert_eq!(said(&mem).len(), 1);
}

/// ADR-158 decision 31: a later ticket's work order carries what the review of a closed one said (a detail the
/// agent's memory did not keep), marked as written then; the run's page lists it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_later_ticket_is_reminded_of_a_closed_tickets_review() {
    let f = fixture();
    let p = Arc::new(ScriptedProvider::new("mock"));
    let (app, rt) = runtime(&f, p.clone(), Arc::new(AutoApprove));
    // Off by default (decision 32): the test turns it on.
    app.update_config(|c| {
        c.memory.learn = false;
        c.memory.trace_recall = true;
    })
    .unwrap();
    let (project, agent) = solo(&app);
    let lead = Actor::Lead(agent);
    let ctxp = |a: Actor| agenium_orchestrator::CallCtx {
        project,
        actor: a,
        intent_id: None,
        run: None,
    };
    app.apply(&ctxp(lead), Intent::ProposeSpec(spec())).unwrap();
    app.apply(
        &ctxp(lead),
        Intent::CreatePlan(plan(&[("PI-1", &[]), ("PI-2", &[])])),
    )
    .unwrap();
    for (item, title) in [
        ("PI-1", "Quiz submission endpoint"),
        ("PI-2", "Quiz submission retry button"),
    ] {
        app.apply(
            &ctxp(lead),
            Intent::CreateTicket(new_ticket(item, title, &[], &[])),
        )
        .unwrap();
    }
    for k in ["AGN-1", "AGN-2"] {
        app.apply(
            &ctxp(lead),
            Intent::AssignTicket(agenium_core::forms::AssignTicket {
                ticket_id: key(k),
                worker: agent,
                note: None,
            }),
        )
        .unwrap();
    }
    p.push_calls(
        "",
        vec![(
            "write_file",
            json!({"path": "deliverables/quiz.txt", "content": "quiz\n"}),
        )],
    );
    p.push_calls(
        "",
        vec![(
            "submit_report",
            serde_json::to_value(report("AGN-1")).unwrap(),
        )],
    );
    p.push_calls(
        "",
        vec![(
            "review_ticket",
            json!({"ticket_id": "AGN-1", "decision": "Approve", "ac_feedback": [{"ac_id": "AC-1",
                "feedback": "A second quiz submission answers 409 QUIZ_ALREADY_SUBMITTED, as the spec asks."}]}),
        )],
    );
    p.push_calls(
        "",
        vec![(
            "close_ticket",
            json!({"ticket_id": "AGN-1", "reason": "All AC met, merged into main"}),
        )],
    );
    rt.run_ticket(project, &key("AGN-1")).await.unwrap();
    rt.review(project, &key("AGN-1")).await.unwrap();
    assert_eq!(
        app.ticket(project, &key("AGN-1")).unwrap().status,
        TicketStatus::Closed
    );
    // Indexed once the review closed it, before any work run recalls.
    let id = app.ticket(project, &key("AGN-1")).unwrap().id;
    let store = app.trace_store(project).unwrap();
    let mut indexed = false;
    for _ in 0..100 {
        if store.meta(&format!("indexed:{id}")).unwrap().is_some() {
            indexed = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(indexed, "the closed ticket is indexed at once");
    // The next ticket of the project: its first message says what the review said.
    p.push(Box::new(|req| {
        let t = req.messages[0].text();
        assert!(t.contains("From the project's closed tickets"), "{t}");
        assert!(
            t.contains("AGN-1 review") && t.contains("QUIZ_ALREADY_SUBMITTED"),
            "{t}"
        );
        Ok(calls_response(
            "",
            vec![(
                "submit_report",
                serde_json::to_value(report("AGN-2")).unwrap(),
            )],
        ))
    }));
    let s = rt.run_ticket(project, &key("AGN-2")).await.unwrap();
    let log = app.store().read(|c| runs::log_entries(c, s.run)).unwrap();
    assert!(
        log.iter().any(|e| e.kind == "memory"
            && e.data["traces"][0]["ticket"] == "AGN-1"
            && e.data["traces"][0]["kind"] == "review"),
        "{log:?}"
    );
}
