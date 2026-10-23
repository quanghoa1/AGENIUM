//! Minimal MCP stdio server used by agenium-mcp's integration tests (not shipped to users).
//! Tools: `echo` (read-only), `whoami` (returns env FAKE_TOKEN), `write_note` (side effect, appends to
//! notes.txt in the working folder), `fail` (isError), `crash` (read-only, exits the process), `note_then_crash` (side effect, then exits). tools/list is paginated.

use std::io::{BufRead, Write};

use serde_json::{json, Value};

fn send(out: &mut impl Write, v: &Value) {
    writeln!(out, "{v}").unwrap();
    out.flush().unwrap();
}

fn main() {
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    eprintln!("test server started");
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let Ok(msg) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let Some(method) = msg["method"].as_str() else {
            continue; // a response to our ping
        };
        let id = msg["id"].clone();
        if id.is_null() {
            continue; // notification
        }
        let result = match method {
            "initialize" => {
                let r = json!({
                    "protocolVersion": "2025-06-18",
                    "capabilities": { "tools": { "listChanged": false } },
                    "serverInfo": { "name": "agenium-test-server", "version": "1" },
                    "instructions": "test server"
                });
                send(&mut out, &json!({"jsonrpc": "2.0", "id": id, "result": r}));
                // Noise a client must cope with: a log notification, a non-JSON line and a request to it.
                send(
                    &mut out,
                    &json!({"jsonrpc": "2.0", "method": "notifications/message", "params": {"level": "info", "data": "hi"}}),
                );
                writeln!(out, "not json").unwrap();
                send(
                    &mut out,
                    &json!({"jsonrpc": "2.0", "id": "srv-1", "method": "ping"}),
                );
                continue;
            }
            "ping" => json!({}),
            "tools/list" => {
                if msg["params"]["cursor"].is_null() {
                    json!({"tools": [
                        {"name": "echo", "description": "Echo text.", "inputSchema": {"type": "object", "properties": {"text": {"type": "string"}}, "required": ["text"]}, "annotations": {"readOnlyHint": true}},
                        {"name": "whoami", "description": "Token in use.", "inputSchema": {"type": "object"}, "annotations": {"readOnlyHint": true}}
                    ], "nextCursor": "page2"})
                } else {
                    json!({"tools": [
                        {"name": "write_note", "description": "Append a note.", "inputSchema": {"type": "object", "properties": {"text": {"type": "string"}}}},
                        {"name": "fail", "description": "Always fails.", "inputSchema": {"type": "object"}},
                        {"name": "crash", "description": "Exit the server.", "inputSchema": {"type": "object"}, "annotations": {"readOnlyHint": true}},
                        {"name": "note_then_crash", "description": "Append a note, then exit.", "inputSchema": {"type": "object"}}
                    ]})
                }
            }
            "tools/call" => {
                let args = &msg["params"]["arguments"];
                match msg["params"]["name"].as_str().unwrap_or("") {
                    "echo" => {
                        json!({"content": [{"type": "text", "text": format!("echo: {}", args["text"].as_str().unwrap_or(""))}]})
                    }
                    "whoami" => {
                        json!({"content": [{"type": "text", "text": std::env::var("FAKE_TOKEN").unwrap_or_else(|_| "none".into())}]})
                    }
                    "write_note" => {
                        let mut f = std::fs::OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open("notes.txt")
                            .unwrap();
                        writeln!(f, "{}", args["text"].as_str().unwrap_or("")).unwrap();
                        json!({"content": [{"type": "text", "text": "saved"}]})
                    }
                    "fail" => {
                        json!({"content": [{"type": "text", "text": "it broke"}], "isError": true})
                    }
                    "crash" => std::process::exit(3),
                    "note_then_crash" => {
                        let mut f = std::fs::OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open("notes.txt")
                            .unwrap();
                        writeln!(f, "before crash").unwrap();
                        std::process::exit(4)
                    }
                    other => {
                        send(
                            &mut out,
                            &json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32602, "message": format!("unknown tool {other}")}}),
                        );
                        continue;
                    }
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
