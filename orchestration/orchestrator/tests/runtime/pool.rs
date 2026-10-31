//! The shared lesson pool weighed by the work it misled (audit M3, 2026-10-31): a ticket run is shown a pool lesson,
//! the review reopens the work, the learning names the pool lesson; in a second context it leaves the pool.

use super::*;

/// The id the extraction prompt gives a shared lesson, from its "MEMORIES SHOWN" line.
fn shared_id_in(prompt: &str, text: &str) -> Option<i64> {
    prompt
        .lines()
        .find(|l| l.contains("(a shared lesson from another agent)") && l.contains(text))?
        .strip_prefix("id=")?
        .split(':')
        .next()?
        .parse()
        .ok()
}

/// One ticket worked (shown the pool lesson) and reopened by the review, whose learning names the pool lesson.
async fn worked_and_reopened(
    p: &Arc<ScriptedProvider>,
    rt: &Runtime,
    app: &agenium_orchestrator::App,
    project: ProjectId,
    k: &str,
    pooled: &'static str,
) -> RunId {
    p.push(Box::new(move |req| {
        let t = req.messages[0].text();
        assert!(t.contains(pooled), "the pool lesson is recalled: {t}");
        Ok(calls_response(
            "",
            vec![(
                "write_file",
                json!({"path": "deliverables/hello.txt", "content": "HELLO\n"}),
            )],
        ))
    }));
    p.push_calls(
        "",
        vec![("submit_report", serde_json::to_value(report(k)).unwrap())],
    );
    assert_eq!(
        rt.run_ticket(project, &key(k)).await.unwrap().outcome,
        "Completed"
    );
    p.push_calls(
        "",
        vec![(
            "review_ticket",
            json!({"ticket_id": k, "decision": "Reopen",
                   "ac_feedback": [{"ac_id": "AC-1", "feedback": "hello.txt must say hello in lower case"}]}),
        )],
    );
    let ticket = k.to_owned();
    p.push(Box::new(move |req| {
        let t = req.messages[0].text();
        assert!(t.contains(&format!("Episode ticket:{ticket}")), "{t}");
        let id = shared_id_in(&t, pooled)
            .unwrap_or_else(|| panic!("the pool lesson is shown to the learning: {t}"));
        Ok(calls_response(
            &json!({"facts": [], "misleading": [id]}).to_string(),
            vec![],
        ))
    }));
    let s = rt.review(project, &key(k)).await.unwrap();
    assert_eq!(
        app.ticket(project, &key(k)).unwrap().status,
        TicketStatus::Reopened
    );
    s.run
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_pool_lesson_that_misleads_in_two_contexts_leaves_the_pool() {
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
    app.apply(
        &ctxp(lead),
        Intent::CreatePlan(plan(&[("PI-1", &[]), ("PI-2", &[]), ("PI-3", &[])])),
    )
    .unwrap();
    for item in ["PI-1", "PI-2", "PI-3"] {
        app.apply(
            &ctxp(lead),
            Intent::CreateTicket(new_ticket(item, "Write the greeting", &[], &[])),
        )
        .unwrap();
    }
    for k in ["AGN-1", "AGN-2", "AGN-3"] {
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
    let e = app.embedder();
    let lesson = |text: &str, source: &str, confidence: f64| agenium_store::NewFact {
        kind: agenium_store::FactKind::Lesson,
        cue: Some("writing greeting files".into()),
        confidence,
        ..agenium_store::NewFact::fact(text, source)
    };
    let vector = |text: &str| e.embed(&[text.to_owned()]).unwrap().pop();

    // Another agent's lesson, copied into the pool as sharing copies it.
    let pooled = "Greeting files are written in upper case letters.";
    let owner = AgentId::new();
    let owner_mem = app.agent_store(owner, e.dim(), e.model_id()).unwrap();
    let original = owner_mem
        .add(
            &lesson(pooled, "ticket:OTHER-1", 0.9),
            vector(pooled).as_deref(),
        )
        .unwrap();
    let pool = app.pool_memory().unwrap();
    let copy = pool
        .add(
            &lesson(pooled, &format!("shared:{owner}:{original}"), 0.6),
            vector(pooled).as_deref(),
        )
        .unwrap();
    // The agent's own lesson, shown beside it: the pool's blame must not reach it.
    let mem = app.agent_store(agent, e.dim(), e.model_id()).unwrap();
    let own_text = "Greeting files end with a newline.";
    let own = mem
        .add(
            &lesson(own_text, "ticket:AGN-0", 0.7),
            vector(own_text).as_deref(),
        )
        .unwrap();

    // First context: the copy loses confidence and stays.
    let review1 = worked_and_reopened(&p, &rt, &app, project, "AGN-1", pooled).await;
    let c = pool.get(copy).unwrap();
    assert!(c.is_live(), "one context is not enough");
    assert!((c.confidence - 0.45).abs() < 1e-9, "{}", c.confidence);
    let log = app.store().read(|c| runs::log_entries(c, review1)).unwrap();
    assert!(
        log.iter()
            .any(|e| e.kind == "memory" && e.data["misleading_shared"] == json!([copy])),
        "{log:?}"
    );
    assert!(!log.iter().any(|e| e.data.get("pool_dropped").is_some()));

    // Second context: it leaves the pool, and its text is marked so sharing never brings it back.
    let review2 = worked_and_reopened(&p, &rt, &app, project, "AGN-2", pooled).await;
    assert!(
        !pool.get(copy).unwrap().is_live(),
        "dropped after two contexts"
    );
    assert!(pool
        .meta(&format!("pool-dropped:{}", pooled.to_lowercase()))
        .unwrap()
        .is_some());
    let log = app.store().read(|c| runs::log_entries(c, review2)).unwrap();
    assert!(
        log.iter()
            .any(|e| e.kind == "memory" && e.data["pool_dropped"] == json!([pooled])),
        "{log:?}"
    );

    // Only the pool changed: the lesson it was copied from and the agent's own lesson are as they were.
    let o = owner_mem.get(original).unwrap();
    assert!(o.is_live() && (o.confidence - 0.9).abs() < 1e-9);
    let m = mem.get(own).unwrap();
    assert!(m.is_live() && (m.confidence - 0.7).abs() < 1e-9, "{m:?}");

    // The next ticket is no longer shown it.
    p.push(Box::new(move |req| {
        let t = req.messages[0].text();
        assert!(!t.contains(pooled) && t.contains(own_text), "{t}");
        Err(agenium_llm::LlmError::Unsupported("stop here".into()))
    }));
    let _ = rt.run_ticket(project, &key("AGN-3")).await;
    assert_eq!(p.remaining(), 0);
}
