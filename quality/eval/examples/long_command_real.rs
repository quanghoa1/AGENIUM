//! Real check of long commands in an API run (ADR-164). A Solo agent on an API model gets a ticket whose check runs for
//! about 150 s, longer than `shell`'s default timeout (120 s), and prints a token the agent cannot guess. Before
//! ADR-164 the command was killed at the timeout; now it goes on as a job whose end comes to the agent. Checks: the
//! deliverable holds the token, the check ran once (not started again), and the run log shows it going on as a job.
//!
//! OPENCODE_KEY=… cargo run -p agenium-eval --example long_command_real -- [model] [seconds]
//! The key is read from the environment only (an OpenCode Go key; the model defaults to deepseek-v4.1-flash).

use std::sync::Arc;

use agenium_core::agent::{BackendKind, Effort, RoleKind};
use agenium_core::forms::*;
use agenium_core::plan::*;
use agenium_core::*;
use agenium_orchestrator::runtime::{AutoApprove, Runtime, StaticProviders};
use agenium_orchestrator::CallCtx;

const PROVIDER: &str = "opencode-go";

fn check(fails: &mut u32, name: &str, ok: bool, seen: &str) {
    if !ok {
        *fails += 1;
    }
    println!("{} {name}: {seen}", if ok { "PASS" } else { "FAIL" });
}

fn head(s: &str, n: usize) -> String {
    s.chars().take(n).collect::<String>().replace('\n', " | ")
}

/// The project's check for this OS: waits `secs`, prints the token.
fn check_script(secs: u64, token: &str) -> (&'static str, String, &'static str) {
    if cfg!(windows) {
        (
            "slow_check.cmd",
            format!(
                "@echo off\r\necho running the checks...\r\npowershell -NoProfile -Command Start-Sleep -Seconds {secs}\r\necho CHECK PASSED {token}\r\n"
            ),
            // `.\` runs it from PowerShell and cmd alike.
            ".\\slow_check.cmd",
        )
    } else {
        (
            "slow_check.sh",
            format!(
                "#!/bin/sh\necho running the checks...\nsleep {secs}\necho CHECK PASSED {token}\n"
            ),
            "sh slow_check.sh",
        )
    }
}

/// `--scripted`: the whole real path (orchestrator, sandbox, a real command, jobs) with a scripted model in place of a
/// paid one. It runs the check with a 2 s timeout, ends its turn (the run waits for the command), then writes the line
/// the command's end brought it.
fn scripted_model(run: &str) -> agenium_llm::mock::ScriptedProvider {
    use agenium_llm::{Message, Part};
    let p = agenium_llm::mock::ScriptedProvider::new(PROVIDER);
    p.push_calls(
        "",
        vec![(
            "shell",
            serde_json::json!({"command": run, "timeout_ms": 2000}),
        )],
    );
    p.push_text("The check goes on; I wait for its result.");
    p.push(Box::new(|req| {
        let text = |m: &Message| {
            m.parts
                .iter()
                .map(|p| match p {
                    Part::Text { text } => text.clone(),
                    Part::ToolResult { content, .. } => content.clone(),
                    _ => String::new(),
                })
                .collect::<String>()
        };
        let all: String = req.messages.iter().map(text).collect();
        let line = all
            .find("CHECK PASSED tok-")
            .map(|i| all[i..].split_whitespace().take(3).collect::<Vec<_>>().join(" "))
            .unwrap_or_else(|| "no result came".into());
        Ok(agenium_llm::mock::calls_response(
            "",
            vec![(
                "write_file",
                serde_json::json!({"path": "deliverables/check.md", "content": format!("{line}\n")}),
            )],
        ))
    }));
    p
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let scripted = args.first().is_some_and(|a| a == "--scripted");
    let model = args
        .first()
        .cloned()
        .filter(|_| !scripted)
        .unwrap_or_else(|| "deepseek-v4.1-flash".into());
    let secs: u64 = args
        .get(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(if scripted { 10 } else { 150 });
    let run_cmd = check_script(secs, "").2;
    let provider: Arc<dyn agenium_llm::Provider> = if scripted {
        Arc::new(scripted_model(run_cmd))
    } else {
        let key = std::env::var("OPENCODE_KEY").map_err(|_| "set OPENCODE_KEY")?;
        agenium_llm::build_provider(&agenium_llm::ProviderSpec {
            custom: Default::default(),
            id: PROVIDER.into(),
            kind: "openai_compat".into(),
            base_url: Some("https://opencode.ai/zen/go/v1".into()),
            api_key: Some(key),
            max_concurrency: 2,
        })?
    };
    let token = format!(
        "tok-{:x}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    );

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
        name: "slow".into(),
        path: Some(tmp.path().join("slow")),
        key_prefix: Some("SL".into()),
        template: None,
        folder: None,
        repo: None,
    })?;
    let root = std::path::PathBuf::from(&p.path);
    let (file, body, run) = check_script(secs, &token);
    std::fs::write(root.join(file), body)?;
    app.repo(&p)
        .commit_all(&root, "The check", ("eval", "eval@agenium.local"), &[])?;
    let binding = ModelBinding {
        provider: PROVIDER.into(),
        model: model.clone(),
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
    app.set_agent_backend(solo.id, BackendKind::Api, &binding)?;
    let ac = vec![AcceptanceCriterion {
        id: "AC-1".into(),
        text: "deliverables/check.md holds the last line the project's check prints when it runs to its end".into(),
    }];
    let user = CallCtx::user(p.id);
    app.apply(
        &user,
        Intent::ProposeSpec(RequirementSpec {
            goals: vec!["Know the project's check result".into()],
            scope: vec![],
            out_of_scope: vec![],
            requirements: vec![Requirement {
                id: "REQ-1".into(),
                text: "The check's result is recorded".into(),
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
                title: "Check".into(),
            }],
            items: vec![PlanItem {
                id: "PI-1".into(),
                milestone: "M1".into(),
                title: "Run the check".into(),
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
            title: "Record the project's check result".into(),
            description: format!(
                "Run the project's check with `{run}` from the worktree root and write deliverables/check.md with the \
                 last line it prints. Do not change the check."
            ),
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
    let rt = Runtime::new(
        app.clone(),
        Arc::new(StaticProviders(vec![(PROVIDER.into(), provider)])),
        Arc::new(AutoApprove),
    );
    let started = std::time::Instant::now();
    rt.drive(p.id, 6).await?;

    let t = app.ticket(p.id, &key)?;
    println!(
        "ticket {key} on {}: {:?} in {:.0} s (check {secs} s)",
        if scripted { "a scripted model" } else { &model },
        t.status,
        started.elapsed().as_secs_f64()
    );
    let wt = root
        .join(".agenium")
        .join("worktrees")
        .join(key.to_string());
    let read = |rel: &str| {
        std::fs::read_to_string(root.join(rel))
            .or_else(|_| std::fs::read_to_string(wt.join(rel)))
            .unwrap_or_default()
    };
    let text = read("deliverables/check.md");
    let mut log = String::new();
    for r in app
        .store()
        .read(|c| agenium_store::core::runs::for_ticket(c, t.id))?
    {
        println!("  run {}: {:?}", r.id, r.outcome);
        for e in app
            .store()
            .read(|c| agenium_store::core::runs::log_entries(c, r.id))?
        {
            log.push_str(&e.kind);
            log.push(' ');
            log.push_str(&e.data.to_string());
            log.push('\n');
        }
    }
    // Each `shell` call that ran the check: once means it was not killed and started again.
    let starts = log
        .lines()
        .filter(|l| l.contains("\"tool\":\"shell\"") && l.contains("slow_check"))
        .count();
    let mut fails = 0;
    check(
        &mut fails,
        "the deliverable holds the check's last line",
        text.contains(&format!("CHECK PASSED {token}")),
        &head(&text, 200),
    );
    check(
        &mut fails,
        "the check ran once (not killed and started again)",
        starts == 1,
        &format!("{starts} start(s)"),
    );
    let went_on = log.contains("goes on as job");
    let killed = log.contains("timed out after");
    check(
        &mut fails,
        "it outlasted the call and went on as a job, not killed",
        went_on && !killed,
        &format!("went on: {went_on}, killed: {killed}"),
    );
    if fails > 0 {
        for l in log.lines().rev().take(12) {
            println!("  run log: {}", head(l, 300));
        }
    }
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
