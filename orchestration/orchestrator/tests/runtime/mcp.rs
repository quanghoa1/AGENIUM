//! MCP tools reaching the agent with its credential.

use super::*;

/// M15-3: an agent bound to an MCP server sees its tools (`mcp__docs__*`) with its own credential; the output is
/// labeled untrusted; review runs only get read-only MCP tools; a broken binding shows in the manifest.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mcp_tools_reach_the_agent_with_its_credential() {
    let f = fixture();
    let p = Arc::new(ScriptedProvider::new("mock"));
    let (app, rt) = runtime(&f, p.clone(), Arc::new(AutoApprove));
    let (project, agent) = solo(&app);
    let url = mcp_stub("docs-dev-token-123").await;
    app.secrets()
        .set("docs-dev", b"docs-dev-token-123")
        .unwrap();
    app.put_mcp_server(agenium_config::McpServerConfig {
        id: "docs".into(),
        command: None,
        args: vec![],
        env: Default::default(),
        url: Some(url),
        headers: [(
            "Authorization".to_string(),
            "Bearer ${cred:TOKEN}".to_string(),
        )]
        .into(),
        timeout_secs: 10,
        oauth: None,
        approval: None,
    })
    .unwrap();
    // A key written as a plain value is the server's system key (the owner's decision: shown in the app).
    app.put_mcp_server(agenium_config::McpServerConfig {
        id: "plain".into(),
        command: Some("x".into()),
        args: vec![],
        env: [("API_TOKEN".to_string(), "abc".to_string())].into(),
        url: None,
        headers: Default::default(),
        timeout_secs: 10,
        oauth: None,
        approval: None,
    })
    .unwrap();
    app.remove_mcp_server("plain").unwrap();
    app.set_agent_mcp(
        agent,
        &[agenium_mcp::Binding {
            server: "docs".into(),
            credentials: [("TOKEN".to_string(), "docs-dev".to_string())].into(),
            keys: Default::default(),
            tools: None,
        }],
    )
    .unwrap();
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

    p.push(Box::new(|req| {
        let names: Vec<&str> = req.tools.iter().map(|t| t.name.as_str()).collect();
        assert!(
            names.contains(&"mcp__docs__search") && names.contains(&"mcp__docs__publish"),
            "{names:?}"
        );
        assert!(
            req.system[2].text.contains("MCP servers: docs"),
            "{}",
            req.system[2].text
        );
        Ok(calls_response(
            "",
            vec![("mcp__docs__search", json!({"q": "retry policy"}))],
        ))
    }));
    p.push(Box::new(|req| {
        let last = req.messages.last().unwrap();
        let ok = matches!(&last.parts[0], agenium_llm::Part::ToolResult { content, is_error: false, .. }
            if content.starts_with("<untrusted_content source=\"MCP server docs, tool search\"")
                && content.contains("\nfound: retry policy") && content.contains("Untrusted content"));
        assert!(ok, "{:?}", last.parts[0]);
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

    // Review run (read-only tools only): publish is not offered, search is.
    p.push(Box::new(|req| {
        let names: Vec<&str> = req.tools.iter().map(|t| t.name.as_str()).collect();
        assert!(
            names.contains(&"mcp__docs__search") && !names.contains(&"mcp__docs__publish"),
            "{names:?}"
        );
        Err(agenium_llm::LlmError::Unsupported("stop here".into()))
    }));
    let _ = rt.review(project, &key("AGN-1")).await;

    // A binding whose secret is missing is reported in the manifest instead of failing the run.
    app.secrets().delete("docs-dev").unwrap();
    rt.mcp.disconnect_all().await;
    p.push(Box::new(|req| {
        assert!(
            req.system[2].text.contains("MCP server unavailable: docs:"),
            "{}",
            req.system[2].text
        );
        assert!(!req.tools.iter().any(|t| t.name.starts_with("mcp__")));
        Err(agenium_llm::LlmError::Unsupported("stop here".into()))
    }));
    let _ = rt.review(project, &key("AGN-1")).await;
    assert_eq!(p.remaining(), 0, "both review requests were made");
}

/// ADR-143: an MCP tool its server marks destructive waits for the user (here refused: the agent is told and the
/// server is not called); a read-only one runs without asking; with the server set to never ask it runs, and so it
/// does once allowed always for the project.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_destructive_mcp_tool_waits_for_the_user() {
    let f = fixture();
    let p = Arc::new(ScriptedProvider::new("mock"));
    let (app, rt) = runtime(&f, p.clone(), Arc::new(Denier));
    let (project, agent) = solo_with_tickets(&app, &["PI-1", "PI-2", "PI-3"]);
    let url = mcp_stub("docs-token-9").await;
    app.secrets().set("docs-key", b"docs-token-9").unwrap();
    let server = |approval| agenium_config::McpServerConfig {
        id: "docs".into(),
        command: None,
        args: vec![],
        env: Default::default(),
        url: Some(url.clone()),
        headers: [(
            "Authorization".to_string(),
            "Bearer ${cred:TOKEN}".to_string(),
        )]
        .into(),
        timeout_secs: 10,
        oauth: None,
        approval,
    };
    app.put_mcp_server(server(None)).unwrap();
    app.set_agent_mcp(
        agent,
        &[agenium_mcp::Binding {
            server: "docs".into(),
            credentials: [("TOKEN".to_string(), "docs-key".to_string())].into(),
            keys: Default::default(),
            tools: None,
        }],
    )
    .unwrap();
    // Calls `publish` then `search`, and reads what came back of each.
    let script = |p: &ScriptedProvider, k: &'static str, publish_ok: bool| {
        p.push(Box::new(|_| {
            Ok(calls_response(
                "",
                vec![
                    ("mcp__docs__publish", json!({"q": "the page"})),
                    ("mcp__docs__search", json!({"q": "retry"})),
                ],
            ))
        }));
        p.push(Box::new(move |req| {
            let results: Vec<(String, bool)> = req
                .messages
                .last()
                .unwrap()
                .parts
                .iter()
                .filter_map(|x| match x {
                    agenium_llm::Part::ToolResult {
                        content, is_error, ..
                    } => Some((content.clone(), *is_error)),
                    _ => None,
                })
                .collect();
            if publish_ok {
                assert!(
                    !results[0].1 && results[0].0.contains("found: the page"),
                    "{results:?}"
                );
            } else {
                assert!(
                    results[0].1 && results[0].0.contains("the user did not allow docs/publish"),
                    "{results:?}"
                );
                assert!(
                    results[0].0.contains("not now: MCP docs/publish"),
                    "{results:?}"
                );
            }
            assert!(
                !results[1].1 && results[1].0.contains("found: retry"),
                "{results:?}"
            );
            Ok(calls_response(
                "",
                vec![("submit_report", serde_json::to_value(report(k)).unwrap())],
            ))
        }));
    };
    script(&p, "AGN-1", false);
    assert_eq!(
        rt.run_ticket(project, &key("AGN-1")).await.unwrap().outcome,
        "Completed"
    );

    // The server set to never ask: publish runs.
    app.put_mcp_server(server(Some(agenium_config::McpApproval::Never)))
        .unwrap();
    rt.mcp.disconnect_all().await;
    script(&p, "AGN-2", true);
    assert_eq!(
        rt.run_ticket(project, &key("AGN-2")).await.unwrap().outcome,
        "Completed"
    );

    // Back to the default, publish allowed always for the project: it runs without asking.
    app.put_mcp_server(server(None)).unwrap();
    rt.mcp.disconnect_all().await;
    app.store()
        .commit(None, "user", |w| {
            agenium_store::core::projects::add_allowlist(
                w.tx,
                project,
                &["mcp:docs/publish".to_string()],
            )
        })
        .unwrap();
    script(&p, "AGN-3", true);
    assert_eq!(
        rt.run_ticket(project, &key("AGN-3")).await.unwrap().outcome,
        "Completed"
    );
    assert_eq!(p.remaining(), 0);
}
