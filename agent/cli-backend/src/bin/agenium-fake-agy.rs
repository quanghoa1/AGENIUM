//! Scripted stand-in for `agy` in headless stream-json mode, used by agenium-cli-backend's tests (not shipped).
//! It reads its MCP servers and hooks from `$USERPROFILE/.gemini` exactly where agy does, so the tests prove the
//! per-run home is used; like agy in YOLO mode it runs every `PreToolUse` hook before a tool and skips the tool when
//! the hook denies it or fails. `FAKE_AGY_SCENARIO`: `report` (default), `chatty`, `loop`, `steps`, `steer`, `slow`
//! (a long command through Agenium's shell: `FAKE_AGY_SLOW`), `approval` (a command through Agenium's shell that the
//! test's gate never answers), `slowjob` (a command longer than the run's foreground limit: `FAKE_AGY_JOB`), `relogin` (its first turn refused for
//! its login). What it
//! saw goes to `fake-agy.log` in its cwd.

use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

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
    config_dir: PathBuf,
    hooks: Vec<String>,
    mcp: Option<(String, String)>,
    step: u64,
}

impl Agy {
    /// Runs the PreToolUse hooks; `Err(reason)` = the tool must not run.
    fn pre_tool(&self, name: &str, args: &Value) -> Result<(), String> {
        let payload = json!({"toolCall": {"name": name, "args": args}, "stepIdx": self.step,
            "conversationId": "fake-1", "workspacePaths": [std::env::current_dir().unwrap()]});
        for cmd in &self.hooks {
            #[cfg(windows)]
            let mut c = {
                use std::os::windows::process::CommandExt;
                let mut c = Command::new("cmd");
                c.arg("/C").raw_arg(cmd);
                c
            };
            #[cfg(not(windows))]
            let mut c = {
                let mut c = Command::new("sh");
                c.arg("-c").arg(cmd);
                c
            };
            let mut child = c
                .current_dir(&self.config_dir)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|e| format!("hook failed to start: {e}"))?;
            child
                .stdin
                .take()
                .unwrap()
                .write_all(payload.to_string().as_bytes())
                .unwrap();
            let o = child.wait_with_output().unwrap();
            if !o.status.success() {
                return Err(format!(
                    "hook failed: {}",
                    String::from_utf8_lossy(&o.stdout)
                ));
            }
            let answer: Value = serde_json::from_slice(&o.stdout).unwrap_or(Value::Null);
            if answer["decision"] != "allow" {
                return Err(answer["reason"].as_str().unwrap_or("denied").to_owned());
            }
        }
        Ok(())
    }

    /// A tool step whose hook failed before it reached Agenium (like agy, the tool is blocked and the step still ends).
    fn hook_failed(&mut self, name: &str, params: Value) {
        log(&format!("blocked {name}: its hook failed"));
        for state in ["ACTIVE", "DONE"] {
            out(
                json!({"event": "step_update", "step_update": {"step_index": self.step, "state": state,
                "step_type": "tool", "tool_name": name, "tool_info": {"name": name, "parameters": params}}}),
            );
        }
        self.step += 1;
    }

    /// One tool step; `false` when the hook refused it.
    fn tool(&mut self, name: &str, params: Value) -> bool {
        if let Err(why) = self.pre_tool(name, &params) {
            log(&format!("denied {name}: {why}"));
            self.step += 1;
            return false;
        }
        for state in ["ACTIVE", "DONE"] {
            out(
                json!({"event": "step_update", "step_update": {"step_index": self.step, "state": state,
                "step_type": "tool", "tool_name": name, "tool_info": {"name": name, "parameters": params}}}),
            );
        }
        self.step += 1;
        true
    }

    fn say(&mut self, text: &str) {
        out(
            json!({"event": "step_update", "step_update": {"step_index": self.step, "state": "DONE",
            "step_type": "agent_response", "text_delta": text}}),
        );
        self.step += 1;
    }

    fn call_mcp(&mut self, tool: &str, args: Value) -> String {
        if !self.tool(
            "call_mcp_tool",
            json!({"ServerName": "agenium", "ToolName": tool, "Arguments": args}),
        ) {
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
        out(
            json!({"event": "result", "result": {"status": "SUCCESS", "response": "",
            "usage": {"input_tokens": 100, "output_tokens": 10, "thinking_tokens": 0, "cache_read_tokens": 40},
            "denied_actions": []}}),
        );
    }
}

fn main() {
    let home = PathBuf::from(std::env::var("USERPROFILE").unwrap_or_default());
    log(&format!("home: {}", home.display()));
    let config_dir = home.join(".gemini/config");
    let mcp = read_json(&config_dir.join("mcp_config.json"));
    let hooks_file = read_json(&config_dir.join("hooks.json"));
    let servers: Vec<String> = mcp["mcpServers"]
        .as_object()
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default();
    log(&format!("servers: {}", servers.join(",")));
    let hooks: Vec<String> = hooks_file
        .as_object()
        .into_iter()
        .flat_map(|o| o.values())
        .flat_map(|h| h["PreToolUse"].as_array().cloned().unwrap_or_default())
        .flat_map(|g| g["hooks"].as_array().cloned().unwrap_or_default())
        .filter_map(|h| h["command"].as_str().map(str::to_owned))
        .collect();
    log(&format!("hooks: {}", hooks.len()));
    let yolo = std::env::args().any(|a| a == "--dangerously-skip-permissions");
    log(&format!("yolo: {yolo}"));
    let mut a = Agy {
        config_dir,
        hooks,
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
    };
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
            // ADR-129: its own file tools reach its scratch folder only and its own shell is off; the worktree is
            // worked on through Agenium's tools over MCP.
            "report" => {
                let target = std::env::current_dir().unwrap().join("a.txt");
                if a.tool(
                    "write_to_file",
                    json!({"TargetFile": target, "Content": "own"}),
                ) {
                    std::fs::write(&target, "own").unwrap();
                }
                let outside = std::env::current_dir().unwrap().join("../escape.txt");
                if a.tool(
                    "write_to_file",
                    json!({"TargetFile": outside, "Content": "x"}),
                ) {
                    std::fs::write(&outside, "x").unwrap();
                }
                let notes = home.join(".gemini/antigravity/brain/c1/task.md");
                if a.tool(
                    "write_to_file",
                    json!({"TargetFile": notes, "Content": "plan"}),
                ) {
                    std::fs::create_dir_all(notes.parent().unwrap()).unwrap();
                    std::fs::write(&notes, "plan").unwrap();
                    log("wrote its notes");
                }
                if a.tool("run_command", json!({"CommandLine": "echo hi"})) {
                    log("ran: echo hi");
                }
                a.tool("read_url_content", json!({"Url": "https://example.com"}));
                // A step whose hook failed: it did not run and must not be logged as if it had.
                a.hook_failed("run_command", json!({"CommandLine": "echo blocked"}));
                let w = a.call_mcp("write_file", json!({"path": "a.txt", "content": "hello"}));
                log(&format!("agenium write_file: {}", w.replace('\n', " ")));
                let r = a.call_mcp("submit_report", json!({"summary": "done"}));
                log(&format!("report: {r}"));
                // agy keeps talking after the tool; Agenium stops it once the turn ends.
                a.say("Reported.");
                a.end();
            }
            "loop" | "steps" => {
                for i in 0..6 {
                    let p = if scenario == "loop" {
                        home.join("notes.txt")
                    } else {
                        home.join(format!("f{i}.txt"))
                    };
                    a.tool("view_file", json!({"AbsolutePath": p}));
                    std::thread::sleep(std::time::Duration::from_millis(200));
                }
                a.end();
            }
            // The Lead is messaged while it works: an Agenium tool result, then a command through Agenium's shell
            // (whose gate check is where the test sends a second message), then the turn ends.
            "steer" if turn == 1 => {
                let r = a.call_mcp("note", json!({}));
                log(&format!("note: {}", r.replace('\n', " ")));
                let out = a.call_mcp("shell", json!({"command": "echo hi"}));
                log(&format!("shell: {}", out.replace('\n', " ")));
                a.say("Working on it.");
                a.end();
            }
            // The run is stopped while Agenium's shell runs a long command for it.
            "slow" => {
                let cmd = std::env::var("FAKE_AGY_SLOW").unwrap_or_else(|_| "sleep 120".into());
                let out = a.call_mcp("shell", json!({"command": cmd, "timeout_ms": 300000}));
                log(&format!("slow shell: {}", out.replace('\n', " ")));
                a.end();
            }
            // The run is stopped while a command through Agenium's shell waits for the user's approval.
            "approval" => {
                let out = a.call_mcp("shell", json!({"command": "echo wait"}));
                log(&format!("approval shell: {}", out.replace('\n', " ")));
                a.end();
            }
            // A command longer than the run's foreground limit (`FAKE_AGY_JOB`): it goes on as a job, a report meanwhile
            // is refused, and its end comes as the next turn's message.
            "slowjob" if turn == 1 => {
                let cmd = std::env::var("FAKE_AGY_JOB").unwrap_or_else(|_| "sleep 3".into());
                let out = a.call_mcp("shell", json!({"command": cmd}));
                log(&format!("slowjob shell: {}", out.replace('\n', " ")));
                let r = a.call_mcp("submit_report", json!({"summary": "too early"}));
                log(&format!("early report: {}", r.replace('\n', " ")));
                a.end();
            }
            "slowjob" => {
                log(&format!("result turn: {}", text.replace('\n', " ")));
                let r = a.call_mcp("submit_report", json!({"summary": "done"}));
                log(&format!("report: {r}"));
                a.end();
            }
            // Its login refused once (another run used the refresh token): the first process's turn fails; started
            // again, it reports.
            "relogin" => {
                let mark = std::env::current_dir().unwrap().join("fake-agy.relogin");
                if mark.exists() {
                    log(&format!(
                        "second try: {}",
                        text.chars().take(40).collect::<String>()
                    ));
                    let r = a.call_mcp("submit_report", json!({"summary": "done"}));
                    log(&format!("report: {r}"));
                    a.end();
                } else {
                    std::fs::write(&mark, "").unwrap();
                    out(json!({"event": "result", "result": {"status": "ERROR",
                        "error": "401 Unauthorized: OAuth token has expired", "usage": {}, "denied_actions": []}}));
                }
            }
            _ => {
                a.say("I think it is done.");
                a.end();
            }
        }
    }
}
