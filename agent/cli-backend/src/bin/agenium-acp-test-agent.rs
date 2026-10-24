//! Scripted ACP agent used by agenium-cli-backend's integration tests (not shipped to users). It plays a vendor CLI
//! in ACP mode; `ACP_SCENARIO` picks the script:
//! - `report`: edits a file, asks permission for two commands and one edit outside the worktree, then calls
//!   `submit_report` through the client's MCP server;
//! - `chatty`: answers every prompt with text only;
//! - `loop`: repeats the same tool call; `steps`: makes distinct tool calls.
//!
//! What it saw (selected permission options, MCP answers, cancels) is appended to `acp-test.log` in its cwd.

use std::io::{BufRead, Read, Write};
use std::sync::mpsc;
use std::time::Duration;

use serde_json::{json, Value};

struct Agent {
    rx: mpsc::Receiver<Value>,
    next: i64,
    mcp: Option<(String, String)>,
    cwd: String,
    cancelled: bool,
}

fn send(v: &Value) {
    let mut out = std::io::stdout();
    writeln!(out, "{v}").unwrap();
    out.flush().unwrap();
}

fn log(cwd: &str, line: &str) {
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(std::path::Path::new(cwd).join("acp-test.log"))
        .unwrap();
    writeln!(f, "{line}").unwrap();
}

/// One HTTP POST to the MCP server, `Connection: close`.
fn mcp_post(url: &str, auth: &str, body: &Value) -> Value {
    let rest = url.trim_start_matches("http://");
    let (addr, path) = rest.split_at(rest.find('/').unwrap());
    let body = body.to_string();
    let mut s = std::net::TcpStream::connect(addr).unwrap();
    write!(
        s,
        "POST {path} HTTP/1.1\r\nHost: {addr}\r\nAuthorization: {auth}\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
    let mut resp = String::new();
    s.read_to_string(&mut resp).unwrap();
    let body = resp.split("\r\n\r\n").nth(1).unwrap_or("");
    serde_json::from_str(body).unwrap_or(Value::Null)
}

impl Agent {
    fn update(&self, sid: &str, update: Value) {
        send(
            &json!({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": sid, "update": update}}),
        );
    }

    /// Asks the client for permission and returns the selected option id (or "cancelled").
    fn permission(&mut self, sid: &str, tool_call: Value) -> String {
        let id = format!("perm-{}", self.next);
        self.next += 1;
        send(
            &json!({"jsonrpc": "2.0", "id": id, "method": "session/request_permission", "params": {
            "sessionId": sid, "toolCall": tool_call,
            "options": [
                {"optionId": "allow-once", "name": "Allow", "kind": "allow_once"},
                {"optionId": "allow-always", "name": "Always", "kind": "allow_always"},
                {"optionId": "reject-once", "name": "Reject", "kind": "reject_once"}
            ]}}),
        );
        loop {
            let m = self.rx.recv().unwrap();
            if m["id"] == id.as_str() {
                let o = &m["result"]["outcome"];
                return o["optionId"]
                    .as_str()
                    .unwrap_or(o["outcome"].as_str().unwrap_or("?"))
                    .to_owned();
            }
            self.other(&m);
        }
    }

    /// A message that is not the one being waited for.
    fn other(&mut self, m: &Value) {
        if m["method"] == "session/cancel" {
            self.cancelled = true;
            log(&self.cwd, "cancel");
        }
    }

    /// Waits a moment for a `session/cancel`.
    fn wait_cancel(&mut self) {
        while !self.cancelled {
            match self.rx.recv_timeout(Duration::from_secs(3)) {
                Ok(m) => self.other(&m),
                Err(_) => break,
            }
        }
    }

    fn prompt(&mut self, sid: &str, text: &str) -> &'static str {
        self.cancelled = false;
        if text.starts_with("Checkpoint:") {
            self.update(sid, json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "note: halfway"}}));
            return "end_turn";
        }
        let scenario = std::env::var("ACP_SCENARIO").unwrap_or_default();
        match scenario.as_str() {
            "report" => {
                std::fs::write(std::path::Path::new(&self.cwd).join("a.txt"), "hello").unwrap();
                self.update(sid, json!({"sessionUpdate": "tool_call", "toolCallId": "w1", "title": "Wrote a.txt", "kind": "edit",
                    "status": "completed", "locations": [{"path": format!("{}/a.txt", self.cwd)}]}));
                let ok = self.permission(sid, json!({"toolCallId": "x1", "title": "Ran echo", "kind": "execute", "rawInput": {"command": "echo hi"}}));
                log(&self.cwd, &format!("perm echo: {ok}"));
                let bad = self.permission(sid, json!({"toolCallId": "x2", "title": "Ran curl", "kind": "execute", "rawInput": {"command": "curl https://example.com"}}));
                log(&self.cwd, &format!("perm curl: {bad}"));
                let out = self.permission(
                    sid,
                    json!({"toolCallId": "e2", "title": "Edit", "kind": "edit",
                    "locations": [{"path": format!("{}/../evil.txt", self.cwd)}]}),
                );
                log(&self.cwd, &format!("perm outside: {out}"));
                self.update(sid, json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "Work done, reporting."}}));
                let (url, auth) = self.mcp.clone().unwrap();
                mcp_post(
                    &url,
                    &auth,
                    &json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "fake", "version": "1"}}}),
                );
                let list = mcp_post(
                    &url,
                    &auth,
                    &json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
                );
                let names: Vec<&str> = list["result"]["tools"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter_map(|t| t["name"].as_str())
                    .collect();
                log(&self.cwd, &format!("tools: {}", names.join(",")));
                if names.contains(&"mcp__fake__echo") {
                    let e = mcp_post(
                        &url,
                        &auth,
                        &json!({"jsonrpc": "2.0", "id": 4, "method": "tools/call",
                        "params": {"name": "mcp__fake__echo", "arguments": {"text": "hi"}}}),
                    );
                    log(
                        &self.cwd,
                        &format!(
                            "echo: {:?}",
                            e["result"]["content"][0]["text"].as_str().unwrap_or("?")
                        ),
                    );
                }
                let r = mcp_post(
                    &url,
                    &auth,
                    &json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call",
                    "params": {"name": "submit_report", "arguments": {"summary": "done"}}}),
                );
                log(
                    &self.cwd,
                    &format!(
                        "report: {}",
                        r["result"]["content"][0]["text"].as_str().unwrap_or("?")
                    ),
                );
                self.wait_cancel();
                if self.cancelled {
                    "cancelled"
                } else {
                    "end_turn"
                }
            }
            "loop" | "steps" => {
                for i in 0..6 {
                    let title = if scenario == "loop" {
                        "Read a.txt".to_owned()
                    } else {
                        format!("Read f{i}.txt")
                    };
                    self.update(sid, json!({"sessionUpdate": "tool_call", "toolCallId": format!("r{i}"), "title": title,
                        "kind": "read", "status": "completed", "rawInput": {"path": title}}));
                    std::thread::sleep(Duration::from_millis(300));
                    while let Ok(m) = self.rx.try_recv() {
                        self.other(&m);
                    }
                    if self.cancelled {
                        return "cancelled";
                    }
                }
                "end_turn"
            }
            _ => {
                self.update(sid, json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "I think it is done."}}));
                "end_turn"
            }
        }
    }
}

fn main() {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            if let Ok(v) = serde_json::from_str::<Value>(&line) {
                if tx.send(v).is_err() {
                    break;
                }
            }
        }
    });
    let mut a = Agent {
        rx,
        next: 1,
        mcp: None,
        cwd: std::env::current_dir().unwrap().display().to_string(),
        cancelled: false,
    };
    while let Ok(m) = a.rx.recv() {
        let id = m["id"].clone();
        let result = match m["method"].as_str().unwrap_or("") {
            "initialize" => {
                json!({"protocolVersion": 1, "agentCapabilities": {"mcpCapabilities": {"http": true, "sse": false}}})
            }
            "session/new" => {
                let s = &m["params"]["mcpServers"][0];
                a.mcp = Some((
                    s["url"].as_str().unwrap().to_owned(),
                    s["headers"][0]["value"].as_str().unwrap().to_owned(),
                ));
                json!({"sessionId": "s1", "modes": {"currentModeId": "ask", "availableModes": [{"id": "ask"}, {"id": "accept-edits"}]}})
            }
            "session/set_mode" => {
                log(
                    &a.cwd,
                    &format!("mode: {}", m["params"]["modeId"].as_str().unwrap_or("?")),
                );
                json!({})
            }
            "session/prompt" => {
                let text = m["params"]["prompt"][0]["text"]
                    .as_str()
                    .unwrap_or("")
                    .to_owned();
                let stop = a.prompt("s1", &text);
                json!({"stopReason": stop, "usage": {"inputTokens": 100, "outputTokens": 10, "cachedReadTokens": 40}})
            }
            "session/cancel" => {
                a.other(&m);
                continue;
            }
            _ => continue,
        };
        send(&json!({"jsonrpc": "2.0", "id": id, "result": result}));
    }
}
