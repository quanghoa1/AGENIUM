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
