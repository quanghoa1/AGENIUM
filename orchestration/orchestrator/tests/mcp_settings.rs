//! M15-3 settings: pasted MCP JSON, secrets moved to the vault, bindings and removal rules.

mod common;

use common::*;

#[test]
fn pasted_json_moves_secrets_to_the_vault() {
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
    assert_eq!(r.secrets.len(), 2);
    let cfg = f.app.config();
    let gh = cfg.mcp_server("github").unwrap();
    assert_eq!(
        gh.env["GITHUB_PERSONAL_ACCESS_TOKEN"],
        "${secret:mcp-github-github-personal-access-token}"
    );
    assert_eq!(gh.env["LOG_LEVEL"], "info", "non-secret values stay");
    let docs = cfg.mcp_server("docs").unwrap();
    assert_eq!(
        docs.headers["Authorization"],
        "Bearer ${secret:mcp-docs-authorization}"
    );
    let text = std::fs::read_to_string(f.app.layout().config()).unwrap();
    assert!(
        !text.contains("ghp_abc") && !text.contains("sk-live"),
        "no secret in config.toml"
    );
    let v = f.app.secret("mcp-docs-authorization").unwrap().unwrap();
    assert_eq!(v.expose_str().unwrap(), "sk-live-1234567890abcdef");

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
                tools: None,
            }],
        )
        .is_err());
    assert!(f.app.remove_mcp_server("docs").is_err());
    f.app.remove_mcp_server("github").unwrap();
    f.app.set_agent_mcp(a.id, &[]).unwrap();
    f.app.remove_mcp_server("docs").unwrap();
    assert!(f.app.config().mcp.is_empty());
}
