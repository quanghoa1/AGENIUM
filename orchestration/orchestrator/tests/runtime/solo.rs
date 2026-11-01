//! A Solo agent end to end: a request to a merged ticket, a project from its spec, sandboxed and denied commands.

use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn solo_agent_takes_a_request_to_a_closed_merged_ticket() {
    let f = fixture();
    let p = Arc::new(ScriptedProvider::new("mock"));
    let (app, rt) = runtime(&f, p.clone(), Arc::new(AutoApprove));
    let (project, agent) = solo(&app);

    // --- chat: discovery -> spec -> plan -> ticket -> assign (all English below the Lead)
    p.push_calls(
        "I will plan this.",
        vec![("propose_spec", serde_json::to_value(spec()).unwrap())],
    );
    p.push_calls(
        "",
        vec![(
            "create_plan",
            serde_json::to_value(plan(&[("PI-1", &[])])).unwrap(),
        )],
    );
    p.push_calls(
        "",
        vec![(
            "create_ticket",
            serde_json::to_value(new_ticket(
                "PI-1",
                "Write hello file",
                &[],
                &["deliverables/**"],
            ))
            .unwrap(),
        )],
    );
    p.push_calls(
        "",
        vec![(
            "assign_ticket",
            json!({"ticket_id": "AGN-1", "worker": agent.to_string()}),
        )],
    );
    p.push_text("Đã tạo ticket AGN-1 và tôi sẽ làm ngay.");
    let chat = rt.chat(project, "Hãy thêm file chào hỏi").await.unwrap();
    assert_eq!(chat.outcome, "Completed");
    assert_eq!(chat.text, "Đã tạo ticket AGN-1 và tôi sẽ làm ngay.");
    let t = app.ticket(project, &key("AGN-1")).unwrap();
    assert_eq!(
        (t.status, t.assignee),
        (TicketStatus::Assigned, Some(agent))
    );

    // --- work run: write, verify with an approved command, submit
    p.push_calls(
        "",
        vec![(
            "write_file",
            json!({"path": "deliverables/hello.txt", "content": "hello\n"}),
        )],
    );
    p.push_calls(
        "",
        vec![("shell", json!({"command": "cat deliverables/hello.txt"}))],
    );
    p.push(Box::new(|req| {
        // The shell result reached the model.
        let last = req.messages.last().unwrap();
        let ok = matches!(&last.parts[0], agenium_llm::Part::ToolResult { content, .. } if content.contains("hello"));
        assert!(ok, "shell output is returned to the model");
        Ok(calls_response("", vec![("submit_report", serde_json::to_value(report("AGN-1")).unwrap())]))
    }));
    // --- review run (Solo reviews itself): approve -> merge queue -> close
    p.push_calls(
        "",
        vec![(
            "review_ticket",
            json!({"ticket_id": "AGN-1", "decision": "Approve"}),
        )],
    );
    p.push(Box::new(|req| {
        let last = req.messages.last().unwrap();
        let merged = matches!(&last.parts[0], agenium_llm::Part::ToolResult { content, .. } if content.contains("\"merged\":true") && content.contains("call close_ticket"));
        assert!(merged, "approval result says the merge ran");
        // A model that stops talking after the approval is nudged, not ended (CLI eval regression).
        Ok(calls_response("Approved; the merge is queued.", vec![]))
    }));
    p.push(Box::new(|req| {
        let last = req.messages.last().unwrap();
        let nudged = matches!(&last.parts[0], agenium_llm::Part::Text { text } if text.contains("does not finish the review"));
        assert!(nudged, "review nudge");
        Ok(calls_response("", vec![("close_ticket", json!({"ticket_id": "AGN-1", "reason": "All AC met, merged into main"}))]))
    }));

    let runs1 = rt.drive(project, 5).await.unwrap();
    let kinds: Vec<(String, String)> = runs1
        .iter()
        .map(|r| (r.kind.clone(), r.outcome.clone()))
        .collect();
    assert_eq!(
        kinds,
        vec![
            ("work".into(), "Completed".into()),
            ("review".into(), "Completed".into())
        ]
    );
    assert_eq!(p.remaining(), 0, "every scripted turn was used");

    let t = app.ticket(project, &key("AGN-1")).unwrap();
    assert_eq!(t.status, TicketStatus::Closed);
    assert!(t.approved && t.merged && t.has_code);
    assert!(
        t.usage.tokens_in > 0,
        "usage is recorded per ticket (display only)"
    );
    let proj = app.project(project).unwrap();
    assert_eq!(
        std::fs::read_to_string(std::path::Path::new(&proj.path).join("deliverables/hello.txt"))
            .unwrap(),
        "hello\n"
    );
    assert!(!std::path::Path::new(&proj.path)
        .join(".agenium/worktrees/AGN-1")
        .exists());
    assert_eq!(app.agent(agent).unwrap().status, AgentStatus::Idle);
    // ADR-63: every plan item done, nothing open: the project is done and its status follows.
    use agenium_orchestrator::views::RunState;
    use agenium_store::core::projects::ProjectStatus;
    assert_eq!(app.run_state(project).unwrap(), RunState::Done);
    assert!(rt.work(project).await.unwrap().is_empty());
    assert_eq!(
        app.project(project).unwrap().status,
        ProjectStatus::Completed
    );
    // The user's next request on the done project: a newer agreed spec makes it run again and asks the Lead to revise
    // the plan (it stayed Done before, and nothing planned the new spec).
    use agenium_core::plan::{PlanRevision, Requirement};
    let lead = agenium_orchestrator::CallCtx {
        project,
        actor: Actor::Lead(agent),
        intent_id: None,
        run: None,
    };
    let mut next = spec();
    next.requirements.push(Requirement {
        id: "REQ-2".into(),
        text: "A farewell file exists".into(),
        acceptance: vec![AcceptanceCriterion {
            id: "AC-1".into(),
            text: "bye.txt says bye".into(),
        }],
    });
    app.apply(&lead, Intent::ProposeSpec(next)).unwrap();
    assert_eq!(app.run_state(project).unwrap(), RunState::Running);
    let inbox = app.lead_inbox(project).unwrap();
    assert!(
        inbox.iter().any(|i| i.text.contains(
            "Spec v2 is approved and plan v1 still follows spec v1: revise_plan (base_version 1)"
        )),
        "{inbox:?}"
    );
    let mut items = plan(&[("PI-1", &[]), ("PI-2", &[])]).items;
    items[1].trace_to = vec!["REQ-2".into()];
    // New work under the id of a done item would count as done by its closed ticket: refused with the text to keep.
    let mut reused = items.clone();
    reused[0].title = "Write the farewell file".into();
    reused[0].trace_to.push("REQ-2".into());
    reused.truncate(1);
    let err = app
        .apply(
            &lead,
            Intent::RevisePlan(PlanRevision {
                base_version: 1,
                reason: "the farewell of spec v2".into(),
                milestones: plan(&[]).milestones,
                items: reused.clone(),
            }),
        )
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("plan item PI-1 is done (AGN-1 closed): keep its title and acceptance as they are: title"),
        "{err}"
    );
    // A plan revised before that rule, with the new work under PI-1 (the showcase team's plan v2), is repaired by
    // giving PI-1 back what it read in v1 and the new work its own item.
    use agenium_store::core::plans::{self, PlanRow};
    app.store()
        .commit(None, "test", |w| {
            plans::insert_plan(
                w.tx,
                &PlanRow {
                    id: "legacy-v2".into(),
                    project,
                    version: 2,
                    spec_version: 2,
                    reason: Some("new work under PI-1".into()),
                    diff: None,
                    body: agenium_core::plan::TicketPlan {
                        spec_version: 2,
                        milestones: plan(&[]).milestones,
                        items: reused.clone(),
                    },
                    proposed_by: "agent".into(),
                    approved_by: Some("agent".into()),
                },
            )
        })
        .unwrap();
    assert_eq!(
        app.run_state(project).unwrap(),
        RunState::Done,
        "the new work reads as done"
    );
    app.apply(
        &lead,
        Intent::RevisePlan(PlanRevision {
            base_version: 2,
            reason: "the farewell of spec v2".into(),
            milestones: plan(&[]).milestones,
            items,
        }),
    )
    .unwrap();
    assert!(
        !app.lead_inbox(project)
            .unwrap()
            .iter()
            .any(|i| i.text.contains("revise_plan (base_version")),
        "the plan follows the spec again"
    );
    assert_eq!(
        app.run_state(project).unwrap(),
        RunState::Running,
        "PI-2 waits for its ticket"
    );
    // A Lead that retypes a done item (case, spacing, punctuation) is not refused: the item keeps its own text.
    let mut retyped = plan(&[("PI-1", &[]), ("PI-2", &[])]).items;
    retyped[1].trace_to = vec!["REQ-2".into()];
    retyped[0].title = format!("  {}. ", retyped[0].title.to_uppercase());
    let out = app
        .apply(
            &lead,
            Intent::RevisePlan(PlanRevision {
                base_version: 3,
                reason: "a later revision".into(),
                milestones: plan(&[]).milestones,
                items: retyped,
            }),
        )
        .unwrap();
    assert_eq!(out.result["plan_version"], 4);
    let v4 = app
        .store()
        .read(|c| plans::plan(c, project, 4))
        .unwrap()
        .unwrap();
    assert_eq!(
        v4.body.items[0].title,
        plan(&[("PI-1", &[])]).items[0].title,
        "PI-1 kept"
    );

    // Work order structure: cache-stable order, protocol + work tools, no sampling knobs.
    let reqs = p.requests();
    let work = &reqs[5];
    assert!(work.system[0].text.contains("You are an agent of Agenium") && !work.system[0].cache);
    // Without the agent's own limit, the model's output limit as its vendor lists it (the scripted model: 8192).
    assert_eq!(work.max_tokens, 8192);
    assert!(
        work.system[1].cache,
        "SOUL/ROLE block ends the cached prefix"
    );
    // ADR-113: the system holds only what is the same across runs (the tools follow it in the prompt); the
    // worktree is in the run's own message.
    let caps = &work.system.last().unwrap().text;
    assert!(
        caps.starts_with("Capabilities") && !caps.contains("Worktree"),
        "{caps}"
    );
    assert!(work.messages[0].text().contains("Your worktree is"));
    let names: Vec<&str> = work.tools.iter().map(|t| t.name.as_str()).collect();
    assert!(
        names.windows(2).all(|w| w[0] < w[1]),
        "tools sorted by name"
    );
    for n in ["submit_report", "write_file", "shell", "review_ticket"] {
        assert!(names.contains(&n), "{n}");
    }
    assert!(!names.contains(&"load_skill"));
    // ADR-91: without a lesson that held in two contexts the skill-writing tools are withheld.
    assert!(
        !names.contains(&"propose_skill_update") && !names.contains(&"propose_new_skill"),
        "{names:?}"
    );
    assert!(work.messages[0].text().contains("# Ticket AGN-1"));
    assert!(work.messages[0]
        .text()
        .contains("AC-1: hello.txt says hello"));

    // Intent ids are derived from run + call id: replaying the same call is idempotent.
    let evs = app.store().read(|c| events::since(c, 0, 10_000)).unwrap();
    assert!(evs.iter().any(|e| e.kind == "TicketStatusChanged"
        && e.intent_id
            .as_deref()
            .is_some_and(|i| i.ends_with(":call_submit_report_0"))));
    let all_runs = app.store().read(|c| runs::for_ticket(c, t.id)).unwrap();
    assert_eq!(all_runs.len(), 2);
    assert!(all_runs.iter().all(|r| r.ended_at.is_some()));
    let log = app
        .store()
        .read(|c| runs::log_entries(c, all_runs[0].id))
        .unwrap();
    assert!(log
        .iter()
        .any(|e| e.kind == "tool" && e.data["tool"] == "shell"));
}

/// ADR-63: a project's team runs once the spec is agreed and until the user stops it; stopping one project stops
/// its work only.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_project_runs_from_the_agreed_spec_and_the_user_stops_it() {
    use agenium_orchestrator::views::RunState;
    let f = fixture();
    let p = Arc::new(ScriptedProvider::new("mock"));
    let (app, rt) = runtime(&f, p.clone(), Arc::new(AutoApprove));
    let (project, ada) = solo(&app);
    assert_eq!(app.run_state(project).unwrap(), RunState::Agreeing);
    assert!(
        rt.work(project).await.unwrap().is_empty(),
        "nothing runs before the agreement"
    );
    let lead = agenium_orchestrator::CallCtx {
        project,
        actor: Actor::Lead(ada),
        intent_id: None,
        run: None,
    };
    app.apply(&lead, Intent::ProposeSpec(spec())).unwrap();
    assert_eq!(app.run_state(project).unwrap(), RunState::Running);
    rt.pause_project(project).unwrap();
    assert_eq!(app.run_state(project).unwrap(), RunState::Paused);
    assert!(rt.project_cancel(project).is_cancelled());
    assert!(
        !rt.project_cancel(f.project.id).is_cancelled(),
        "other projects go on"
    );
    assert!(
        rt.work(project).await.unwrap().is_empty(),
        "a stopped project runs nothing"
    );
    rt.resume_project(project).unwrap();
    assert_eq!(app.run_state(project).unwrap(), RunState::Running);
    assert!(!rt.project_cancel(project).is_cancelled());
    // The kill switch stops every project's work.
    rt.cancel.cancel();
    assert!(rt.project_cancel(project).is_cancelled());
    rt.cancel.reset();
    assert_eq!(p.remaining(), 0);
}

/// ADR-64/117: with the default policy a command runs without asking when a sandbox backend works, and goes to the
/// user without one unless the user accepted running unsandboxed; YOLO without a sandbox is refused. (A vendor CLI's
/// own shell never reaches this gate: its hook refuses it, ADR-129.)
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sandboxed_commands_run_without_asking() {
    use agenium_config::ApprovalPolicy;
    use agenium_orchestrator::runtime::bridge::RunGate;
    use agenium_tools::{CommandGate, GateDecision};
    let f = fixture();
    let (app, _rt) = runtime(
        &f,
        Arc::new(ScriptedProvider::new("mock")),
        Arc::new(Denier),
    );
    let (project, _) = solo(&app);
    assert_eq!(app.config().sandbox.approval, ApprovalPolicy::Sandboxed);
    let gate = || RunGate {
        app: app.clone(),
        project,
        ticket: None,
        agent: "ada".into(),
        approver: Arc::new(Denier),
    };
    let cwd = f.root.clone();
    let asked = |d: &GateDecision| matches!(d, GateDecision::Deny(r) if r.contains("not now"));
    let sandbox = agenium_sandbox::detect().backend.is_some();
    let d = gate().check("cargo test", &cwd).await;
    if sandbox {
        assert_eq!(d, GateDecision::Allow);
    } else {
        assert!(asked(&d), "{d:?}");
    }
    app.update_config(|c| c.sandbox.unsandboxed = true).unwrap();
    assert_eq!(
        gate().check("cargo test", &cwd).await,
        GateDecision::Allow,
        "with a sandbox, or accepted without one"
    );
    // ADR-120: installing on the user's machine asks the user even when commands run without asking.
    let d = gate()
        .check("winget install Python.Python.3.12", &cwd)
        .await;
    assert!(asked(&d), "an install goes to the user: {d:?}");
    assert_eq!(
        gate().check("npm install express", &cwd).await,
        GateDecision::Allow,
        "a project dependency does not"
    );
    app.update_config(|c| c.sandbox.unsandboxed = false)
        .unwrap();
    app.update_config(|c| c.sandbox.approval = ApprovalPolicy::OnRequest)
        .unwrap();
    let d = gate().check("cargo test", &cwd).await;
    assert!(asked(&d), "on-request asks even with a sandbox: {d:?}");
    app.update_config(|c| c.sandbox.approval = ApprovalPolicy::Never)
        .unwrap();
    let d = gate().check("cargo test", &cwd).await;
    if sandbox {
        assert_eq!(d, GateDecision::Allow);
    } else {
        assert!(
            matches!(&d, GateDecision::Deny(r) if r.contains("fail-closed")),
            "{d:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn denied_command_question_and_continuation() {
    let f = fixture();
    let p = Arc::new(ScriptedProvider::new("mock"));
    let (app, rt) = runtime(&f, p.clone(), Arc::new(Denier));
    // The user reviews every command (sandbox or not).
    app.update_config(|c| c.sandbox.approval = agenium_config::ApprovalPolicy::OnRequest)
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

    // The command is denied by the user; the agent asks a blocking question and the run parks.
    p.push_calls(
        "",
        vec![("shell", json!({"command": "curl https://example.com"}))],
    );
    p.push(Box::new(|req| {
        let last = req.messages.last().unwrap();
        let denied = matches!(&last.parts[0], agenium_llm::Part::ToolResult { content, is_error: true, .. } if content.contains("command_denied") && content.contains("not now"));
        assert!(denied);
        Ok(calls_response("", vec![("ask_question", json!({"ticket_id": "AGN-1", "category": "Permission", "question": "May I download the example page?", "blocking": true}))]))
    }));
    let s = rt.run_ticket(project, &key("AGN-1")).await.unwrap();
    assert_eq!(s.outcome, "Parked");
    assert_eq!(
        app.ticket(project, &key("AGN-1")).unwrap().status,
        TicketStatus::Pending
    );
    assert!(
        rt.tick(project).await.unwrap().is_empty(),
        "nothing runs while Pending"
    );

    // The user answers; the continuation run resumes the stored conversation with the answer appended.
    let q = app
        .store()
        .read(|c| {
            agenium_store::core::comments::open_blocking(
                c,
                app.ticket(project, &key("AGN-1")).unwrap().id,
            )
        })
        .unwrap();
    app.user_answer(project, &key("AGN-1"), q[0].id, "No, work offline.", None)
        .unwrap();
    p.push(Box::new(|req| {
        let texts: String = req
            .messages
            .iter()
            .map(|m| m.text())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            texts.contains("Your run resumes") && texts.contains("No, work offline."),
            "{texts}"
        );
        assert!(
            req.messages
                .iter()
                .any(|m| m.tool_calls().iter().any(|c| c.name == "ask_question")),
            "previous turns are kept"
        );
        Ok(calls_response(
            "",
            vec![(
                "write_file",
                json!({"path": "notes/offline.md", "content": "offline\n"}),
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
    assert_eq!(
        app.ticket(project, &key("AGN-1")).unwrap().status,
        TicketStatus::Resolved
    );

    // ADR-120: the next ticket starts knowing what the user decided on AGN-1, so it does not ask again.
    app.apply(
        &ctxp(lead),
        Intent::CreateTicket(new_ticket("PI-1", "second", &[], &[])),
    )
    .unwrap();
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
        let first = req.messages[0].text();
        assert!(
            first.contains("The user's decisions on other tickets")
                && first.contains("On AGN-1: asked \"May I download the example page?\"")
                && first.contains("No, work offline."),
            "{first}"
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
    assert_eq!(s.outcome, "Completed");
}

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
