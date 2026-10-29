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
