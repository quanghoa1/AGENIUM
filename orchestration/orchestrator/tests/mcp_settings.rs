//! M15-3 settings: pasted MCP JSON, per-agent keys, bindings and removal rules.

mod common;

use common::*;

#[test]
fn pasted_json_keeps_keys_and_agents_get_their_own() {
    let f = fixture();
    let r = f
        .app
        .import_mcp_json(
            r#"{"mcpServers": {
                "GitHub": {"command": "npx", "args": ["-y", "@modelcontextprotocol/server-github"],
                           "env": {"GITHUB_PERSONAL_ACCESS_TOKEN": "ghp_abcdefghijklmnopqrstuvwxyz0123456789", "LOG_LEVEL": "info"}},
                "docs": {"url": "https://mcp.example.com/mcp", "headers": {"Authorization": "Bearer sk-live-1234567890abcdef"}}
            }}"#,
        )
        .unwrap();
    assert_eq!(r.servers, vec!["github", "docs"]);
    let cfg = f.app.config();
    let gh = cfg.mcp_server("github").unwrap();
    assert_eq!(
        gh.env["GITHUB_PERSONAL_ACCESS_TOKEN"], "ghp_abcdefghijklmnopqrstuvwxyz0123456789",
        "keys are kept as written (the system key)"
    );
    let docs = cfg.mcp_server("docs").unwrap();
    assert_eq!(
        docs.headers["Authorization"],
        "Bearer sk-live-1234567890abcdef"
    );

    // Insecure URLs are refused by the config check.
    assert!(f
        .app
        .import_mcp_json(r#"{"mcpServers": {"x": {"url": "http://example.com/mcp"}}}"#)
        .is_err());

    // Bound servers cannot be removed.
    let a = solo_with_plan(&f, &[("PI-1", &[])]);
    f.app
        .set_agent_mcp(
            a.id,
            &[agenium_mcp::Binding {
                server: "docs".into(),
                credentials: Default::default(),
                keys: Default::default(),
                tools: None,
            }],
        )
        .unwrap();
    assert!(f
        .app
        .set_agent_mcp(
            a.id,
            &[agenium_mcp::Binding {
                server: "nope".into(),
                credentials: Default::default(),
                keys: Default::default(),
                tools: None,
            }],
        )
        .is_err());
    // The agent's own key for one entry; the app shows both, logs mask them.
    assert!(
        f.app
            .set_agent_mcp_key(
                a.id,
                "github",
                "GITHUB_PERSONAL_ACCESS_TOKEN",
                Some("x"),
                "user"
            )
            .is_err(),
        "the agent does not use github"
    );
    assert!(f
        .app
        .set_agent_mcp_key(a.id, "docs", "Nope", Some("x"), "user")
        .is_err());
    let keys = f.app.agent_mcp_keys(a.id).unwrap();
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0].own, None);
    assert_eq!(
        keys[0].system.as_deref(),
        Some("Bearer sk-live-1234567890abcdef")
    );
    f.app
        .set_agent_mcp_key(
            a.id,
            "docs",
            "Authorization",
            Some("Bearer agent-own-key-123"),
            "user",
        )
        .unwrap();
    let keys = f.app.agent_mcp_keys(a.id).unwrap();
    assert_eq!(keys[0].own.as_deref(), Some("Bearer agent-own-key-123"));
    let log = f
        .app
        .redactor()
        .redact("sent agent-own-key-123 and sk-live-1234567890abcdef");
    assert!(
        !log.contains("agent-own-key-123") && !log.contains("sk-live-1234567890abcdef"),
        "{log}"
    );
    f.app
        .set_agent_mcp_key(a.id, "docs", "Authorization", None, "user")
        .unwrap();
    assert_eq!(f.app.agent_mcp_keys(a.id).unwrap()[0].own, None);
    assert!(f.app.remove_mcp_server("docs").is_err());
    f.app.remove_mcp_server("github").unwrap();
    f.app.set_agent_mcp(a.id, &[]).unwrap();
    f.app.remove_mcp_server("docs").unwrap();
    assert!(f.app.config().mcp.is_empty());
}
