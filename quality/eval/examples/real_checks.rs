//! The 2026-10-26 features on a real model (ADR-97..105), one check each, printed as PASS/FAIL with what was seen.
//!
//! cargo run -p agenium-eval --example real_checks -- --root <root with the provider> --provider <id> --model <id>
//!
//! The root only supplies the provider (its key stays in that root's vault); everything the checks write lives in
//! temporary folders. Needs `target/debug/agenium-mcp-oauth-test-server` (built by `cargo test -p agenium-mcp`).

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use agenium_backend::*;
use agenium_core::{Effort, RoleKind};
use agenium_llm::*;
use agenium_tools::{AllowAll, Tool, ToolCtx, ToolError};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde_json::{json, Value};

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

/// `submit_report {answer}` ends the run and keeps the answer.
#[derive(Default)]
struct Report(Mutex<Option<String>>);

#[async_trait]
impl Protocol for Report {
    async fn call(&self, name: &str, args: Value, _id: &str) -> Result<(Value, Effect), ToolError> {
        match name {
            "submit_report" => {
                *self.0.lock().unwrap() = Some(args["answer"].as_str().unwrap_or("").to_owned());
                Ok((json!("report accepted"), Effect::End))
            }
            _ => Err(ToolError::new("unknown_tool", name.to_owned())),
        }
    }
}

/// Counts what the checks look for and can put a message in the run's queue at its first step.
#[derive(Default)]
struct Watch {
    compacted: AtomicUsize,
    tools: Mutex<Vec<String>>,
    steer: Mutex<Option<(Steer, String)>>,
}

impl Observer for Watch {
    fn message(&self, m: &Message) {
        if m.text().starts_with("[Context compacted") {
            self.compacted.fetch_add(1, Ordering::SeqCst);
        }
    }
    fn tool(&self, name: &str, _args: &Value, _r: &std::result::Result<String, ToolError>) {
        self.tools.lock().unwrap().push(name.to_owned());
    }
    fn step(&self, _step: u32, _u: &Usage) {
        if let Some((q, text)) = self.steer.lock().unwrap().take() {
            q.lock().unwrap().push(text);
        }
    }
}

struct Model {
    provider: Arc<dyn Provider>,
    model: String,
}

impl Model {
    fn cfg(&self) -> LoopConfig {
        LoopConfig {
            model: self.model.clone(),
            effort: Effort::Low,
            max_tokens: 8000,
            max_steps: 30,
            ..Default::default()
        }
    }
}

/// One agent loop on `dir` with the file tools (+ `extra`), the task, and `cfg`.
async fn agent(
    m: &Model,
    dir: &Path,
    task: &str,
    extra: Vec<Arc<dyn Tool>>,
    cfg: LoopConfig,
    watch: &Watch,
    steer: Option<Steer>,
) -> (RunReport, Option<String>) {
    let ctx = ToolCtx::new(dir, "real-check", Arc::new(AllowAll));
    let mut tools: Vec<Arc<dyn Tool>> = agenium_tools::work_tools()
        .into_iter()
        .filter(|t| {
            matches!(
                t.name(),
                "read_file" | "list_dir" | "edit_file" | "write_file" | "glob" | "grep"
            )
        })
        .collect();
    tools.extend(extra);
    let mut specs = work_specs(&tools);
    specs.push(ToolSpec {
        name: "submit_report".into(),
        description: "Finish: give your answer.".into(),
        parameters: json!({"type": "object", "properties": {"answer": {"type": "string"}}, "required": ["answer"]}),
    });
    let report = Report::default();
    let r = Run {
        provider: m.provider.as_ref(),
        tools: &tools,
        ctx: &ctx,
        protocol: &report,
        specs: sorted_specs(specs),
        protocol_names: vec!["submit_report".into()],
        cfg,
        observer: watch,
        cancel: Cancel::default(),
        chat: false,
        nudge: None,
        steer,
    };
    let order = WorkOrder {
        system: vec![SystemBlock {
            text: "You are a careful coding agent working in a folder. Use the tools; when done, call submit_report \
                   with your answer."
                .into(),
            cache: false,
        }],
        messages: vec![Message::user_text(task)],
    };
    let rep = r.run(&order).await;
    let answer = report.0.lock().unwrap().clone();
    (rep, answer)
}

fn verdict(name: &str, ok: bool, seen: String) -> bool {
    println!("{} {name}: {seen}", if ok { "PASS" } else { "FAIL" });
    ok
}

fn request(m: &Model, messages: Vec<Message>, tools: Vec<ToolSpec>) -> ChatRequest {
    ChatRequest {
        model: m.model.clone(),
        system: vec![],
        messages,
        tools,
        effort: Effort::Low,
        max_tokens: 2000,
        cache_key: None,
        long_cache: false,
    }
}

/// A plain HTTP/1.1 GET on this machine; returns the `Location` header if any.
fn get_location(url: &str) -> Option<String> {
    let rest = url.strip_prefix("http://")?;
    let (host, path) = rest.split_once('/').map(|(h, p)| (h, format!("/{p}")))?;
    let mut s = std::net::TcpStream::connect(host).ok()?;
    write!(
        s,
        "GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n"
    )
    .ok()?;
    let mut buf = String::new();
    s.read_to_string(&mut buf).ok()?;
    buf.lines()
        .find_map(|l| l.strip_prefix("Location: ").map(str::to_owned))
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let provider_id = arg(&args, "--provider").ok_or_else(|| anyhow!("--provider"))?;
    let model_id = arg(&args, "--model").ok_or_else(|| anyhow!("--model"))?;
    let only = arg(&args, "--only");
    // Either a root whose configured provider (key in its vault) is used, or a provider built here from
    // `--base-url` and the key in the environment variable `--key-env` (a test key that must not be stored).
    let provider: Arc<dyn Provider> = match arg(&args, "--base-url") {
        Some(base) => {
            let var =
                arg(&args, "--key-env").ok_or_else(|| anyhow!("--key-env with --base-url"))?;
            let key = std::env::var(&var).map_err(|_| anyhow!("{var} is not set"))?;
            build_provider(&ProviderSpec {
                id: provider_id.clone(),
                kind: arg(&args, "--kind").unwrap_or_else(|| "openai_compat".into()),
                base_url: Some(base),
                api_key: Some(key),
                max_concurrency: 2,
            })?
        }
        None => {
            let root =
                PathBuf::from(arg(&args, "--root").ok_or_else(|| anyhow!("--root or --base-url"))?);
            let (app, _) = agenium_orchestrator::App::open(
                &root,
                agenium_orchestrator::AppOptions {
                    keyring: Arc::new(agenium_secrets::OsKeyring::default()),
                    checks: Arc::new(agenium_orchestrator::NoCheckFactory),
                },
            )?;
            let source = agenium_orchestrator::runtime::ConfiguredProviders::new(Arc::new(app));
            agenium_orchestrator::runtime::ProviderSource::provider(&source, &provider_id)?
        }
    };
    let m = Model {
        provider,
        model: model_id.clone(),
    };
    let run = |n: &str| only.as_deref().is_none_or(|o| o == n);
    let mut passed = 0;
    let mut total = 0;
    let mut tally = |ok: bool| {
        total += 1;
        passed += usize::from(ok);
    };
    let noop = |_: ChatEvent| {};

    // 1. ADR-103: the provider's own overflow wording is recognised.
    if run("overflow") {
        let info = m
            .provider
            .list_models()
            .await
            .unwrap_or_default()
            .into_iter()
            .find(|i| i.id == m.model);
        // `--overflow-tokens N` when the provider does not report its window.
        let forced = arg(&args, "--overflow-tokens").and_then(|v| v.parse::<u64>().ok());
        match info.and_then(|i| i.context_window).or(forced) {
            Some(cw) => {
                let text = "lorem ".repeat((cw as usize) * 5 / 6 + 1);
                let r = m
                    .provider
                    .chat(&request(&m, vec![Message::user_text(text)], vec![]), &noop)
                    .await;
                tally(verdict(
                    "overflow wording recognised",
                    // By its wording, or a wordless 400/413 on a request this large (the loop's rule, ADR-103).
                    matches!(&r, Err(e) if is_context_overflow(e)
                        || matches!(e, LlmError::Http { status: 400 | 413, message, .. }
                            if !message.to_ascii_lowercase().contains("error") && message.len() < 200)),
                    match &r {
                        Ok(_) => format!("accepted a {cw}-token-window overflow"),
                        Err(e) => e.to_string().chars().take(300).collect(),
                    },
                ));
            }
            None => println!("SKIP overflow: the provider does not report the context window"),
        }
    }

    // 2. ADR-101: a history with a tool call that never got a result.
    if run("dangling") {
        let tool = ToolSpec {
            name: "read_file".into(),
            description: "Read a file.".into(),
            parameters: json!({"type": "object", "properties": {"path": {"type": "string"}}}),
        };
        let history = vec![
            Message::user_text("Read a.txt."),
            Message {
                role: Role::Assistant,
                parts: vec![Part::ToolCall {
                    id: "call_dangling|1".into(),
                    name: "read_file".into(),
                    args: json!({"path": "a.txt"}),
                    signature: None,
                }],
            },
            Message::user_text("Never mind. Reply with the single word OK."),
        ];
        let raw = m
            .provider
            .chat(&request(&m, history.clone(), vec![tool.clone()]), &noop)
            .await;
        let fixed = m
            .provider
            .chat(
                &request(&m, history::prepare(&history, &m.model), vec![tool]),
                &noop,
            )
            .await;
        tally(verdict(
            "dangling tool call sent after the history pass",
            fixed.is_ok(),
            format!(
                "as stored: {}; prepared: {}",
                match &raw {
                    Ok(_) => "accepted".to_owned(),
                    Err(e) => format!(
                        "refused ({})",
                        e.to_string().chars().take(120).collect::<String>()
                    ),
                },
                match &fixed {
                    Ok(r) => format!("answered {:?}", r.message.text().trim()),
                    Err(e) => format!("refused ({e})"),
                }
            ),
        ));
    }

    // 3. ADR-100/103: a run that outgrows a small window clears, compacts and still answers right.
    if run("compaction") {
        let tmp = tempfile::tempdir()?;
        let numbers = [17, 4, 29, 8, 13, 21];
        for (i, n) in numbers.iter().enumerate() {
            let filler: String = (0..120)
                .map(|k| format!("filler line {k} of part {} with nothing in it\n", i + 1))
                .collect();
            std::fs::write(
                tmp.path().join(format!("part{}.txt", i + 1)),
                format!("{filler}NUMBER: {n}\n"),
            )?;
        }
        let watch = Watch::default();
        let cfg = LoopConfig {
            context_window: 14_000,
            ..m.cfg()
        };
        let (rep, answer) = agent(
            &m,
            tmp.path(),
            "Files part1.txt to part6.txt each end with a line `NUMBER: n`. Read them one at a time, one read_file per \
             step, and keep a running total. Then call submit_report with the sum as the answer.",
            vec![],
            cfg,
            &watch,
            None,
        )
        .await;
        let sum: i32 = numbers.iter().sum();
        let ok = answer
            .as_deref()
            .is_some_and(|a| a.contains(&sum.to_string()));
        tally(verdict(
            "compaction keeps the work going",
            ok && watch.compacted.load(Ordering::SeqCst)
                + usize::from(format!("{:?}", rep.messages).contains("[cleared to save context"))
                > 0,
            format!(
                "outcome {:?}, steps {}, compactions {}, answer {:?} (expected {sum})",
                rep.outcome,
                rep.steps,
                watch.compacted.load(Ordering::SeqCst),
                answer
            ),
        ));
    }

    // 4. ADR-102: a CRLF file with a BOM and typographic quotes, edited by the model.
    if run("edit") {
        let tmp = tempfile::tempdir()?;
        let f = tmp.path().join("Greeter.cs");
        std::fs::write(
            &f,
            "\u{feff}class Greeter\r\n{\r\n    string Hi() => \u{201C}Hello, world\u{201D};\r\n}\r\n",
        )?;
        let watch = Watch::default();
        let (rep, _) = agent(
            &m,
            tmp.path(),
            "In Greeter.cs, change the greeting text from Hello, world to Xin chao, the world. Read the file, then use \
             edit_file. Then call submit_report.",
            vec![],
            m.cfg(),
            &watch,
            None,
        )
        .await;
        let after = std::fs::read_to_string(&f)?;
        let crlf = !after.replace("\r\n", "").contains('\n');
        tally(verdict(
            "edit keeps BOM and CRLF",
            after.starts_with('\u{feff}') && crlf && after.contains("Xin chao"),
            format!("outcome {:?}, file now {:?}", rep.outcome, after),
        ));
    }

    // 5. ADR-97: the model calls a tool of an MCP server that needed OAuth.
    if run("mcp") {
        let exe = std::env::current_dir()?.join(if cfg!(windows) {
            "target/debug/agenium-mcp-oauth-test-server.exe"
        } else {
            "target/debug/agenium-mcp-oauth-test-server"
        });
        let mut child = std::process::Command::new(&exe)
            .arg("--auto-approve")
            .stdout(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| anyhow!("{}: {e}", exe.display()))?;
        let mut line = String::new();
        std::io::BufRead::read_line(
            &mut std::io::BufReader::new(child.stdout.take().unwrap()),
            &mut line,
        )?;
        let url = line.trim().trim_start_matches("listening ").to_owned();
        let tmp = tempfile::tempdir()?;
        let hub = agenium_mcp::McpHub::new(Arc::new(|_: &str| None), tmp.path())
            .with_tokens(Arc::new(agenium_mcp::MemoryTokens::default()));
        let cfg = agenium_config::McpServerConfig {
            id: "demo".into(),
            command: None,
            args: vec![],
            env: Default::default(),
            url: Some(url),
            headers: Default::default(),
            timeout_secs: 20,
            oauth: None,
        };
        let browser = |u: &str| {
            let u = u.to_owned();
            std::thread::spawn(move || {
                if let Some(cb) = get_location(&u) {
                    let _ = get_location(&cb);
                }
            });
        };
        let login = hub.login(&cfg, &[], &browser).await;
        let binding = agenium_mcp::Binding {
            server: "demo".into(),
            credentials: Default::default(),
            keys: Default::default(),
            tools: None,
        };
        let t = hub.tools_for("eval", &[binding], &[cfg]).await;
        let extra: Vec<Arc<dyn Tool>> =
            t.tools.iter().map(|x| x.clone() as Arc<dyn Tool>).collect();
        let watch = Watch::default();
        let (rep, answer) = agent(
            &m,
            tmp.path(),
            "Call the MCP tool that tells which client made the call (whoami), then call submit_report with exactly \
             what it returned.",
            extra,
            m.cfg(),
            &watch,
            None,
        )
        .await;
        let _ = child.kill();
        tally(verdict(
            "MCP tool behind OAuth used by the model",
            login.is_ok() && answer.as_deref().is_some_and(|a| a.contains("client dyn-")),
            format!(
                "login {}, problems {:?}, outcome {:?}, answer {:?}",
                if login.is_ok() { "ok" } else { "failed" },
                t.problems,
                rep.outcome,
                answer
            ),
        ));
    }

    // 6. ADR-104/105: a message during the run is followed; read-only calls may run together.
    if run("steer") {
        let tmp = tempfile::tempdir()?;
        for n in ["a.txt", "b.txt", "c.txt"] {
            std::fs::write(tmp.path().join(n), format!("{n} content\n"))?;
        }
        let q: Steer = Default::default();
        let watch = Watch::default();
        *watch.steer.lock().unwrap() = Some((
            q.clone(),
            "Also end your report with the word BANANA.".into(),
        ));
        let (rep, answer) = agent(
            &m,
            tmp.path(),
            "Read a.txt, b.txt and c.txt (you may read them in one step), then call submit_report with a one-line \
             summary of them.",
            vec![],
            m.cfg(),
            &watch,
            Some(q),
        )
        .await;
        tally(verdict(
            "a message sent during the run is followed",
            answer.as_deref().is_some_and(|a| a.contains("BANANA")),
            format!(
                "outcome {:?}, tools {:?}, answer {:?}",
                rep.outcome,
                watch.tools.lock().unwrap(),
                answer
            ),
        ));
    }

    // 7. ADR-98: the Lead learns the user's preference from the chat.
    if run("chat") {
        let tmp = tempfile::tempdir()?;
        let (app, _) = agenium_orchestrator::App::open(
            &tmp.path().join("root"),
            agenium_orchestrator::AppOptions {
                keyring: Arc::new(agenium_secrets::MemoryKeyring::default()),
                checks: Arc::new(agenium_orchestrator::NoCheckFactory),
            },
        )?;
        let app = Arc::new(app);
        let p = app.create_project(&agenium_orchestrator::CreateProject {
            name: "shop".into(),
            path: None,
            key_prefix: None,
            template: None,
            folder: None,
            repo: None,
        })?;
        let (_, lead) = app.create_solo(
            p.id,
            &agenium_orchestrator::CreateAgent {
                name: "ada".into(),
                role: "solo".into(),
                role_kind: RoleKind::Lead,
                tags: vec![],
                model: agenium_core::ModelBinding {
                    provider: provider_id.clone(),
                    model: model_id.clone(),
                    effort: Effort::Low,
                    max_tokens: None,
                },
            },
        )?;
        let rt = agenium_orchestrator::runtime::Runtime::new(
            app.clone(),
            Arc::new(agenium_orchestrator::runtime::StaticProviders(vec![(
                provider_id.clone(),
                m.provider.clone(),
            )])),
            Arc::new(agenium_orchestrator::runtime::AutoApprove),
        );
        let s = rt
            .chat(
                p.id,
                "Before we start: always answer me in Vietnamese, and this shop project is deployed with `make ship`. \
                 Just acknowledge, nothing else to do yet.",
            )
            .await;
        let learned = rt.learn_from_chat(p.id, true).await;
        let e = app.embedder();
        let facts: Vec<String> = app
            .agent_store(lead.id, e.dim(), e.model_id())?
            .live(20)?
            .into_iter()
            .map(|f| f.text)
            .collect();
        let found = facts
            .iter()
            .any(|f| f.to_lowercase().contains("vietnamese"))
            && facts.iter().any(|f| f.contains("make ship"));
        tally(verdict(
            "the Lead learns from the chat",
            found,
            format!(
                "chat {}, learn {}, facts {facts:?}",
                match &s {
                    Ok(s) => s.outcome.clone(),
                    Err(e) => e.to_string(),
                },
                match &learned {
                    Ok(Some(r)) => format!("added {}", r.added),
                    Ok(None) => "nothing".into(),
                    Err(e) => e.to_string(),
                }
            ),
        ));
    }

    println!("\n{passed}/{total} passed ({model_id})");
    Ok(())
}
