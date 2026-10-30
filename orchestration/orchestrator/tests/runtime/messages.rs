//! A run's messages and context: the user's messages to a Lead, continuations, the review diff, cancelling, images.

use super::*;

/// ADR-149: when the Lead's chat turn and another of its runs overlap, the user's messages wait in the queue until no
/// run of the Lead goes on, then one chat turn reads them all; none is lost or misrouted.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn overlapping_lead_runs_keep_the_users_messages() {
    let f = fixture();
    let p = Arc::new(ScriptedProvider::new("mock"));
    let (app, rt) = runtime(&f, p.clone(), Arc::new(AutoApprove));
    let rt = Arc::new(rt);
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
    // The chat turn's first step runs an inbox run of the same Lead to its end (as the scheduler can meanwhile).
    let r = rt.clone();
    p.push(Box::new(move |_| {
        let inbox = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(r.inbox(project))
        });
        assert!(inbox.unwrap().is_some());
        assert!(
            r.steer(project, "And keep it small.").unwrap(),
            "the chat turn still goes on: the message waits"
        );
        Ok(calls_response("", vec![("list_dir", json!({}))]))
    }));
    let r = rt.clone();
    p.push(Box::new(move |_| {
        assert!(r.steer(project, "Use Rust.").unwrap());
        Err(agenium_llm::LlmError::Unsupported(
            "the inbox run stops here".into(),
        ))
    }));
    p.push(Box::new(|req| {
        let all: String = req.messages.iter().map(|m| m.text()).collect();
        assert!(
            !all.contains("Use Rust.") && !all.contains("keep it small"),
            "{all}"
        );
        Ok(calls_response("Noted.", vec![]))
    }));
    let s = rt.chat(project, "Plan an API.").await.unwrap();
    assert_eq!(s.outcome, "Completed");
    assert_eq!(app.undelivered_lead_messages(project).unwrap().len(), 2);
    // No run of the Lead goes on: its next chat turn reads both, oldest first ("Use Rust." was sent during the inbox
    // run the chat turn's first step ran).
    p.push(Box::new(|req| {
        let last = req.messages.last().unwrap().text();
        let rust = last.find("Use Rust.").unwrap();
        let small = last.find("And keep it small.").unwrap();
        assert!(rust < small, "{last}");
        Ok(calls_response("Small, in Rust.", vec![]))
    }));
    let left = rt.work(project).await.unwrap();
    assert!(
        left.iter()
            .any(|r| r.kind == "chat" && r.text == "Small, in Rust."),
        "{left:?}"
    );
    assert!(app.undelivered_lead_messages(project).unwrap().is_empty());
    assert_eq!(p.remaining(), 0);
}

/// ADR-149: a message queued while the Lead works is answered once the project is not running yet (Agreeing); one
/// whose turn fails goes back to the queue, is tried twice (not every round), then waits for the user's next message;
/// a queued message is taken back before the Lead reads it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queued_messages_are_answered_kept_or_taken_back() {
    let f = fixture();
    let p = Arc::new(ScriptedProvider::new("mock"));
    let (app, rt) = runtime(&f, p.clone(), Arc::new(AutoApprove));
    let rt = Arc::new(rt);
    let (project, agent) = solo(&app);
    let while_working = |text: &'static str| -> agenium_llm::mock::Responder {
        let r = rt.clone();
        Box::new(move |_| {
            assert!(r.steer(project, text).unwrap());
            Ok(calls_response("Noted.", vec![]))
        })
    };
    p.push(while_working("And add a README."));
    rt.chat(project, "Plan an API.").await.unwrap();
    use agenium_orchestrator::views::RunState;
    assert_eq!(app.run_state(project).unwrap(), RunState::Agreeing);
    p.push(Box::new(|req| {
        let last = req.messages.last().unwrap().text();
        assert!(last.contains("And add a README."), "{last}");
        Ok(calls_response("Will do.", vec![]))
    }));
    let runs = rt.work(project).await.unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].text, "Will do.");
    assert!(rt.work(project).await.unwrap().is_empty(), "answered once");

    // Taken back before the Lead reads it: no turn.
    p.push(while_working("Never mind this."));
    rt.chat(project, "Thanks.").await.unwrap();
    let q = app.undelivered_lead_messages(project).unwrap();
    assert_eq!(q.len(), 1);
    app.drop_lead_message(project, q[0].id).unwrap();
    assert!(rt.work(project).await.unwrap().is_empty());

    // The turn that would answer fails: the message goes back to the queue, tried twice, then waits for the user.
    p.push(while_working("Also a licence."));
    rt.chat(project, "Good.").await.unwrap();
    app.set_agent_backend(agent, BackendKind::Cli, &devin())
        .unwrap();
    for _ in 0..3 {
        assert!(rt.work(project).await.unwrap().is_empty());
    }
    let failed = app
        .store()
        .read(|c| runs::for_agent(c, agent, 50))
        .unwrap()
        .iter()
        .filter(|r| {
            r.outcome
                .as_deref()
                .is_some_and(|o| o.starts_with("Error:"))
        })
        .count();
    assert_eq!(failed, 2);
    assert_eq!(app.undelivered_lead_messages(project).unwrap().len(), 1);
    app.set_agent_backend(agent, BackendKind::Api, &model())
        .unwrap();
    p.push(Box::new(|req| {
        let last = req.messages.last().unwrap().text();
        assert!(
            last.contains("Also a licence.") && last.contains("One more thing."),
            "{last}"
        );
        Ok(calls_response("Sure.", vec![]))
    }));
    rt.chat(project, "One more thing.").await.unwrap();
    assert_eq!(p.remaining(), 0);
}

/// A continuation run is shown what was posted on its ticket while its previous run went on (it never saw those
/// comments), not only what came after that run ended.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_continuation_sees_comments_posted_during_the_last_run() {
    let f = fixture();
    let p = Arc::new(ScriptedProvider::new("mock"));
    let (app, rt) = runtime(&f, p.clone(), Arc::new(AutoApprove));
    let (project, _) = solo_with_tickets(&app, &["PI-1"]);
    let a = app.clone();
    p.push(Box::new(move |_| {
        a.user_comment(project, &key("AGN-1"), "Use tabs, not spaces.")
            .unwrap();
        Ok(calls_response("thinking out loud", vec![]))
    }));
    p.push_text("thinking out loud");
    p.push_text("thinking out loud");
    let s = rt.run_ticket(project, &key("AGN-1")).await.unwrap();
    assert_eq!(s.outcome, "Stopped:no_tool_call");
    p.push(Box::new(|req| {
        let last = req.messages.last().unwrap().text();
        assert!(
            last.contains("New on the ticket since your last step")
                && last.contains("Use tabs, not spaces."),
            "{last}"
        );
        Ok(calls_response(
            "",
            vec![(
                "submit_report",
                serde_json::to_value(report("AGN-1")).unwrap(),
            )],
        ))
    }));
    let s = rt.run_ticket(project, &key("AGN-1")).await.unwrap();
    assert_eq!(s.outcome, "Completed");
    assert_eq!(p.remaining(), 0);
}

/// A diff that cannot be read reaches the reviewer as such, never as "no code changes".
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_reviewer_is_told_when_the_diff_cannot_be_read() {
    use agenium_store::core::workspaces;
    let f = fixture();
    let p = Arc::new(ScriptedProvider::new("mock"));
    let (app, rt) = runtime(&f, p.clone(), Arc::new(AutoApprove));
    let (project, agent) = solo_with_tickets(&app, &["PI-1"]);
    resolve(&app, project, agent, "AGN-1");
    let t = app.ticket(project, &key("AGN-1")).unwrap();
    app.store()
        .commit(None, "test", |w| {
            let ws = workspaces::get(w.tx, t.id)?.unwrap();
            workspaces::upsert(
                w.tx,
                &workspaces::TicketWorkspace {
                    base_sha: "0123456789012345678901234567890123456789".into(),
                    ..ws
                },
            )
        })
        .unwrap();
    p.push(Box::new(|req| {
        let task = req.messages[0].text();
        assert!(
            task.contains("(the diff could not be read:") && !task.contains("(no code changes)"),
            "{task}"
        );
        Err(agenium_llm::LlmError::Unsupported("enough".into()))
    }));
    rt.review(project, &key("AGN-1")).await.unwrap();
    assert_eq!(p.remaining(), 0);
}

/// A chat turn's run keeps only its own messages: the history it replays lives in the chat session already.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_chat_turn_stores_only_its_own_messages() {
    let f = fixture();
    let p = Arc::new(ScriptedProvider::new("mock"));
    let (app, rt) = runtime(&f, p.clone(), Arc::new(AutoApprove));
    let (project, agent) = solo(&app);
    let e = app.embedder();
    let mem = app.agent_store(agent, e.dim(), e.model_id()).unwrap();
    let mut sizes = vec![];
    for (ask, answer) in [
        ("Hello.", "Hi."),
        ("Plan an API.", "Sure."),
        ("Thanks.", "Welcome."),
    ] {
        p.push_text(answer);
        let s = rt.chat(project, ask).await.unwrap();
        sizes.push(mem.load_messages(&s.run.to_string()).unwrap().len());
    }
    assert_eq!(sizes, vec![2, 2, 2], "the user's message and the reply");
    assert_eq!(
        mem.load_messages(&format!("chat:{project}")).unwrap().len(),
        6
    );
}

/// A ticket cancelled while its run works stops the run before its next step, and its agent stays busy until that run
/// has ended (the scheduler could otherwise start a second run of the same agent beside it).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_ticket_cancelled_under_its_run_stops_it() {
    let f = fixture();
    let p = Arc::new(ScriptedProvider::new("mock"));
    let (app, rt) = runtime(&f, p.clone(), Arc::new(AutoApprove));
    app.update_config(|c| c.memory.learn = false).unwrap();
    let (project, agent) = solo_with_tickets(&app, &["PI-1"]);
    let a = app.clone();
    p.push(Box::new(move |_| {
        a.apply(
            &agenium_orchestrator::CallCtx {
                project,
                actor: Actor::Lead(agent),
                intent_id: None,
                run: None,
            },
            Intent::CancelTicket(agenium_core::forms::CancelTicket {
                ticket_id: key("AGN-1"),
                reason: "Out of scope now".into(),
                replaced_by: None,
            }),
        )
        .unwrap();
        assert_eq!(
            a.agent(agent).unwrap().status,
            AgentStatus::Working,
            "busy while its run is live"
        );
        // Longer than the runtime's watch period.
        tokio::task::block_in_place(|| std::thread::sleep(std::time::Duration::from_millis(2600)));
        Ok(calls_response("", vec![("list_dir", json!({}))]))
    }));
    p.push_calls("", vec![("list_dir", json!({}))]);
    let s = rt.run_ticket(project, &key("AGN-1")).await.unwrap();
    assert_eq!(s.outcome, "Cancelled");
    assert_eq!(p.remaining(), 1, "no model call after the cancel");
    assert_eq!(app.agent(agent).unwrap().status, AgentStatus::Idle);
}

/// `view_image` is offered only to a model that takes images (an image sent to a text model fails the call); here the
/// provider's hand-set capabilities say so.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn view_image_is_offered_to_a_model_that_takes_images_only() {
    for vision in [false, true] {
        let f = fixture();
        let p = Arc::new(ScriptedProvider::new("mock"));
        let (app, rt) = runtime(&f, p.clone(), Arc::new(AutoApprove));
        app.update_config(|c| {
            c.memory.learn = false;
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
                    tools: None,
                    vision: Some(vision),
                    context_window: None,
                }],
            })
        })
        .unwrap();
        let (project, _) = solo_with_tickets(&app, &["PI-1"]);
        p.push_calls(
            "",
            vec![(
                "submit_report",
                serde_json::to_value(report("AGN-1")).unwrap(),
            )],
        );
        assert_eq!(
            rt.run_ticket(project, &key("AGN-1")).await.unwrap().outcome,
            "Completed"
        );
        let offered = p.requests()[0].tools.iter().any(|t| t.name == "view_image");
        assert_eq!(offered, vision);
    }
}

/// ADR-129: a work run offers the background companions of `shell`; a review run of an API agent, which has no
/// `shell`, offers neither (a Lead called `shell_output` with guessed job ids).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn background_shell_tools_come_only_with_shell() {
    let f = fixture();
    let p = Arc::new(ScriptedProvider::new("mock"));
    let (app, rt) = runtime(&f, p.clone(), Arc::new(AutoApprove));
    app.update_config(|c| c.memory.learn = false).unwrap();
    let (project, _) = solo_with_tickets(&app, &["PI-1"]);
    p.push_calls(
        "",
        vec![(
            "submit_report",
            serde_json::to_value(report("AGN-1")).unwrap(),
        )],
    );
    rt.run_ticket(project, &key("AGN-1")).await.unwrap();
    let names = |i: usize| -> Vec<String> {
        p.requests()[i]
            .tools
            .iter()
            .map(|t| t.name.clone())
            .collect()
    };
    let work = names(0);
    for t in ["shell", "shell_output", "shell_stop"] {
        assert!(work.contains(&t.to_owned()), "{t}: {work:?}");
    }
    p.push_text("Looks fine.");
    let _ = rt.review(project, &key("AGN-1")).await;
    let review = names(p.requests().len() - 1);
    assert!(!review.contains(&"shell".to_owned()), "{review:?}");
    assert!(
        !review
            .iter()
            .any(|n| n == "shell_output" || n == "shell_stop"),
        "{review:?}"
    );
}

/// ADR-147: a message to the Lead is stored before it is acknowledged. One whose turn never ran (the app stopped
/// first) reaches the Lead's next turn after a restart, once; a delivered one is not sent again.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_message_to_the_lead_survives_a_stop_before_its_turn() {
    let f = fixture();
    let p = Arc::new(ScriptedProvider::new("mock"));
    let (app, rt) = runtime(&f, p.clone(), Arc::new(AutoApprove));
    app.update_config(|c| c.memory.learn = false).unwrap();
    let (project, _, _) = team_project(&app, "duo");

    // A chat turn: its message is delivered.
    p.push_text("Hello.");
    rt.chat(project, "Hi, plan an API.").await.unwrap();
    assert!(app.undelivered_lead_messages(project).unwrap().is_empty());

    // An answer is acknowledged (stored), then the app stops before the Lead's turn.
    app.post_lead_message(project, "My answer: use Postgres.")
        .unwrap();
    drop(rt);
    std::thread::sleep(std::time::Duration::from_millis(20));
    let rt = Runtime::new(
        app.clone(),
        Arc::new(StaticProviders(vec![(
            "mock".into(),
            p.clone() as Arc<dyn Provider>,
        )])),
        Arc::new(AutoApprove),
    );
    p.push(Box::new(|req| {
        let all: String = req.messages.iter().map(|m| m.text()).collect();
        assert!(all.contains("use Postgres"), "{all}");
        Ok(calls_response("Noted: Postgres.", vec![]))
    }));
    let out = rt.work(project).await.unwrap();
    assert_eq!(out.len(), 1, "the Lead's turn ran on it");
    assert!(app.undelivered_lead_messages(project).unwrap().is_empty());
    assert!(rt.work(project).await.unwrap().is_empty(), "once");
    assert_eq!(p.remaining(), 0);
}
