//! Connects to one MCP server and prints what it offers: protocol, capabilities, tools, resources, prompts.
//!
//! cargo run -p agenium-mcp --example mcp_probe -- <url>
//! cargo run -p agenium-mcp --example mcp_probe -- --cmd <program> [args...]

use std::collections::BTreeMap;
use std::time::Duration;

use agenium_mcp::{Client, ConnectSpec};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let spec = match args.first().map(String::as_str) {
        Some("--cmd") => ConnectSpec::Stdio {
            command: args.get(1).cloned().ok_or("--cmd needs a program")?,
            args: args[2..].to_vec(),
            env: BTreeMap::new(),
            cwd: std::env::current_dir()?,
        },
        Some(url) => ConnectSpec::Http {
            url: url.to_owned(),
            headers: BTreeMap::new(),
            bearer: None,
        },
        None => return Err("usage: mcp_probe <url> | --cmd <program> [args...]".into()),
    };
    let t = Duration::from_secs(30);
    let c = Client::connect(&spec, t).await?;
    println!(
        "server: {} (protocol {})",
        c.server_info["name"], c.protocol
    );
    println!("capabilities: {}", c.capabilities);
    let tools = c.list_tools(t).await?;
    println!(
        "tools ({}): {}",
        tools.len(),
        tools
            .iter()
            .map(|x| x.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
    if c.capabilities.get("resources").is_some() {
        let r = c.list_resources(t).await?;
        println!(
            "resources: {} listed; first: {}",
            r.lines().count(),
            r.lines().next().unwrap_or("")
        );
        if let Some(uri) = r.lines().next().and_then(|l| l.split(" — ").next()) {
            let body = c.read_resource(uri, t).await?;
            println!(
                "read {uri}: {}",
                body.chars()
                    .take(120)
                    .collect::<String>()
                    .replace('\n', " ")
            );
        }
    }
    if c.capabilities.get("prompts").is_some() {
        let p = c.list_prompts(t).await?;
        println!("prompts: {}", p.replace('\n', " | "));
        if let Some(name) = p.lines().next().and_then(|l| l.split('(').next()) {
            let got = c.get_prompt(name, serde_json::json!({}), t).await;
            println!(
                "get {name}: {}",
                match got {
                    Ok(s) => s.chars().take(120).collect::<String>().replace('\n', " "),
                    Err(e) => format!("error: {e}"),
                }
            );
        }
    }
    Ok(())
}
