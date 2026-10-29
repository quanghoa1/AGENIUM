//! An MCP stdio server with 123 tools shaped like real servers' (`data/many-tools.json`), for ADR-139's tests and
//! measurement (not shipped to users). `--start-small`: it starts as a server with its own tool search — only
//! `find_tools` and `run_tool` listed — whose search is out of quota at the first use: the call fails, the server lists
//! all its tools from then on and says so (`notifications/tools/list_changed`). Every tool answers with what it was
//! called with; `releases_codename` answers the codename of a version (`BLUE-HERON`), which nothing else holds.
//! `--back-after N`: N calls after it listed every tool, its search is back and it lists only its own two again,
//! without a notice.

use std::io::{BufRead, Write};

use serde_json::{json, Value};

const TOOLS: &str = include_str!("../../data/many-tools.json");

fn send(out: &mut impl Write, v: &Value) {
    writeln!(out, "{v}").unwrap();
    out.flush().unwrap();
}

fn own_search() -> Value {
    json!([
        {"name": "find_tools", "description": "Find this server's tools for a task (semantic search).",
         "inputSchema": {"type": "object", "properties": {"query": {"type": "string"}}, "required": ["query"]},
         "annotations": {"readOnlyHint": true}},
        {"name": "run_tool", "description": "Run one of this server's tools found with find_tools.",
         "inputSchema": {"type": "object", "properties": {"name": {"type": "string"}, "arguments": {"type": "object"}}, "required": ["name"]}}
    ])
}

fn answer(name: &str, args: &Value) -> String {
    match name {
        "releases_codename" => format!(
            "The codename of version {} is BLUE-HERON.",
            args["version"].as_str().unwrap_or("?")
        ),
        _ => format!("{name} done with {args}"),
    }
}

fn main() {
    let mut all: Value = serde_json::from_str(TOOLS).expect("the catalogue");
    // `--opaque`: the tools named `tool_000`, `tool_001`, … in the catalogue's order, their descriptions kept: a server
    // whose names say nothing, so only a search of the descriptions finds a tool.
    if std::env::args().any(|a| a == "--opaque") {
        for (i, t) in all.as_array_mut().expect("a list").iter_mut().enumerate() {
            t["name"] = json!(format!("tool_{i:03}"));
        }
    }
    let codename_tool = if std::env::args().any(|a| a == "--opaque") {
        let i = serde_json::from_str::<Value>(TOOLS)
            .expect("the catalogue")
            .as_array()
            .expect("a list")
            .iter()
            .position(|t| t["name"] == "releases_codename")
            .expect("the codename tool");
        format!("tool_{i:03}")
    } else {
        "releases_codename".to_owned()
    };
    let answer = |name: &str, args: &Value| {
        answer(
            if name == codename_tool {
                "releases_codename"
            } else {
                name
            },
            args,
        )
    };
    let mut small = std::env::args().any(|a| a == "--start-small");
    // `--back-after N`: once listing every tool, its search's quota comes back after N calls and it lists only its own
    // search again, without a word.
    let back_after: Option<u32> = std::env::args()
        .skip_while(|a| a != "--back-after")
        .nth(1)
        .and_then(|n| n.parse().ok());
    let mut calls_full = 0u32;
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let Ok(msg) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let (Some(method), id) = (msg["method"].as_str(), msg["id"].clone()) else {
            continue;
        };
        if id.is_null() {
            continue;
        }
        let result = match method {
            "initialize" => json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {"tools": {"listChanged": true}},
                "serverInfo": {"name": "agenium-many-tools", "version": "1"}
            }),
            "ping" => json!({}),
            "tools/list" => json!({"tools": if small { own_search() } else { all.clone() }}),
            "tools/call" => {
                let name = msg["params"]["name"].as_str().unwrap_or("");
                let args = &msg["params"]["arguments"];
                if small && name == "find_tools" {
                    small = false;
                    send(
                        &mut out,
                        &json!({"jsonrpc": "2.0", "method": "notifications/tools/list_changed"}),
                    );
                    json!({"content": [{"type": "text", "text": "the tool search is out of quota: every tool is listed now"}], "isError": true})
                } else if small && name == "run_tool" {
                    let n = args["name"].as_str().unwrap_or("");
                    json!({"content": [{"type": "text", "text": answer(n, &args["arguments"])}]})
                } else if all.as_array().unwrap().iter().any(|t| t["name"] == name) && !small {
                    calls_full += 1;
                    if back_after.is_some_and(|n| calls_full >= n) {
                        small = true;
                    }
                    json!({"content": [{"type": "text", "text": answer(name, args)}]})
                } else {
                    send(
                        &mut out,
                        &json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32602, "message": format!("unknown tool {name}")}}),
                    );
                    continue;
                }
            }
            other => {
                send(
                    &mut out,
                    &json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": format!("no method {other}")}}),
                );
                continue;
            }
        };
        send(
            &mut out,
            &json!({"jsonrpc": "2.0", "id": id, "result": result}),
        );
    }
}
