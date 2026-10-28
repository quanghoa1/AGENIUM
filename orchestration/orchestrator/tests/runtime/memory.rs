//! Skills, memory and conversation across runs and vendors; lessons from a closed ticket.

use super::*;

/// M15-4: the skills index is in the cached system prefix, `load_skill` returns the body and the skill folder is
/// readable (not writable) through `read_file`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn skills_are_indexed_loaded_and_readable() {
    let f = fixture();
    let p = Arc::new(ScriptedProvider::new("mock"));
    let (app, rt) = runtime(&f, p.clone(), Arc::new(AutoApprove));
    let (project, agent) = solo(&app);
    let dir = app.layout().roles().join("solo/skills/greeting-style");
    std::fs::create_dir_all(dir.join("references")).unwrap();
    std::fs::write(
        dir.join("SKILL.md"),
        "---\nname: greeting-style\ndescription: How greeting files are written.\n---\nGreetings end with a newline. Tone: [tone](references/tone.md).\n",
    )
    .unwrap();
    std::fs::write(dir.join("references/tone.md"), "Be warm.\n").unwrap();
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

    // ADR-91: a lesson that held in two contexts makes the skill-writing tools available, with a hint naming it.
    let e = app.embedder();
    let mem = app.agent_store(agent, e.dim(), e.model_id()).unwrap();
    let lesson = mem
        .add(
            &agenium_store::NewFact {
                kind: agenium_store::FactKind::Lesson,
                cue: Some("writing greeting files".into()),
                ..agenium_store::NewFact::fact("Greeting files end with a newline.", "ticket:AGN-0")
            },
            None,
        )
        .unwrap();
    mem.validate_in(lesson, "p/ticket:AGN-0").unwrap();
    mem.validate_in(lesson, "p/ticket:AGN-9").unwrap();
    let ready = format!("#{lesson}");

    let tone = dir.join("references/tone.md").display().to_string();
    let tone2 = tone.clone();
    p.push(Box::new(move |req| {
        let offered = |n: &str| req.tools.iter().any(|t| t.name == n);
        assert!(offered("propose_new_skill") && offered("propose_skill_update"));
        let first = req.messages[0].text();
        assert!(
            first.contains("Lessons that held in two or more contexts") && first.contains(&ready),
            "{first}"
        );
        assert!(req.system[1].cache);
        assert!(
            req.system[1]
                .text
                .contains("- greeting-style: How greeting files are written."),
            "{}",
            req.system[1].text
        );
        assert!(req.tools.iter().any(|t| t.name == "load_skill"));
        Ok(calls_response(
            "",
            vec![
                ("load_skill", json!({"name": "greeting-style"})),
                ("load_skill", json!({"name": "nope"})),
            ],
        ))
    }));
    p.push(Box::new(move |req| {
        let results: Vec<(String, bool)> = req
            .messages
            .iter()
            .flat_map(|m| m.parts.iter())
            .filter_map(|p| match p {
                agenium_llm::Part::ToolResult {
                    content, is_error, ..
                } => Some((content.clone(), *is_error)),
                _ => None,
            })
            .collect();
        assert!(
            results[0].0.contains("# Skill: greeting-style")
                && results[0].0.contains("Greetings end with a newline")
                && results[0].0.contains("references/tone.md")
                && !results[0].1,
            "{results:?}"
        );
        assert!(results[1].1 && results[1].0.contains("available: greeting-style"));
        Ok(calls_response(
            "",
            vec![
                ("read_file", json!({"path": tone2})),
                ("write_file", json!({"path": tone2, "content": "x"})),
            ],
        ))
    }));
    p.push(Box::new(|req| {
        let last = req.messages.last().unwrap();
        let r: Vec<(String, bool)> = last
            .parts
            .iter()
            .filter_map(|p| match p {
                agenium_llm::Part::ToolResult {
                    content, is_error, ..
                } => Some((content.clone(), *is_error)),
                _ => None,
            })
            .collect();
        assert!(r[0].0.contains("Be warm.") && !r[0].1, "{r:?}");
        assert!(r[1].1, "skill folders are read-only: {r:?}");
        Ok(calls_response(
            "",
            vec![(
                "write_file",
                json!({"path": "deliverables/hello.txt", "content": "hello\n"}),
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
    assert_eq!(std::fs::read_to_string(&tone).unwrap(), "Be warm.\n");
}

/// M15-1: switching an agent to another vendor keeps its memory, skills and the stored conversation; the run
/// continues on the new provider with the new model.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn vendor_switch_keeps_memory_skills_and_conversation() {
    let f = fixture();
    let old = Arc::new(ScriptedProvider::new("vendor-a"));
    let new = Arc::new(ScriptedProvider::new("vendor-b"));
    let app = Arc::new(open_app(
        &f.root.parent().unwrap().join("rt-root"),
        f.keyring.clone(),
        None,
    ));
    let rt = Runtime::new(
        app.clone(),
        Arc::new(StaticProviders(vec![
            ("vendor-a".into(), old.clone() as Arc<dyn Provider>),
            ("vendor-b".into(), new.clone() as Arc<dyn Provider>),
        ])),
        Arc::new(AutoApprove),
    );
    let p = app
        .create_project(&agenium_orchestrator::CreateProject {
            name: "switch".into(),
            path: None,
            key_prefix: None,
            template: None,
            folder: None,
            repo: None,
        })
        .unwrap();
    let mut spec_a = agent_spec("ada", "solo", RoleKind::Lead, &[]);
    spec_a.model.provider = "vendor-a".into();
    spec_a.model.model = "model-a".into();
    let (_, a) = app.create_solo(p.id, &spec_a).unwrap();
    let project = p.id;
    let dir = app.layout().agent_dir(&a.id).join("skills/house-style");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("SKILL.md"),
        "---\nname: house-style\ndescription: House writing style.\n---\nWrite short sentences.\n",
    )
    .unwrap();
    let e = app.embedder();
    let mem = app.agent_store(a.id, e.dim(), e.model_id()).unwrap();
    mem.add(
        &agenium_store::NewFact::fact("Greeting files end with a newline", "ticket:OLD-1"),
        Some(
            &e.embed(&["Greeting files end with a newline".into()])
                .unwrap()[0],
        ),
    )
    .unwrap();
    let lead = Actor::Lead(a.id);
    let ctxp = |a: Actor| agenium_orchestrator::CallCtx {
        project,
        actor: a,
        intent_id: None,
        run: None,
    };
    app.apply(&ctxp(lead), Intent::ProposeSpec(spec())).unwrap();
    app.apply(&ctxp(lead), Intent::CreatePlan(plan(&[("PI-1", &[])])))
        .unwrap();
    let mut nt = new_ticket("PI-1", "Write the greeting file", &[], &[]);
    nt.description = "Write a greeting file that ends with a newline.".into();
    app.apply(&ctxp(lead), Intent::CreateTicket(nt)).unwrap();
    app.apply(
        &ctxp(lead),
        Intent::AssignTicket(agenium_core::forms::AssignTicket {
            ticket_id: key("AGN-1"),
            worker: a.id,
            note: None,
        }),
    )
    .unwrap();

    // Vendor A: thinks (vendor-specific block), asks a blocking question -> parked.
    old.push(Box::new(|req| {
        assert_eq!(req.model, "model-a");
        let mut r = calls_response(
            "",
            vec![(
                "ask_question",
                json!({"ticket_id": "AGN-1", "category": "Requirement", "question": "Which greeting text?", "blocking": true}),
            )],
        );
        r.message.parts.insert(
            0,
            agenium_llm::Part::Thinking {
                text: "The user wants a greeting.".into(),
                signature: Some("sig-a".into()),
                provider: "anthropic".into(),
                redacted: false,
                model: None,
            },
        );
        Ok(r)
    }));
    assert_eq!(
        rt.run_ticket(project, &key("AGN-1")).await.unwrap().outcome,
        "Parked"
    );

    // The user answers and switches the agent to vendor B.
    let q = app
        .store()
        .read(|c| {
            agenium_store::core::comments::open_blocking(
                c,
                app.ticket(project, &key("AGN-1")).unwrap().id,
            )
        })
        .unwrap();
    app.user_answer(project, &key("AGN-1"), q[0].id, "Say hello.", None)
        .unwrap();
    app.set_agent_model(
        a.id,
        &ModelBinding {
            provider: "vendor-b".into(),
            model: "model-b".into(),
            effort: Effort::MEDIUM,
            max_tokens: None,
        },
    )
    .unwrap();
    new.push(Box::new(|req| {
        assert_eq!(req.model, "model-b");
        let sys: String = req.system.iter().map(|b| b.text.as_str()).collect();
        assert!(
            sys.contains("- house-style: House writing style."),
            "skills survive"
        );
        assert!(req.tools.iter().any(|t| t.name == "load_skill"));
        let texts: String = req
            .messages
            .iter()
            .map(|m| m.text())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            texts.contains("Greeting files end with a newline"),
            "memory survives: {texts}"
        );
        assert!(texts.contains("Say hello."), "the answer is appended");
        assert!(
            req.messages
                .iter()
                .any(|m| m.tool_calls().iter().any(|c| c.name == "ask_question")),
            "the conversation from vendor A is kept"
        );
        // The stored history keeps vendor A's thinking, but it is never sent to another model: its signature is
        // valid for model-a only (ADR-101).
        assert!(!req
            .messages
            .iter()
            .flat_map(|m| m.parts.iter())
            .any(|p| matches!(p, agenium_llm::Part::Thinking { .. })));
        Ok(calls_response(
            "",
            vec![(
                "write_file",
                json!({"path": "deliverables/hello.txt", "content": "hello\n"}),
            )],
        ))
    }));
    new.push_calls(
        "",
        vec![(
            "submit_report",
            serde_json::to_value(report("AGN-1")).unwrap(),
        )],
    );
    let s = rt.run_ticket(project, &key("AGN-1")).await.unwrap();
    assert_eq!(s.outcome, "Completed");
    assert_eq!(old.remaining(), 0);
    assert_eq!(new.remaining(), 0);
    assert_eq!(
        mem.count_live().unwrap(),
        1,
        "memory untouched by the switch"
    );
    let runs_ = app
        .store()
        .read(|c| runs::for_ticket(c, app.ticket(project, &key("AGN-1")).unwrap().id))
        .unwrap();
    assert_eq!(runs_.last().unwrap().provider, "vendor-b");
}

/// M15-2: closing a ticket runs the memory pipeline for the assignee once; the next ticket's work order recalls
/// the learned fact.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn closing_a_ticket_teaches_the_assignee() {
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
        Intent::CreatePlan(plan(&[("PI-1", &[]), ("PI-2", &[])])),
    )
    .unwrap();
    for (item, title) in [
        ("PI-1", "Write the greeting"),
        ("PI-2", "Write the farewell"),
    ] {
        app.apply(
            &ctxp(lead),
            Intent::CreateTicket(new_ticket(item, title, &[], &[])),
        )
        .unwrap();
    }
    app.apply(
        &ctxp(lead),
        Intent::AssignTicket(agenium_core::forms::AssignTicket {
            ticket_id: key("AGN-1"),
            worker: agent,
            note: None,
        }),
    )
    .unwrap();

    // Work, then review: approve (merge) and close.
    p.push_calls(
        "",
        vec![(
            "write_file",
            json!({"path": "deliverables/hello.txt", "content": "hello\n"}),
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
    // Memory pipeline: the extraction sees the work conversation (no thinking), memory is empty so no decision call.
    p.push(Box::new(|req| {
        let t = req.messages[0].text();
        assert!(t.contains("Episode ticket:AGN-1") && t.contains("[tool call] write_file"), "{t}");
        Ok(calls_response(
            "{\"facts\": [{\"text\": \"Greeting files in this project end with a newline.\", \"kind\": \"fact\"}]}",
            vec![],
        ))
    }));
    assert_eq!(
        rt.run_ticket(project, &key("AGN-1")).await.unwrap().outcome,
        "Completed"
    );
    let s = rt.review(project, &key("AGN-1")).await.unwrap();
    assert_eq!(
        app.ticket(project, &key("AGN-1")).unwrap().status,
        TicketStatus::Closed
    );
    let e = app.embedder();
    let mem = app.agent_store(agent, e.dim(), e.model_id()).unwrap();
    let facts = mem.live(10).unwrap();
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].source, "ticket:AGN-1");
    let log = app.store().read(|c| runs::log_entries(c, s.run)).unwrap();
    assert!(log
        .iter()
        .any(|e| e.kind == "memory" && e.data["added"] == 1));
    // The extraction's tokens count like a run's, as the assignee's Memory run.
    let memory: Vec<runs::Run> = app
        .store()
        .read(|c| runs::for_agent(c, agent, 50))
        .unwrap()
        .into_iter()
        .filter(|r| r.outcome.as_deref() == Some("Memory"))
        .collect();
    assert_eq!(memory.len(), 1, "{memory:?}");
    assert!(memory[0].usage.tokens_in > 0 && memory[0].ticket.is_none());
    // Once per ticket state.
    assert!(rt
        .learn_from_ticket(project, &key("AGN-1"), s.run)
        .await
        .unwrap()
        .is_none());

    // The next ticket recalls it.
    app.apply(
        &ctxp(lead),
        Intent::AssignTicket(agenium_core::forms::AssignTicket {
            ticket_id: key("AGN-2"),
            worker: agent,
            note: None,
        }),
    )
    .unwrap();
    p.push(Box::new(|req| {
        let t = req.messages[0].text();
        assert!(t.contains("end with a newline"), "{t}");
        Err(agenium_llm::LlmError::Unsupported("stop here".into()))
    }));
    let _ = rt.run_ticket(project, &key("AGN-2")).await;
    assert_eq!(p.remaining(), 0);
}
