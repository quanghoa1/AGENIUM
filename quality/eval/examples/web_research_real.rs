//! Real check of reading the web (ADR-163).
//! 1. `fetch` on the open internet (no allowlist): a documentation page reads as Markdown with absolute links and
//!    without the site's navigation; a PDF reads as its text; the private network is still refused.
//! 2. With a CLI: a Solo agent gets a ticket that needs a fact newer than its training (ripgrep's latest release on
//!    GitHub, checked here against GitHub's API). It must search (Claude Code and Codex with their own search, other
//!    CLIs with Agenium's `web_search`), read without asking for network access, and cite the URL.
//!
//! cargo run -p agenium-eval --example web_research_real -- [claude|codex] [model]
//! cargo run -p agenium-eval --example web_research_real -- --fetch-only

use std::sync::Arc;

use agenium_core::agent::{BackendKind, Effort, RoleKind};
use agenium_core::comment::CommentBody;
use agenium_core::forms::*;
use agenium_core::plan::*;
use agenium_core::*;
use agenium_orchestrator::CallCtx;
use agenium_tools::Tool;

fn check(fails: &mut u32, name: &str, ok: bool, seen: &str) {
    if !ok {
        *fails += 1;
    }
    println!("{} {name}: {seen}", if ok { "PASS" } else { "FAIL" });
}

fn head(s: &str, n: usize) -> String {
    s.chars().take(n).collect::<String>().replace('\n', " | ")
}

struct Allow;
#[async_trait::async_trait]
impl agenium_tools::CommandGate for Allow {
    async fn check(&self, _: &str, _: &std::path::Path) -> agenium_tools::GateDecision {
        agenium_tools::GateDecision::Allow
    }
}

async fn fetch_checks(fails: &mut u32) -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let mut ctx = agenium_tools::ToolCtx::new(tmp.path(), "web", Arc::new(Allow));
    ctx.fetch_open = true;
    let fetch = agenium_tools::FetchTool;
    let page = fetch
        .call(
            &ctx,
            serde_json::json!({"url": "https://doc.rust-lang.org/std/vec/struct.Vec.html"}),
        )
        .await?;
    let t = &page.text;
    check(
        fails,
        "a documentation page reads as Markdown, links absolute",
        t.contains("HTTP 200")
            && t.contains("Vec")
            && t.contains("](https://doc.rust-lang.org/")
            && !t.contains("<div"),
        &head(t, 300),
    );
    let pdf = fetch
        .call(
            &ctx,
            serde_json::json!({"url": "https://www.rfc-editor.org/rfc/rfc9110.pdf"}),
        )
        .await?;
    check(
        fails,
        "a PDF reads as its text",
        pdf.text.contains("HTTP Semantics") && !pdf.text.contains("%PDF-"),
        &head(&pdf.text, 300),
    );
    let private = fetch
        .call(
            &ctx,
            serde_json::json!({"url": "http://169.254.169.254/latest/meta-data/"}),
        )
        .await;
    // `web_search` for the agents whose CLI does not search: Exa's free search, and each key's API on its own when
    // keys are given (AGENIUM_TEST_SEARCH_KEYS, comma-separated, from the environment only; never printed).
    let mut keys = vec![String::new()];
    if let Ok(k) = std::env::var("AGENIUM_TEST_SEARCH_KEYS") {
        keys.extend(
            k.split(',')
                .map(|k| k.trim().to_owned())
                .filter(|k| !k.is_empty()),
        );
    }
    for key in keys {
        let tool = agenium_tools::WebSearchTool::with(vec![agenium_tools::Engine::new(&key)]);
        let api = agenium_tools::SearchApi::of_key(&key).name();
        let r = tool
            .call(
                &ctx,
                serde_json::json!({"query": "ripgrep latest release BurntSushi GitHub", "max_results": 4, "recent": "year"}),
            )
            .await;
        let mut seen = match &r {
            Ok(o) => head(&o.text, 400),
            Err(e) => format!("{}: {}", e.code, e.message),
        };
        if !key.is_empty() {
            seen = seen.replace(key.as_str(), "<KEY>");
        }
        check(
            fails,
            &format!(
                "web_search through {api}{} finds ripgrep's releases",
                if key.is_empty() {
                    " (free, no key)"
                } else {
                    " (key)"
                }
            ),
            r.as_ref()
                .is_ok_and(|o| o.text.contains("github.com/BurntSushi/ripgrep")),
            &seen,
        );
    }
    check(
        fails,
        "the private network is still refused",
        private
            .as_ref()
            .is_err_and(|e| e.code == "address_not_allowed"),
        &format!("{:?}", private.err().map(|e| e.code)),
    );
    Ok(())
}

/// ripgrep's latest release tag, from GitHub's API (read with `fetch` itself): what the agent must find.
async fn latest_ripgrep() -> Result<String, Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let mut ctx = agenium_tools::ToolCtx::new(tmp.path(), "api", Arc::new(Allow));
    ctx.fetch_open = true;
    let out = agenium_tools::FetchTool
        .call(
            &ctx,
            serde_json::json!({"url": "https://api.github.com/repos/BurntSushi/ripgrep/releases/latest"}),
        )
        .await?;
    let at = out.text.find("\"tag_name\"").ok_or("no tag_name")?;
    let rest = &out.text[at + "\"tag_name\"".len()..];
    let tag = rest.split('"').nth(1).ok_or("no tag")?;
    Ok(tag.to_owned())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut fails = 0;
    fetch_checks(&mut fails).await?;
    if args.iter().any(|a| a == "--fetch-only") {
        println!(
            "{}",
            if fails == 0 {
                "ALL PASS"
            } else {
                "SOME FAILED"
            }
        );
        std::process::exit(i32::from(fails > 0));
    }
    let cli = args.first().cloned().unwrap_or_else(|| "claude".into());
    let model = args.get(1).cloned().unwrap_or_else(|| {
        if cli == "claude" {
            "haiku"
        } else {
            "gpt-6-luna"
        }
        .into()
    });
    let latest = latest_ripgrep().await?;
    println!("ripgrep's latest release on GitHub: {latest}");

    let tmp = tempfile::tempdir()?;
    let (app, _) = agenium_orchestrator::App::open(
        &tmp.path().join("root"),
        agenium_orchestrator::AppOptions {
            keyring: Arc::new(agenium_secrets::MemoryKeyring::default()),
            checks: Arc::new(agenium_orchestrator::NoCheckFactory),
        },
    )?;
    let app = Arc::new(app);
    app.update_config(|c| c.memory.learn = false)?;
    let p = app.create_project(&agenium_orchestrator::CreateProject {
        name: "research".into(),
        path: Some(tmp.path().join("research")),
        key_prefix: Some("RS".into()),
        template: None,
        folder: None,
        repo: None,
    })?;
    let binding = ModelBinding {
        provider: cli.clone(),
        model,
        effort: Effort::LOW,
        max_tokens: None,
    };
    let (_, solo) = app.create_solo(
        p.id,
        &agenium_orchestrator::CreateAgent {
            name: "ada".into(),
            role: "solo".into(),
            role_kind: RoleKind::Lead,
            tags: vec![],
            model: binding.clone(),
        },
    )?;
    app.set_agent_backend(solo.id, BackendKind::Cli, &binding)?;
    let ac = vec![AcceptanceCriterion {
        id: "AC-1".into(),
        text: "deliverables/ripgrep.md gives ripgrep's latest release version as published today, and the URL of the \
               page it was read from"
            .into(),
    }];
    let user = CallCtx::user(p.id);
    app.apply(
        &user,
        Intent::ProposeSpec(RequirementSpec {
            goals: vec!["Know ripgrep's current release".into()],
            scope: vec![],
            out_of_scope: vec![],
            requirements: vec![Requirement {
                id: "REQ-1".into(),
                text: "ripgrep's latest release is recorded with its source".into(),
                acceptance: ac.clone(),
            }],
            constraints: vec![],
        }),
    )?;
    app.apply(
        &user,
        Intent::CreatePlan(TicketPlan {
            spec_version: 1,
            milestones: vec![Milestone {
                id: "M1".into(),
                title: "Research".into(),
            }],
            items: vec![PlanItem {
                id: "PI-1".into(),
                milestone: "M1".into(),
                title: "ripgrep release".into(),
                trace_to: vec!["REQ-1".into()],
                acceptance: ac.clone(),
                role: "solo".into(),
                depends_on: vec![],
                estimate: Estimate::S,
                scope_paths: vec![],
            }],
        }),
    )?;
    let out = app.apply(
        &user,
        Intent::CreateTicket(NewTicket {
            kind: TicketKind::Task,
            plan_item: "PI-1".into(),
            parent: None,
            title: "Record ripgrep's latest release".into(),
            description: "Find the latest release of ripgrep (BurntSushi/ripgrep) as published today, not as you \
                          remember it: start with a web search (not a URL you remember), then read the page it leads to. Write \
                          deliverables/ripgrep.md with the version and the URL of the page you \
                          read it on."
                .into(),
            acceptance: ac,
            priority: 2,
            depends_on: vec![],
            required_role: None,
            required_tags: vec![],
            scope_paths: vec![],
            report_template: None,
            follows: None,
        }),
    )?;
    let key: TicketKey = serde_json::from_value(out.result["ticket_id"].clone())?;
    app.apply(
        &user,
        Intent::AssignTicket(AssignTicket {
            ticket_id: key.clone(),
            worker: solo.id,
            note: None,
        }),
    )?;
    let rt = agenium_orchestrator::runtime::Runtime::new(
        app.clone(),
        Arc::new(agenium_orchestrator::runtime::StaticProviders(vec![])),
        Arc::new(agenium_orchestrator::runtime::AutoApprove),
    );
    let started = std::time::Instant::now();
    rt.drive(p.id, 8).await?;

    let t = app.ticket(p.id, &key)?;
    let wt = std::path::Path::new(&p.path)
        .join(".agenium")
        .join("worktrees")
        .join(key.to_string())
        .join("deliverables")
        .join("ripgrep.md");
    let on_main = std::path::Path::new(&p.path)
        .join("deliverables")
        .join("ripgrep.md");
    let text = std::fs::read_to_string(&on_main)
        .or_else(|_| std::fs::read_to_string(&wt))
        .unwrap_or_default();
    let comments = app
        .store()
        .read(|c| agenium_store::core::comments::for_ticket(c, t.id))?;
    let asked_network = comments
        .iter()
        .any(|c| matches!(c.body, CommentBody::Permission { .. }));
    // Every logged step of the ticket's runs: the tool calls among them.
    let mut calls = String::new();
    for r in app
        .store()
        .read(|c| agenium_store::core::runs::for_ticket(c, t.id))?
    {
        let log = app
            .store()
            .read(|c| agenium_store::core::runs::log_entries(c, r.id))?;
        if t.status != TicketStatus::Closed {
            // Why a run that did not finish the ticket ended.
            for e in log.iter().rev().take(3) {
                println!("  run log: {} {}", e.kind, head(&e.data.to_string(), 300));
            }
        }
        for e in log {
            calls.push_str(&e.data.to_string());
            calls.push('\n');
        }
    }
    println!(
        "ticket {key}: {:?} in {:.0} s",
        t.status,
        started.elapsed().as_secs_f64()
    );
    let bare = latest.trim_start_matches('v');
    check(
        &mut fails,
        "the deliverable gives today's release",
        text.contains(bare),
        &head(&text, 300),
    );
    check(
        &mut fails,
        "and the URL it read it on",
        text.contains("https://"),
        "",
    );
    // A logged call of the tool (`"tool": "cli:WebSearch"` for a CLI's own, the bare name for Agenium's).
    let call_of = |name: &str| {
        calls
            .lines()
            .find(|l| {
                serde_json::from_str::<serde_json::Value>(l)
                    .ok()
                    .and_then(|v| v.as_object().cloned())
                    .and_then(|o| o.get("tool").and_then(|t| t.as_str()).map(str::to_owned))
                    .is_some_and(|t| t == format!("cli:{name}") || t == name)
            })
            .map(|l| head(l, 160))
    };
    let searched = call_of("WebSearch").or_else(|| call_of("web_search"));
    check(
        &mut fails,
        "the agent searched the web (its CLI's own search, or Agenium's web_search)",
        searched.is_some(),
        &searched.clone().unwrap_or_else(|| {
            // What the log holds instead, to see its shape.
            calls
                .lines()
                .find(|l| l.contains("WebSearch") || l.contains("web_search"))
                .map(|l| {
                    let mut v: serde_json::Value = serde_json::from_str(l).unwrap_or_default();
                    if let Some(o) = v.as_object_mut() {
                        o.remove("args");
                        o.remove("result");
                    }
                    format!("not as a call; seen: {}", head(&v.to_string(), 300))
                })
                .unwrap_or_default()
        }),
    );
    check(
        &mut fails,
        "it read the web without asking for network access",
        !asked_network,
        &format!("fetch used: {}", calls.contains("fetch")),
    );
    println!(
        "{}",
        if fails == 0 {
            "ALL PASS"
        } else {
            "SOME FAILED"
        }
    );
    std::process::exit(i32::from(fails > 0));
}
