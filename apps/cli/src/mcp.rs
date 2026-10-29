//! MCP server commands. Split out of `main.rs`.

use crate::*;

pub(crate) async fn mcp_cmd(cli: &Cli, mc: &McpCmd) -> Result<()> {
    let app = open(&root(cli)?)?;
    match mc {
        McpCmd::Add {
            id,
            command,
            args,
            url,
            env,
            headers,
            timeout,
            oauth_client_id,
            oauth_client_secret,
            oauth_scopes,
            oauth_callback_port,
        } => {
            let oauth = (oauth_client_id.is_some()
                || oauth_client_secret.is_some()
                || !oauth_scopes.is_empty()
                || oauth_callback_port.is_some())
            .then(|| agenium_config::McpOAuthConfig {
                client_id: oauth_client_id.clone(),
                client_secret: oauth_client_secret.clone(),
                scopes: oauth_scopes.clone(),
                callback_port: *oauth_callback_port,
            });
            app.put_mcp_server(agenium_config::McpServerConfig {
                id: id.clone(),
                command: command.clone(),
                args: args.clone(),
                env: pairs(env)?,
                url: url.clone(),
                headers: pairs(headers)?,
                timeout_secs: *timeout,
                oauth,
                approval: None,
            })?;
            println!("MCP server {id} saved (test it: agenium mcp test {id})");
        }
        McpCmd::Import { file } => {
            let text = if file == "-" {
                let mut s = String::new();
                std::io::Read::read_to_string(&mut std::io::stdin(), &mut s)?;
                s
            } else {
                std::fs::read_to_string(file)?
            };
            let r = app.import_mcp_json(&text)?;
            println!("imported: {}", r.servers.join(", "));
        }
        McpCmd::List => {
            let hub = agenium_orchestrator::mcp::hub(app.clone());
            for m in app.config().mcp {
                let target = match (&m.command, &m.url) {
                    (Some(c), _) => format!("stdio: {c} {}", m.args.join(" ")),
                    (_, Some(u)) if hub.signed_in(&m) => format!("http: {u} (signed in)"),
                    (_, Some(u)) => format!("http: {u}"),
                    _ => "?".into(),
                };
                println!("{}	{}", m.id, target.trim_end());
            }
        }
        McpCmd::Remove { id } => {
            app.remove_mcp_server(id)?;
            println!("removed {id}");
        }
        McpCmd::Login {
            id,
            scopes,
            no_browser,
        } => {
            let cfg = app
                .config()
                .mcp_server(id)
                .cloned()
                .ok_or_else(|| anyhow!("no MCP server {id}"))?;
            let hub = agenium_orchestrator::mcp::hub(app.clone());
            let no_browser = *no_browser;
            let open = move |url: &str| {
                if no_browser || !agenium_mcp::oauth::open_browser(url) {
                    println!("Open this URL to sign in:\n{url}");
                } else {
                    println!("Signing in through your browser… (if it did not open: {url})");
                }
            };
            let t = hub.login(&cfg, scopes, &open).await?;
            println!(
                "signed in to {id} (client {}{})",
                t.client_id,
                t.scope
                    .as_deref()
                    .map(|s| format!(", scope {s}"))
                    .unwrap_or_default()
            );
        }
        McpCmd::Logout { id } => {
            agenium_orchestrator::mcp::hub(app.clone())
                .logout(id)
                .await?;
            println!("signed out of {id}");
        }
        McpCmd::Test { id, agent } => {
            let cfg = app
                .config()
                .mcp_server(id)
                .cloned()
                .ok_or_else(|| anyhow!("no MCP server {id}"))?;
            let binding = match agent {
                Some(n) => agenium_mcp::parse_bindings(&app.agent_named(n)?.mcp)?
                    .into_iter()
                    .find(|b| &b.server == id)
                    .ok_or_else(|| anyhow!("{n} is not bound to {id}"))?,
                None => agenium_mcp::Binding {
                    server: id.clone(),
                    credentials: Default::default(),
                    keys: Default::default(),
                    tools: None,
                },
            };
            let hub = agenium_orchestrator::mcp::hub(app.clone());
            let (info, protocol, tools) = hub.test(&cfg, &binding).await?;
            println!(
                "connected: {} {} (protocol {protocol})",
                info["name"].as_str().unwrap_or("?"),
                info["version"].as_str().unwrap_or("")
            );
            for t in tools {
                let ro = if t.read_only { " [read-only]" } else { "" };
                println!(
                    "  {}{ro}: {}",
                    t.name,
                    t.description.lines().next().unwrap_or("")
                );
            }
        }
    }
    Ok(())
}
