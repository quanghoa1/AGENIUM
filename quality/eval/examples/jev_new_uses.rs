//! Measures the two judge uses of ADR-126/127 against the real Jev API, the way Jev is judged (docs/JEV.md): the work
//! taken off the main agents, and that nothing needed is lost.
//!
//! - **Output trimmer**: real tool output from this repository's own runs (`data/jev-trim/`, cut to what the shell
//!   tool returns: 128 head + 128 tail lines) — lines and tokens left out, and every line the work needs (failures,
//!   panics, test results, the definitions a grep was for) still there.
//! - **Done check**: labelled reports with their run facts — sent back when a "done" is unverified and a check would
//!   apply, accepted otherwise; with review_assist on, the review's questions ride in the same request.
//!
//! Usage: `JEV_KEY=… cargo run -p agenium-eval --example jev_new_uses [-- --model jev-1.14.0]`. The key is read from the
//! environment only and kept in a throwaway root's in-memory keyring.

use std::path::Path;
use std::sync::Arc;

use agenium_core::forms::WorkReport;
use agenium_core::*;
use agenium_orchestrator::judge::done_check::run_facts;
use agenium_orchestrator::judge::JudgeInput;
use agenium_store::core::runs::LogEntry;
use anyhow::{anyhow, Result};
use serde::Deserialize;

#[derive(Deserialize)]
struct TrimCase {
    file: String,
    tool: String,
    what: String,
    goal: String,
    must_keep: Vec<String>,
}

fn ticket(n: u64, title: &str, acs: &[&str]) -> Ticket {
    Ticket {
        id: TicketId::new(),
        key: TicketKey::new("EVAL", n).unwrap(),
        project: ProjectId::new(),
        parent: None,
        kind: TicketKind::Task,
        title: title.into(),
        description: String::new(),
        acceptance: acs
            .iter()
            .enumerate()
            .map(|(i, t)| AcceptanceCriterion {
                id: format!("AC-{}", i + 1),
                text: (*t).into(),
            })
            .collect(),
        status: TicketStatus::InProgress,
        assignee: None,
        plan_item: "PI-1".into(),
        priority: 2,
        depends_on: vec![],
        reopen_count: 0,
        escalated: false,
        required_role: None,
        required_tags: vec![],
        scope_paths: vec![],
        report_template: "default".into(),
        approved: false,
        merged: false,
        has_code: true,
        close_reason: None,
        usage: Default::default(),
        created_at: String::new(),
        updated_at: String::new(),
    }
}

fn report(t: &Ticket, summary: &str, evidence: &str) -> WorkReport {
    // A worker that did not finish says so (NotMet); every other report claims done.
    let status = if summary.starts_with("Could not finish") {
        "NotMet"
    } else {
        "Met"
    };
    serde_json::from_value(serde_json::json!({
        "ticket_id": t.key.to_string(),
        "summary": summary,
        "confidence": 0.9,
        "acceptance": t.acceptance.iter().map(|ac| serde_json::json!({"ac_id": ac.id, "status": status, "evidence": evidence})).collect::<Vec<_>>(),
    }))
    .unwrap()
}

fn edit(path: &str) -> LogEntry {
    LogEntry {
        n: 0,
        kind: "tool".into(),
        data: serde_json::json!({"tool": "edit_file", "args": {"path": path}, "ok": true, "result": "edited"}),
        at: String::new(),
    }
}

fn shell(cmd: &str, code: i64) -> LogEntry {
    LogEntry {
        n: 0,
        kind: "tool".into(),
        data: serde_json::json!({"tool": "shell", "args": {"command": cmd}, "ok": true, "result": format!("exit code {code}\n")}),
        at: String::new(),
    }
}

/// (title, criteria, summary, evidence, run, expected: true = send back, None = either is fine)
type DoneCase = (
    &'static str,
    Vec<&'static str>,
    &'static str,
    &'static str,
    Vec<LogEntry>,
    Option<bool>,
);

fn done_cases() -> Vec<DoneCase> {
    vec![
        ("Fix the off-by-one in the parser", vec!["the parser handles the last token"], "Fixed the off-by-one in the parser; all tests pass.", "src/parser.rs, tests pass", vec![edit("src/parser.rs")], Some(true)),
        ("Add CSV export", vec!["the report can be exported as CSV"], "Implemented the CSV export.", "src/export.rs", vec![edit("src/export.rs"), shell("cargo build", 101)], Some(true)),
        ("Fix the install section of the README", vec!["the README install steps are correct"], "Updated the install section of the README.", "README.md lines 10-24", vec![edit("README.md")], Some(false)),
        ("Validate the signup form", vec!["invalid emails are rejected"], "Added input validation to the signup form; ready for review.", "src/signup.ts", vec![edit("src/signup.ts"), shell("ls src", 0), shell("cat src/signup.ts", 0)], Some(true)),
        ("Handle empty input in the tokenizer", vec!["empty input returns no tokens"], "Handled empty input in the tokenizer.", "src/tokenizer.rs, cargo test passed", vec![edit("src/tokenizer.rs"), shell("cargo test", 0)], Some(false)),
        ("Connect the payment API", vec!["payments go through the API"], "Could not finish: the payment API key is missing, so AC-1 is not met yet. The client code is written but untested.", "src/payment.rs (untested)", vec![edit("src/payment.rs")], Some(false)),
        ("Write the release plan", vec!["the plan lists the release steps"], "Wrote the release plan in notes/release-plan.md.", "notes/release-plan.md", vec![edit("notes/release-plan.md")], Some(false)),
        ("Fix the date bug", vec!["dates render in the user's time zone"], "Fixed the date rendering; the tests pass now.", "src/dates.js; npm test passes", vec![edit("src/dates.js"), shell("npm test", 1), edit("src/dates.js")], Some(true)),
        ("Add parser tests", vec!["the parser has tests for nested lists"], "Added tests for nested lists in the parser.", "tests/test_parser.py", vec![edit("tests/test_parser.py")], Some(true)),
        ("Add a lint step to CI", vec!["CI runs the linter"], "Added the lint step to the CI workflow.", ".github/workflows/ci.yml", vec![edit(".github/workflows/ci.yml")], None),
    ]
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let model = args
        .iter()
        .position(|a| a == "--model")
        .and_then(|i| args.get(i + 1).cloned())
        .unwrap_or_else(|| "jev-1.14.0".into());
    let key = std::env::var("JEV_KEY").map_err(|_| anyhow!("set JEV_KEY"))?;
    let tmp = tempfile::tempdir()?;
    let (app, _) = agenium_orchestrator::App::open(
        &tmp.path().join("root"),
        agenium_orchestrator::AppOptions {
            keyring: Arc::new(agenium_secrets::MemoryKeyring::default()),
            checks: Arc::new(agenium_orchestrator::NoCheckFactory),
        },
    )?;
    app.set_judge(&JudgeInput {
        base_url: "https://api.typesafe.ai".into(),
        model: model.clone(),
        api_key: Some(key),
        review_assist: true,
        done_check: true,
        trim_output: true,
        ..Default::default()
    })?;
    let usage_of = |app: &agenium_orchestrator::App| -> u64 {
        app.judge_activity(1)
            .first()
            .and_then(|a| a.usage.as_ref())
            .and_then(|u| u["input_tokens"].as_u64())
            .unwrap_or(0)
    };

    // ---------------------------------------------------------------- output trimmer
    let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("data/jev-trim");
    let cases: Vec<TrimCase> =
        serde_json::from_str(&std::fs::read_to_string(data.join("cases.json"))?)?;
    println!("## Output trimmer ({model})\n");
    println!("| Output | lines shown | lines left out | tokens saved (≈ chars/4) | needed lines kept | Jev input tokens | ms |");
    println!("|---|---|---|---|---|---|---|");
    let (mut saved_total, mut jev_total, mut lost_total) = (0usize, 0u64, 0usize);
    for c in &cases {
        let full = std::fs::read_to_string(data.join(&c.file))?;
        let shown = agenium_tools::text::truncate(&full, 30_000, 256).0;
        let needed: Vec<&str> = shown
            .lines()
            .filter(|l| c.must_keep.iter().any(|m| l.contains(m.as_str())))
            .collect();
        let t = std::time::Instant::now();
        let out = app
            .trim_output(&c.goal, "EVAL-1", &c.tool, &c.what, &shown)
            .await;
        let ms = t.elapsed().as_millis();
        let asked = app
            .judge_activity(1)
            .first()
            .is_some_and(|a| a.use_ == "trim_output" && a.ticket.as_deref() == Some("EVAL-1"));
        let jev = if asked { usage_of(&app) } else { 0 };
        let text = out.as_deref().unwrap_or(&shown);
        let kept = needed.iter().filter(|l| text.contains(*l)).count();
        let lines_in = shown.lines().count();
        let left_out = lines_in.saturating_sub(text.lines().count());
        let saved = shown.len().saturating_sub(text.len()) / 4;
        saved_total += saved;
        jev_total += jev;
        lost_total += needed.len() - kept;
        println!(
            "| {} (`{}`) | {lines_in} | {left_out} | {saved} | {kept}/{} | {jev} | {ms} |",
            c.file,
            c.what,
            needed.len()
        );
    }
    println!(
        "\nTotal: ≈ {saved_total} tokens left out of the workers' context per occurrence (re-read on every later turn), \
         {lost_total} needed lines lost, {jev_total} Jev input tokens (≈ ${:.5}).\n",
        jev_total as f64 * 0.042 / 1e6
    );

    // ---------------------------------------------------------------- done check
    println!("## Done check ({model}, review_assist prefetched in the same request)\n");
    println!("| Ticket | run | expected | decision | judge read | Jev input tokens |");
    println!("|---|---|---|---|---|---|");
    let (mut right, mut judged) = (0, 0);
    for (i, (title, acs, summary, evidence, run, expected)) in done_cases().into_iter().enumerate()
    {
        let t = ticket(i as u64 + 1, title, &acs);
        let r = report(&t, summary, evidence);
        let facts = run_facts(&run);
        let before = app.judge_activity(1).first().map(|a| a.at.clone());
        let sent_back = app.unverified_done(&t, &r, &facts).await.is_some();
        let last = app.judge_activity(1).into_iter().next();
        let read = match &last {
            Some(a) if Some(&a.at) != before.as_ref() && a.use_ == "done_check" => {
                a.outcome.clone()
            }
            _ => "not asked (not claimed done, nothing changed, or verified)".into(),
        };
        let jev = if read.starts_with("not asked") {
            0
        } else {
            usage_of(&app)
        };
        let run_text = run
            .iter()
            .map(|e| match e.data["tool"].as_str() {
                Some("shell") => format!(
                    "`{}` {}",
                    e.data["args"]["command"].as_str().unwrap_or(""),
                    e.data["result"].as_str().unwrap_or("").trim()
                ),
                _ => format!("edit {}", e.data["args"]["path"].as_str().unwrap_or("")),
            })
            .collect::<Vec<_>>()
            .join("; ");
        let exp = match expected {
            Some(true) => "send back",
            Some(false) => "accept",
            None => "either",
        };
        if let Some(e) = expected {
            judged += 1;
            right += usize::from(e == sent_back);
        }
        println!(
            "| {title} | {run_text} | {exp} | {} | {read} | {jev} |",
            if sent_back {
                "**sent back**"
            } else {
                "accepted"
            }
        );
    }
    println!("\nDecisions as expected: {right}/{judged}.");
    Ok(())
}
