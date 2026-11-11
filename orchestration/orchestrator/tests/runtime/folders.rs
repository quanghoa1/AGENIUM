//! A project's reference folders in its runs (ADR-159): named in the manifest, readable, never writable.

use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_reference_folder_is_named_readable_and_never_written() {
    let f = fixture();
    let p = Arc::new(ScriptedProvider::new("mock"));
    let (app, rt) = runtime(&f, p.clone(), Arc::new(AutoApprove));
    let (project, agent) = solo(&app);
    let sdk = f._tmp.path().join("device-sdk");
    std::fs::create_dir_all(&sdk).unwrap();
    std::fs::write(sdk.join("api.h"), "int blink(int led);\n").unwrap();
    let kept = app
        .set_references(
            project,
            vec![agenium_store::core::projects::Reference {
                path: sdk.display().to_string(),
                note: " the device SDK ".into(),
            }],
        )
        .unwrap();
    assert_eq!(kept[0].note, "the device SDK");
    let header = std::path::Path::new(&kept[0].path)
        .join("api.h")
        .display()
        .to_string();
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

    let h = header.clone();
    p.push(Box::new(move |req| {
        let system: String = req.system.iter().map(|s| s.text.as_str()).collect();
        assert!(
            system.contains("Reference folders (read-only") && system.contains("(the device SDK)"),
            "{system}"
        );
        Ok(calls_response(
            "",
            vec![
                ("read_file", json!({"path": h})),
                ("write_file", json!({"path": h, "content": "x"})),
            ],
        ))
    }));
    p.push(Box::new(|req| {
        let r: Vec<(String, bool)> = req
            .messages
            .last()
            .unwrap()
            .parts
            .iter()
            .filter_map(|p| match p {
                agenium_llm::Part::ToolResult {
                    content, is_error, ..
                } => Some((content.clone(), *is_error)),
                _ => None,
            })
            .collect();
        assert!(r[0].0.contains("int blink") && !r[0].1, "{r:?}");
        assert!(r[1].1, "a reference folder is read-only: {r:?}");
        Ok(calls_response(
            "",
            vec![(
                "write_file",
                json!({"path": "deliverables/blink.c", "content": "#include \"api.h\"\n"}),
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
        std::fs::read_to_string(&header).unwrap(),
        "int blink(int led);\n"
    );
}

/// ADR-159: a picked repository on a new branch. The ticket's work is merged into Agenium's branch and its checkout
/// (`.agenium/base`); the user's folder keeps its own branch and change in progress.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn work_on_a_new_branch_is_merged_beside_the_users_branch() {
    let f = fixture();
    let p = Arc::new(ScriptedProvider::new("mock"));
    let (app, rt) = runtime(&f, p.clone(), Arc::new(AutoApprove));
    let dir = f._tmp.path().join("mine");
    std::fs::create_dir_all(&dir).unwrap();
    let git = |args: &[&str]| agenium_vcs::git::Git::default().check(&dir, args).unwrap();
    git(&["init", "-q", "-b", "master"]);
    std::fs::write(dir.join("app.txt"), "v1\n").unwrap();
    git(&["add", "app.txt"]);
    git(&[
        "-c",
        "user.name=u",
        "-c",
        "user.email=u@x",
        "commit",
        "-q",
        "-m",
        "first",
    ]);
    std::fs::write(dir.join("app.txt"), "v1 + my edit\n").unwrap();
    let project = app
        .create_project_on(
            &agenium_orchestrator::CreateProject {
                name: "mine".into(),
                path: Some(dir.clone()),
                key_prefix: Some("AGN".into()),
                template: None,
                folder: None,
                repo: None,
            },
            Some("agenium-work"),
        )
        .unwrap()
        .id;
    let (_, agent) = app
        .create_solo(project, &agent_spec("ada", "solo", RoleKind::Lead, &[]))
        .unwrap();
    let agent = agent.id;
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
    p.push_calls(
        "",
        vec![(
            "write_file",
            json!({"path": "deliverables/feature.txt", "content": "by agent\n"}),
        )],
    );
    p.push_calls(
        "",
        vec![(
            "submit_report",
            serde_json::to_value(report("AGN-1")).unwrap(),
        )],
    );
    let base = dir.join(".agenium").join("base");
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
            json!({"ticket_id": "AGN-1", "reason": "All AC met, merged"}),
        )],
    );
    rt.run_ticket(project, &key("AGN-1")).await.unwrap();
    rt.review(project, &key("AGN-1")).await.unwrap();
    assert_eq!(
        app.ticket(project, &key("AGN-1")).unwrap().status,
        TicketStatus::Closed
    );
    assert_eq!(
        std::fs::read_to_string(base.join("deliverables/feature.txt")).unwrap(),
        "by agent\n"
    );
    assert_eq!(git(&["symbolic-ref", "--short", "HEAD"]).trim(), "master");
    assert!(!dir.join("deliverables").exists());
    assert_eq!(
        std::fs::read_to_string(dir.join("app.txt")).unwrap(),
        "v1 + my edit\n"
    );
    assert_eq!(git(&["log", "--format=%s", "master"]).trim(), "first");
}
