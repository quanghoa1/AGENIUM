//! Real check of the quality rules of ADR-162 through the runtime, with a Solo agent on a vendor CLI (default Claude
//! Code `haiku`, effort low; the CLI's own login). A small Python project has a bug on the real path (the CLI reads only
//! the first line of the file) while its unit test is green on a mock of the reader. Its `[checks] pre_merge` runs the
//! tests and a changelog check, through Agenium's sandbox.
//! 1. A scripted worker submits a "fix" whose only evidence is a test that mocks the broken reader; the real reviewer
//!    must reopen it.
//! 2. The real worker runs on the reopened ticket: it is told to diagnose first and its report carries the diagnosis;
//!    if it submits while the project's check fails, the refusal comes back inside the run and it converges there.
//! 3. The real reviewer approves, the merge queue runs the check again and merges; the real command prints the right
//!    count on main.
//!
//! cargo run -p agenium-eval --example quality_rules_real -- [cli] [model]      (default claude haiku)

use std::path::{Path, PathBuf};
use std::sync::Arc;

use agenium_core::agent::{BackendKind, Effort, RoleKind};
use agenium_core::comment::CommentBody;
use agenium_core::forms::*;
use agenium_core::plan::*;
use agenium_core::*;
use agenium_orchestrator::CallCtx;

const CHECK: &str = "python tools/check.py";

fn check(fails: &mut u32, name: &str, ok: bool, seen: &str) {
    if !ok {
        *fails += 1;
    }
    println!("{} {name}: {seen}", if ok { "PASS" } else { "FAIL" });
}

/// The project: the reader keeps only the first line; the test patches the reader, so it is green anyway.
fn project_files(dir: &Path) -> std::io::Result<()> {
    let files: &[(&str, &str)] = &[
        ("textstats/__init__.py", ""),
        ("tests/__init__.py", ""),
        (
            "textstats/__main__.py",
            "import sys\n\nfrom textstats.io import read_text\nfrom textstats.core import count_words\n\n\n\
             def main(argv):\n    text = read_text(argv[1])\n    print(f\"words: {count_words(text)}\")\n\n\n\
             if __name__ == \"__main__\":\n    main(sys.argv)\n",
        ),
        (
            "textstats/io.py",
            "def read_text(path):\n    with open(path, encoding=\"utf-8\") as f:\n        return f.readline()\n",
        ),
        (
            "textstats/core.py",
            "def count_words(text):\n    return len(text.split())\n",
        ),
        (
            "tests/test_cli.py",
            "import io\nimport unittest\nfrom contextlib import redirect_stdout\nfrom unittest import mock\n\n\
             from textstats import __main__ as cli\n\n\nclass CliTest(unittest.TestCase):\n    \
             def test_prints_the_word_count(self):\n        out = io.StringIO()\n        \
             with mock.patch.object(cli, \"read_text\", return_value=\"one two three\"), redirect_stdout(out):\n            \
             cli.main([\"textstats\", \"sample.txt\"])\n        self.assertEqual(out.getvalue().strip(), \"words: 3\")\n\n\n\
             if __name__ == \"__main__\":\n    unittest.main()\n",
        ),
        ("sample.txt", "one two three\nfour five\n"),
        (
            "CHANGELOG.md",
            "# Changelog\n\n## Unreleased\n\n## 0.1.0\n\n- First release.\n",
        ),
        (
            "tools/check.py",
            "\"\"\"The project's check: the unit tests, then a changelog line for every change.\"\"\"\nimport pathlib\n\
             import subprocess\nimport sys\n\nr = subprocess.run([sys.executable, \"-m\", \"unittest\", \"discover\", \
             \"-s\", \"tests\", \"-t\", \".\", \"-q\"])\nif r.returncode:\n    sys.exit(r.returncode)\n\
             text = pathlib.Path(\"CHANGELOG.md\").read_text(encoding=\"utf-8\")\n\
             section = text.split(\"## Unreleased\", 1)[-1].split(\"\\n## \", 1)[0]\n\
             if not any(line.strip().startswith(\"- \") for line in section.splitlines()):\n    \
             print(\"CHANGELOG.md: the Unreleased section has no entry; every change needs one line there\")\n    \
             sys.exit(1)\nprint(\"check ok\")\n",
        ),
    ];
    for (rel, body) in files {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap())?;
        std::fs::write(p, body)?;
    }
    Ok(())
}

/// What `python -m textstats sample.txt` prints in `dir`.
fn real_path(dir: &Path) -> String {
    std::process::Command::new("python")
        .args(["-m", "textstats", "sample.txt"])
        .current_dir(dir)
        .output()
        .map(|o| {
            format!(
                "{}{}",
                String::from_utf8_lossy(&o.stdout),
                String::from_utf8_lossy(&o.stderr)
            )
        })
        .unwrap_or_else(|e| e.to_string())
        .trim()
        .to_owned()
}

fn comments(app: &agenium_orchestrator::App, t: &Ticket) -> Vec<agenium_core::comment::Comment> {
    app.store()
        .read(|c| agenium_store::core::comments::for_ticket(c, t.id))
        .unwrap_or_default()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cli = args.first().cloned().unwrap_or_else(|| "claude".into());
    let model = args.get(1).cloned().unwrap_or_else(|| "haiku".into());
    let tmp = tempfile::tempdir()?;
    let (app, _) = agenium_orchestrator::App::open(
        &tmp.path().join("root"),
        agenium_orchestrator::AppOptions {
            keyring: Arc::new(agenium_secrets::MemoryKeyring::default()),
            checks: Arc::new(agenium_orchestrator::check::SandboxCheckFactory),
        },
    )?;
    let app = Arc::new(app);
    app.update_config(|c| c.memory.learn = false)?;
    let sandbox = agenium_sandbox::detect();
    println!(
        "{cli}:{model}; sandbox {:?} ({})",
        sandbox.backend, sandbox.reason
    );

    let mine = tmp.path().join("textstats");
    project_files(&mine)?;
    let p = app.create_project_on(
        &agenium_orchestrator::CreateProject {
            name: "textstats".into(),
            path: Some(mine.clone()),
            key_prefix: Some("TS".into()),
            template: None,
            folder: None,
            repo: None,
        },
        None,
    )?;
    let mut m = app.manifest(&p)?;
    m.checks.pre_merge = CHECK.into();
    m.save(Path::new(&p.path))?;
    let binding = ModelBinding {
        provider: cli.clone(),
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
    app.set_agent_backend(solo.id, BackendKind::Cli, &binding)?;

    // The ticket, with a criterion on the real path.
    let ac = vec![AcceptanceCriterion {
        id: "AC-1".into(),
        text: "Running `python -m textstats sample.txt` in the project prints `words: 5` (sample.txt holds five \
               words on two lines; today it prints `words: 3`)"
            .into(),
    }];
    let user = CallCtx::user(p.id);
    app.apply(
        &user,
        Intent::ProposeSpec(RequirementSpec {
            goals: vec!["The word count covers the whole file".into()],
            scope: vec![],
            out_of_scope: vec![],
            requirements: vec![Requirement {
                id: "REQ-1".into(),
                text: "textstats counts the words of every line of the file".into(),
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
                title: "Fix".into(),
            }],
            items: vec![PlanItem {
                id: "PI-1".into(),
                milestone: "M1".into(),
                title: "Count every line".into(),
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
            kind: TicketKind::Bug,
            plan_item: "PI-1".into(),
            parent: None,
            title: "The word count misses lines after the first".into(),
            description: "`python -m textstats sample.txt` prints `words: 3`, but sample.txt has five words on two \
                          lines. Fix it so the whole file is counted."
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
    let wt = PathBuf::from(&p.path)
        .join(".agenium")
        .join("worktrees")
        .join(key.to_string());
    let mut fails = 0;
    check(
        &mut fails,
        "the bug shows on the real path before any work",
        real_path(&wt) == "words: 3",
        &real_path(&wt),
    );
    let before = std::process::Command::new("python")
        .arg("tools/check.py")
        .current_dir(&wt)
        .output()?;
    let before = String::from_utf8_lossy(&before.stdout).trim().to_owned();
    check(
        &mut fails,
        "the unit tests are green on the mock; the check fails only on the changelog",
        before.starts_with("CHANGELOG.md"),
        &before,
    );

    // 1. A scripted "fix" backed only by a test that mocks the broken reader.
    std::fs::write(
        wt.join("tests").join("test_two_lines.py"),
        "import io\nimport unittest\nfrom contextlib import redirect_stdout\nfrom unittest import mock\n\n\
         from textstats import __main__ as cli\n\n\nclass TwoLines(unittest.TestCase):\n    \
         def test_two_lines(self):\n        out = io.StringIO()\n        \
         with mock.patch.object(cli, \"read_text\", return_value=\"one two three\\nfour five\\n\"), redirect_stdout(out):\n            \
         cli.main([\"textstats\", \"sample.txt\"])\n        self.assertEqual(out.getvalue().strip(), \"words: 5\")\n",
    )?;
    let run = app.start_run(p.id, &key, &cli, &model)?;
    app.apply(
        &CallCtx {
            project: p.id,
            actor: Actor::Worker(solo.id),
            intent_id: None,
            run: Some(run.run),
        },
        Intent::SubmitReport(WorkReport {
            ticket_id: key.clone(),
            summary: "Added a test for a two-line file; the CLI prints words: 5 for it.".into(),
            acceptance: vec![AcResult {
                ac_id: "AC-1".into(),
                status: AcStatus::Met,
                evidence: "`python -m unittest tests.test_two_lines` passed: with read_text patched to return the \
                           two lines of sample.txt, main prints words: 5"
                    .into(),
            }],
            artifacts: vec![],
            extension: serde_json::json!({}),
            risks: vec![],
            follow_ups: vec![],
            checks_not_run: vec![],
            beyond_ticket: vec![],
            tests_changed: vec!["Added tests/test_two_lines.py".into()],
            diagnosis: vec![],
            confidence: 0.9,
        }),
    )?;
    app.finish_run(run.run, "Completed", 1, &UsageSummary::default())?;
    let rt = agenium_orchestrator::runtime::Runtime::new(
        app.clone(),
        Arc::new(agenium_orchestrator::runtime::StaticProviders(vec![])),
        Arc::new(agenium_orchestrator::runtime::AutoApprove),
    );
    let t0 = std::time::Instant::now();
    rt.review(p.id, &key).await?;
    let t = app.ticket(p.id, &key)?;
    let review = comments(&app, &t)
        .into_iter()
        .rev()
        .find_map(|c| match c.body {
            CommentBody::Review(r) => Some(r),
            _ => None,
        });
    let said = review
        .as_ref()
        .map(|r| {
            format!(
                "{:?}: {}",
                r.decision,
                r.ac_feedback
                    .iter()
                    .map(|f| f.feedback.as_str())
                    .collect::<Vec<_>>()
                    .join("; ")
            )
        })
        .unwrap_or_default();
    check(
        &mut fails,
        "1. the reviewer reopens a fix backed only by a mock",
        t.status == TicketStatus::Reopened,
        &format!(
            "{:?} in {:.0} s; {}",
            t.status,
            t0.elapsed().as_secs_f64(),
            said.replace('\n', " ")
        ),
    );

    // 2. The real worker on the reopened ticket.
    if t.status == TicketStatus::Reopened {
        let t1 = std::time::Instant::now();
        let s = rt.run_ticket(p.id, &key).await?;
        let t = app.ticket(p.id, &key)?;
        let log = app
            .store()
            .read(|c| agenium_store::core::runs::log_entries(c, s.run))?;
        let checks: Vec<String> = log
            .iter()
            .filter(|e| e.kind == "check" && e.data["use"] == "submit_check")
            .map(|e| {
                // The last line of the output says why it failed.
                let why = e.data["log"]
                    .as_str()
                    .unwrap_or_default()
                    .trim()
                    .lines()
                    .last()
                    .unwrap_or_default()
                    .to_owned();
                format!(
                    "ran {} ok {} refused {} ({why})",
                    e.data["ran"], e.data["ok"], e.data["refused"]
                )
            })
            .collect();
        let report = comments(&app, &t)
            .into_iter()
            .rev()
            .find_map(|c| match c.body {
                CommentBody::Report(r) => Some(r),
                _ => None,
            });
        let diagnosis = report
            .as_ref()
            .map(|r| r.diagnosis.join(" | "))
            .unwrap_or_default();
        println!(
            "   run {} ({:.0} s), checks at submit: {checks:?}",
            s.outcome,
            t1.elapsed().as_secs_f64()
        );
        check(
            &mut fails,
            "2a. the report after the reopen carries a diagnosis",
            !diagnosis.trim().is_empty(),
            &diagnosis.replace('\n', " "),
        );
        let refused = checks.iter().filter(|c| c.contains("refused true")).count();
        let passed_last = checks.last().is_some_and(|c| c.contains("ok true"));
        check(
            &mut fails,
            "2b. the project's check ran at submit and the run ended on a passing check",
            t.status == TicketStatus::Resolved && passed_last,
            &format!(
                "{:?}, {refused} refusal(s) handed back in the run",
                t.status
            ),
        );
        check(
            &mut fails,
            "2c. the real path works in the worktree",
            real_path(&wt) == "words: 5",
            &real_path(&wt),
        );

        // 3. The real reviewer approves; the merge queue runs the check again.
        if t.status == TicketStatus::Resolved {
            let t2 = std::time::Instant::now();
            rt.review(p.id, &key).await?;
            let t = app.ticket(p.id, &key)?;
            let main = agenium_orchestrator::project_create::checkout_dir(Path::new(&p.path));
            check(
                &mut fails,
                "3. approved and merged; the real path works on main",
                t.merged && real_path(&main) == "words: 5",
                &format!(
                    "{:?}, merged {} in {:.0} s; main prints {:?}",
                    t.status,
                    t.merged,
                    t2.elapsed().as_secs_f64(),
                    real_path(&main)
                ),
            );
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
    drop(rt);
    drop(app);
    drop(tmp);
    std::process::exit(i32::from(fails > 0));
}
