//! ADR-166: the project map. Agents add what they traced with its evidence; Agenium checks it (refusing what the code
//! or the page does not show), keeps a ticket's additions tentative until it closes, follows code that moved, marks
//! what broke stale, and drops what a cancelled ticket added.

use super::*;

const APP_PY: &str = "import config\n\ndef main():\n    cfg = config.load_config()\n    run(cfg)\n\ndef run(c):\n    print(c)\n";

fn last_result(req: &agenium_llm::ChatRequest) -> String {
    match &req.messages.last().unwrap().parts[0] {
        agenium_llm::Part::ToolResult { content, .. } => content.clone(),
        other => panic!("not a tool result: {other:?}"),
    }
}

fn commit(app: &agenium_orchestrator::App, project: ProjectId, files: &[(&str, &str)]) {
    let p = app.project(project).unwrap();
    let root = std::path::PathBuf::from(&p.path);
    for (f, body) in files {
        let path = root.join(f);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }
    app.repo(&p)
        .commit_all(&root, "code", ("user", "user@example.com"), &[])
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn agents_build_the_map_agenium_checks_it_and_a_closed_ticket_confirms_it() {
    let f = fixture();
    let p = Arc::new(ScriptedProvider::new("mock"));
    let (app, rt) = runtime(&f, p.clone(), Arc::new(AutoApprove));
    let (project, agent) = solo_with_tickets(&app, &["PI-1", "PI-2"]);
    commit(
        &app,
        project,
        &[
            ("src/app.py", APP_PY),
            ("src/config.py", "def load_config():\n    return {}\n"),
        ],
    );

    p.push(Box::new(|req| {
        assert!(
            req.messages[0].text().contains("map_add") || req.tools.iter().any(|t| t.name == "map_add"),
            "the map tools are offered"
        );
        Ok(calls_response(
            "",
            vec![(
                "map_add",
                json!({
                    "nodes": [{"name": "symbol:src/app.py#main", "summary": "Entry point: loads the config, runs"}],
                    "links": [
                        {"from": "symbol:src/app.py#main", "type": "calls", "to": "symbol:src/config.py#load_config",
                         "evidence": {"file": "src/app.py", "line": 4}},
                        {"from": "symbol:src/app.py#main", "type": "calls", "to": "symbol:src/app.py#run",
                         "evidence": {"file": "src/app.py", "line": 4}},
                        {"from": "symbol:src/app.py#main", "type": "calls", "to": "symbol:src/config.py#save",
                         "evidence": {"file": "src/app.py", "line": 4}},
                        {"from": "symbol:src/app.py#main", "type": "calls", "to": "symbol:src/app.py#run",
                         "evidence": {"file": "src/app.py", "line": 5}}
                    ]
                }),
            )],
        ))
    }));
    p.push(Box::new(|req| {
        let r = last_result(req);
        assert!(
            r.contains("Kept 3"),
            "main, its call of load_config and of run: {r}"
        );
        assert!(r.contains("Refused 2"), "{r}");
        assert!(
            r.contains("does not name \\\"run\\\"") || r.contains("does not name"),
            "{r}"
        );
        assert!(
            r.contains("declares no \\\"save\\\"") || r.contains("declares no"),
            "{r}"
        );
        Ok(calls_response(
            "",
            vec![(
                "map_query",
                json!({"node": "symbol:src/config.py#load_config", "impact": true}),
            )],
        ))
    }));
    p.push(Box::new(|req| {
        let r = last_result(req);
        assert!(
            r.contains("symbol:src/app.py#main --calls--> symbol:src/config.py#load_config  [src/app.py:4] (tentative: AGN-1)"),
            "{r}"
        );
        Ok(calls_response(
            "",
            vec![("submit_report", serde_json::to_value(report("AGN-1")).unwrap())],
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
            json!({"ticket_id": "AGN-1", "reason": "Checked: the map is right"}),
        )],
    );
    rt.run_ticket(project, &key("AGN-1")).await.unwrap();
    rt.review(project, &key("AGN-1")).await.unwrap();
    assert_eq!(
        app.ticket(project, &key("AGN-1")).unwrap().status,
        TicketStatus::Closed
    );
    let s = app.map_stats(project).unwrap();
    assert_eq!(
        (s.nodes, s.links, s.tentative),
        (3, 2, 5),
        "tentative until settled"
    );
    app.map_settle(project).unwrap();
    let s = app.map_stats(project).unwrap();
    assert_eq!(
        (s.tentative, s.stale),
        (0, 0),
        "confirmed on the main branch"
    );

    // The user's search and a path, as the Map page asks them.
    let hits = app.map_search(project, "symbol:load").unwrap();
    assert_eq!(hits[0].key, "symbol:src/config.py#load_config");
    let path = app
        .map_path(
            project,
            "symbol:src/app.py#main",
            "symbol:src/config.py#load_config",
        )
        .unwrap()
        .unwrap();
    assert_eq!(path.links.len(), 1);
    let around = app
        .map_around(project, Some("symbol:src/app.py#main"), 1)
        .unwrap();
    assert_eq!(around.nodes.len(), 3);

    // Code moved down: the link follows it. The call removed: the link is stale, not deleted.
    let moved = APP_PY.replace("def main():\n", "\n\ndef main():\n    # read it\n");
    commit(&app, project, &[("src/app.py", &moved)]);
    app.map_recheck(project).unwrap();
    let l = app
        .map_around(project, Some("symbol:src/config.py#load_config"), 1)
        .unwrap();
    assert_eq!(l.links[0].evidence, "src/app.py:7");
    assert_eq!(l.links[0].status, "confirmed");
    commit(&app, project, &[("src/app.py", "def main():\n    pass\n")]);
    app.map_recheck(project).unwrap();
    let s = app.map_stats(project).unwrap();
    assert!(s.stale >= 2, "both calls of main are gone: {s:?}");

    // What a cancelled ticket added goes with it.
    p.push_calls(
        "",
        vec![(
            "map_add",
            json!({"nodes": [{"name": "file:src/config.py", "summary": "Settings"}]}),
        )],
    );
    p.push_calls(
        "",
        vec![(
            "submit_report",
            serde_json::to_value(report("AGN-2")).unwrap(),
        )],
    );
    rt.run_ticket(project, &key("AGN-2")).await.unwrap();
    assert!(app
        .map_search(project, "file:config")
        .unwrap()
        .iter()
        .any(|n| n.status == "tentative"));
    app.apply(
        &agenium_orchestrator::CallCtx {
            project,
            actor: Actor::Lead(agent),
            intent_id: None,
            run: None,
        },
        Intent::CancelTicket(agenium_core::forms::CancelTicket {
            ticket_id: key("AGN-2"),
            reason: "Not needed any more".into(),
            replaced_by: None,
        }),
    )
    .unwrap();
    app.map_settle(project).unwrap();
    assert!(app.map_search(project, "file:config").unwrap().is_empty());
}

/// A page Agenium reads itself: a link from what the page says, refused when the page does not say it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_link_from_the_web_holds_only_when_the_page_says_it() {
    let f = fixture();
    let app = Arc::new(open_app(
        &f.root.parent().unwrap().join("web-root"),
        f.keyring.clone(),
        None,
    ));
    let (project, _) = solo(&app);
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    tokio::spawn(async move {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        while let Ok((mut s, _)) = l.accept().await {
            let mut buf = [0u8; 2048];
            let _ = s.read(&mut buf).await;
            let body = "<html><body><h1>Acme Desktop</h1><p>The Acme Desktop app talks to api.acme.example over HTTPS.</p></body></html>";
            let _ = s
                .write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes())
                .await;
        }
    });
    let root = std::path::PathBuf::from(app.project(project).unwrap().path);
    let mut ctx = agenium_tools::ToolCtx::new(&root, "r", Arc::new(agenium_tools::AllowAll));
    ctx.fetch_allow = vec!["127.0.0.1".into()];
    let tools = agenium_orchestrator::map::tools::tools(
        app.clone(),
        agenium_orchestrator::map::tools::Who {
            project,
            ticket: None,
            agent: "ada".into(),
            run: RunId::new(),
        },
    );
    let add = tools.iter().find(|t| t.name() == "map_add").unwrap();
    let url = format!("http://127.0.0.1:{port}/acme");
    let o = add
        .call(
            &ctx,
            json!({"links": [
                {"from": "product:Acme Desktop", "type": "requests", "to": "domain:api.acme.example", "evidence": {"url": url}},
                {"from": "product:Acme Desktop", "type": "requests", "to": "domain:telemetry.acme.example", "evidence": {"url": url}}
            ]}),
        )
        .await
        .unwrap();
    assert!(o.text.contains("Kept 1"), "{}", o.text);
    assert!(
        o.text
            .contains("does not mention \"telemetry.acme.example\""),
        "{}",
        o.text
    );
    let s = app.map_stats(project).unwrap();
    assert_eq!(
        (s.nodes, s.links, s.tentative),
        (2, 1, 0),
        "no ticket: confirmed at once"
    );
    // Taking back what still holds is refused.
    let retract = tools.iter().find(|t| t.name() == "map_retract").unwrap();
    let e = retract
        .call(
            &ctx,
            json!({"link": {"from": "product:Acme Desktop", "type": "requests", "to": "domain:api.acme.example"}, "reason": "wrong"}),
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, "still_holds");
}
