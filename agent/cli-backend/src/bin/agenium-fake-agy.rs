//! Scripted stand-in for `agy` in headless stream-json mode, used by agenium-cli-backend's tests (not shipped).
//! It reads its MCP servers and allow-rules from `$USERPROFILE/.gemini` exactly where agy does, so the tests prove
//! the per-run home is used. Like agy, a command without an allow-rule ends the turn with `denied_actions`.
//! `FAKE_AGY_SCENARIO`: `report` (default), `chatty`, `loop`, `steps`. What it saw goes to `fake-agy.log` in its cwd.

use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

fn out(v: Value) {
    let mut o = std::io::stdout();
    writeln!(o, "{v}").unwrap();
    o.flush().unwrap();
}

fn log(line: &str) {
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("fake-agy.log")
        .unwrap();
    writeln!(f, "{line}").unwrap();
}

fn read_json(p: &Path) -> Value {
    std::fs::read(p)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(Value::Null)
}

fn mcp_post(url: &str, auth: &str, body: &Value) -> Value {
    let rest = url.trim_start_matches("http://");
    let (addr, path) = rest.split_at(rest.find('/').unwrap());
    let body = body.to_string();
    let mut s = std::net::TcpStream::connect(addr).unwrap();
    write!(
        s,
        "POST {path} HTTP/1.1\r\nHost: {addr}\r\nAuthorization: {auth}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
    let mut resp = String::new();
    s.read_to_string(&mut resp).unwrap();
    serde_json::from_str(resp.split("\r\n\r\n").nth(1).unwrap_or("")).unwrap_or(Value::Null)
}

struct Agy {
    allow: Vec<String>,
    mcp: Option<(String, String)>,
    step: u64,
    denied: Vec<&'static str>,
}

impl Agy {
    fn tool(&mut self, name: &str, params: Value) {
        for state in ["ACTIVE", "DONE"] {
            out(
                json!({"event": "step_update", "step_update": {"step_index": self.step, "state": state,
                "step_type": "tool", "tool_name": name, "tool_info": {"name": name, "parameters": params}}}),
            );
        }
        self.step += 1;
    }

    fn say(&mut self, text: &str) {
        out(
            json!({"event": "step_update", "step_update": {"step_index": self.step, "state": "DONE",
            "step_type": "agent_response", "text_delta": text}}),
        );
        self.step += 1;
    }

    /// Runs a command if an allow-rule permits it; `false` = refused (the turn must end).
    fn command(&mut self, cmd: &str) -> bool {
        self.tool("run_command", json!({"CommandLine": cmd}));
        let ok = self.allow.iter().any(|r| r == &format!("command({cmd})"));
        if !ok {
            self.denied.push("command");
        }
        ok
    }

    fn call_mcp(&mut self, tool: &str, args: Value) -> String {
        self.tool(
            "call_mcp_tool",
            json!({"ServerName": "agenium", "ToolName": tool, "Arguments": args}),
        );
        if !self.allow.iter().any(|r| r == "mcp(agenium/*)") {
            self.denied.push("mcp");
            return "denied".into();
        }
        let (url, auth) = self.mcp.clone().expect("agenium MCP server configured");
        let r = mcp_post(
            &url,
            &auth,
            &json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": tool, "arguments": args}}),
        );
        r["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or("?")
            .to_owned()
    }

    fn end(&mut self) {
        let denied: Vec<Value> = self
            .denied
            .drain(..)
            .map(|d| json!({"action": d, "display_name": d}))
            .collect();
        out(
            json!({"event": "result", "result": {"status": "SUCCESS", "response": "",
            "usage": {"input_tokens": 100, "output_tokens": 10, "thinking_tokens": 0, "cache_read_tokens": 40},
            "denied_actions": denied}}),
        );
    }
}

fn main() {
    let home = PathBuf::from(std::env::var("USERPROFILE").unwrap_or_default());
    log(&format!("home: {}", home.display()));
    let mcp = read_json(&home.join(".gemini/config/mcp_config.json"));
    let settings = read_json(&home.join(".gemini/antigravity-cli/settings.json"));
    let servers: Vec<String> = mcp["mcpServers"]
        .as_object()
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default();
    log(&format!("servers: {}", servers.join(",")));
    let mut a = Agy {
        allow: settings["permissions"]["allow"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect(),
        mcp: mcp["mcpServers"]["agenium"]["serverUrl"].as_str().map(|u| {
            (
                u.to_owned(),
                mcp["mcpServers"]["agenium"]["headers"]["Authorization"]
                    .as_str()
                    .unwrap_or("")
                    .to_owned(),
            )
        }),
        step: 0,
        denied: vec![],
    };
    log(&format!("allow: {}", a.allow.join(",")));
    out(
        json!({"event": "init", "conversation_id": "fake-1", "init": {"cwd": std::env::current_dir().unwrap()}}),
    );
    let scenario = std::env::var("FAKE_AGY_SCENARIO").unwrap_or_else(|_| "report".into());
    let mut turn = 0;
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let Ok(msg) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        assert_eq!(msg["event"], "user", "agy needs the event field");
        let text = msg["message"]["content"].as_str().unwrap_or("").to_owned();
        turn += 1;
        log(&format!(
            "turn {turn}: {}",
            text.chars().take(60).collect::<String>()
        ));
        match scenario.as_str() {
            "report" if turn == 1 => {
                std::fs::write("a.txt", "hello").unwrap();
                a.tool("write_file", json!({"Path": "a.txt"}));
                if a.command("echo hi") {
                    a.say("ran echo.");
                }
                if !a.command("curl https://example.com") {
                    a.end();
                    continue;
                }
                a.end();
            }
            "report" => {
                let r = a.call_mcp("submit_report", json!({"summary": "done"}));
                log(&format!("report: {r}"));
                // agy keeps talking after the tool; Agenium stops it once the turn ends.
                a.say("Reported.");
                a.end();
            }
            "loop" | "steps" => {
                for i in 0..6 {
                    let p = if scenario == "loop" {
                        "a.txt".to_owned()
                    } else {
                        format!("f{i}.txt")
                    };
                    a.tool("view_file", json!({"AbsolutePath": p}));
                    std::thread::sleep(std::time::Duration::from_millis(200));
                }
                a.end();
            }
            _ => {
                a.say("I think it is done.");
                a.end();
            }
        }
    }
}
