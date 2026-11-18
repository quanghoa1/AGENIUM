//! The chat with a team's Lead: learned from once quiet, summarised when long, messages that wait for its runs,
//! and inbox runs that do not overlap it.

use super::*;

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
