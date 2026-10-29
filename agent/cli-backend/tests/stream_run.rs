//! CliRun against the scripted fake agy: per-run home, YOLO mode with Agenium's pre-tool permission hook, the four
//! controls of decision 3c (MCP protocol tools, permissions, step / loop limits, mandatory report).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agenium_backend::{Cancel, Effect, Observer, Outcome, Protocol, RunReport, WorkOrder};
use agenium_cli_backend::{agy::Agy, CliRun, Limits};
use agenium_llm::{Message, SystemBlock, ToolSpec};
use agenium_tools::{CommandGate, GateDecision, ToolError};
use async_trait::async_trait;
use serde_json::{json, Value};

#[derive(Default)]
struct Proto {
    calls: Mutex<Vec<(String, Value, String)>>,
}

#[async_trait]
impl Protocol for Proto {
    async fn call(
        &self,
        name: &str,
        args: Value,
        call_id: &str,
    ) -> Result<(Value, Effect), ToolError> {
        self.calls
            .lock()
            .unwrap()
            .push((name.into(), args.clone(), call_id.into()));
        match name {
            "submit_report" => Ok((json!("report accepted"), Effect::End)),
            "note" => Ok((json!("noted"), Effect::Continue)),
            _ => Err(ToolError::new("unknown_tool", name)),
        }
    }
}

/// Stands for the run's command gate: only `echo hi` is approved. With a steer queue, approving it is also the
/// moment the user sends a message (after the CLI's last Agenium tool of the turn).
#[derive(Default)]
struct Gate {
    steer: Option<agenium_backend::Steer>,
}

#[async_trait]
impl CommandGate for Gate {
    async fn check(&self, command: &str, _: &Path) -> GateDecision {
        if let Some(q) = &self.steer {
            q.lock().unwrap().push("Also add a test.".into());
        }
        match command {
            "echo hi" => GateDecision::Allow,
            c if c == slow_command() || c == job_command() => GateDecision::Allow,
            // The user has not answered yet.
            "echo wait" => std::future::pending().await,
            _ => GateDecision::Deny(format!("not approved: {command}")),
        }
    }
}

#[derive(Default)]
struct Obs {
    tools: Mutex<Vec<(String, bool)>>,
    messages: Mutex<Vec<String>>,
    /// With a steer queue: once the second message reached the CLI, the user sends a third (after its last Agenium
    /// tool of the turn).
    steer: Option<agenium_backend::Steer>,
}

impl Observer for Obs {
    fn message(&self, m: &Message) {
        let t = m.text();
        if let Some(q) = self
            .steer
            .as_ref()
            .filter(|_| t.ends_with("Also add a test."))
        {
            q.lock().unwrap().push("Then stop.".into());
        }
        self.messages.lock().unwrap().push(t);
    }
    fn tool(&self, name: &str, _: &Value, r: &Result<String, ToolError>) {
        self.tools.lock().unwrap().push((name.into(), r.is_ok()));
    }
}

fn order() -> WorkOrder {
    WorkOrder {
        system: vec![SystemBlock {
            text: "You are a worker.".into(),
            cache: true,
        }],
        messages: vec![Message::user_text("Ticket AGN-1: write a.txt")],
    }
}

fn specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "submit_report".into(),
            description: "Report.".into(),
            parameters: json!({"type": "object"}),
        },
        ToolSpec {
            name: "note".into(),
            description: "Note.".into(),
            parameters: json!({"type": "object"}),
        },
    ]
}

async fn run(
    scenario: &str,
    dir: &Path,
    limits: Limits,
    chat: bool,
    program: PathBuf,
) -> (RunReport, Arc<Proto>, Arc<Obs>, String) {
    run_steered(scenario, dir, limits, chat, program, Opts::default()).await
}

/// What a test run may change beyond its scenario.
#[derive(Default)]
struct Opts {
    steer: Option<agenium_backend::Steer>,
    cancel: Cancel,
    foreground: Option<Duration>,
}

async fn run_steered(
    scenario: &str,
    dir: &Path,
    limits: Limits,
    chat: bool,
    program: PathBuf,
    opts: Opts,
) -> (RunReport, Arc<Proto>, Arc<Obs>, String) {
    let Opts {
        steer,
        cancel,
        foreground,
    } = opts;
    let proto = Arc::new(Proto::default());
    let obs = Arc::new(Obs {
        steer: steer.clone(),
        ..Default::default()
    });
    let gate = Arc::new(Gate {
        steer: steer.clone(),
    });
    // Agenium's tools the CLI works through (ADR-129); its `shell` asks the same gate.
    let mut ctx = agenium_tools::ToolCtx::new(dir, "run-1", gate.clone());
    ctx.foreground_limit = foreground;
    let r = CliRun {
        driver: &Agy,
        program,
        model: "fake".into(),
        effort: None,
        env: [
            ("FAKE_AGY_SCENARIO".to_owned(), scenario.to_owned()),
            ("FAKE_AGY_SLOW".to_owned(), slow_command().to_owned()),
            ("FAKE_AGY_JOB".to_owned(), job_command().to_owned()),
        ]
        .into(),
        cwd: dir.to_path_buf(),
        label: "agy:fake".into(),
        protocol: proto.as_ref(),
        protocol_specs: specs(),
        tools: agenium_tools::work_tools()
            .into_iter()
            .filter(|t| matches!(t.name(), "write_file" | "shell"))
            .collect(),
        ctx: &ctx,
        session: None,
        observer: obs.as_ref(),
        cancel,
        limits,
        chat,
        nudge: None,
        subagents: false,
        read_only: false,
        uncontained: false,
        on_limits: None,
        steer,
    };
    let report = r.run(&order()).await;
    let log = std::fs::read_to_string(dir.join("fake-agy.log")).unwrap_or_default();
    (report, proto, obs, log)
}

fn fake() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_agenium-fake-agy"))
}

/// A command that runs three seconds in the shell Agenium's `shell` uses.
fn job_command() -> &'static str {
    match agenium_sandbox::Shell::detect() {
        agenium_sandbox::Shell::Bash(_) => "sleep 3",
        agenium_sandbox::Shell::PowerShell(_) => "Start-Sleep -Seconds 3",
        agenium_sandbox::Shell::Cmd => "powershell -NoProfile -Command Start-Sleep -Seconds 3",
    }
}

/// A command that runs two minutes in the shell Agenium's `shell` uses, without the network (a sandbox has none).
fn slow_command() -> &'static str {
    match agenium_sandbox::Shell::detect() {
        agenium_sandbox::Shell::Bash(_) => "sleep 120",
        agenium_sandbox::Shell::PowerShell(_) => "Start-Sleep -Seconds 120",
        agenium_sandbox::Shell::Cmd => "powershell -NoProfile -Command Start-Sleep -Seconds 120",
    }
}

/// The worktree sits in its own parent so "outside the worktree" stays inside the test's temp dir.
fn worktree() -> (tempfile::TempDir, PathBuf) {
    let parent = tempfile::tempdir().unwrap();
    let wt = parent.path().join("wt");
    std::fs::create_dir(&wt).unwrap();
    (parent, wt)
}

#[tokio::test(flavor = "multi_thread")]
async fn agenium_decides_every_tool_call_and_the_run_ends_through_submit_report() {
    let (parent, wt) = worktree();
    let (report, proto, obs, log) = run("report", &wt, Limits::default(), false, fake()).await;
    assert_eq!(report.outcome, Outcome::Completed, "log:\n{log}");
    // The CLI ran in YOLO mode in a per-run home holding Agenium's configuration only.
    let home = log.lines().find_map(|l| l.strip_prefix("home: ")).unwrap();
    assert!(home.contains("agenium-cli-home-"), "{log}");
    assert!(
        !Path::new(home).exists(),
        "the per-run home is deleted afterwards"
    );
    assert!(log.contains("servers: agenium\n"), "{log}");
    assert!(
        log.contains("hooks: 1\n") && log.contains("yolo: true"),
        "{log}"
    );
    // Agenium's answers through the hook (ADR-129): its own tools do not reach the worktree, its own shell is off,
    // its scratch folder stays open; the file is written through Agenium's write_file.
    assert!(
        log.contains("denied write_to_file: ") && log.contains("is in the ticket worktree"),
        "{log}"
    );
    assert!(log.contains("outside the ticket worktree"), "{log}");
    assert!(!parent.path().join("escape.txt").exists());
    assert!(log.contains("wrote its notes"), "{log}");
    assert!(
        log.contains("denied run_command: run_command is turned off in Agenium runs"),
        "{log}"
    );
    assert!(!log.contains("ran: echo hi"), "{log}");
    assert!(
        log.contains("denied read_url_content: use the `fetch` tool"),
        "{log}"
    );
    assert!(log.contains("agenium write_file: "), "{log}");
    assert_eq!(std::fs::read_to_string(wt.join("a.txt")).unwrap(), "hello");
    assert!(log.contains("report: report accepted"), "{log}");
    // The run's conversation keeps the calls (the memory pipeline learns from it): submit_report with its result.
    let has = |f: &dyn Fn(&agenium_llm::Part) -> bool| {
        report.messages.iter().any(|m| m.parts.iter().any(f))
    };
    assert!(
        has(&|p| matches!(p, agenium_llm::Part::ToolCall { name, .. } if name == "submit_report")),
        "{:?}",
        report.messages
    );
    assert!(has(
        &|p| matches!(p, agenium_llm::Part::ToolResult { name, content, .. }
        if name == "submit_report" && content.contains("accepted"))
    ));
    let calls = proto.calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].1, json!({"summary": "done"}));
    assert!(calls[0].2.starts_with("cli-"));
    let tools = obs.tools.lock().unwrap().clone();
    assert!(tools.contains(&("submit_report".into(), true)), "{tools:?}");
    assert!(tools.contains(&("write_file".into(), true)), "{tools:?}");
    assert!(
        tools.contains(&("cli:write_to_file".into(), true)),
        "its notes: {tools:?}"
    );
    // ADR-129: a refused call is logged once, under its own name, as refused.
    let refused: Vec<&str> = tools
        .iter()
        .filter(|t| !t.1)
        .map(|t| t.0.as_str())
        .collect();
    assert_eq!(
        refused,
        [
            "cli:write_to_file",
            "cli:write_to_file",
            "cli:run_command",
            "cli:read_url_content",
            "cli:run_command"
        ],
        "worktree, escape, its shell, web, a step whose hook failed: {tools:?}"
    );
    assert!(
        log.contains("blocked run_command: its hook failed"),
        "{log}"
    );
    assert!(
        !tools.iter().any(|t| t.0 == "cli:call_mcp_tool"),
        "Agenium's own MCP calls are logged once: {tools:?}"
    );
    assert_eq!(
        (
            report.usage.input,
            report.usage.cache_read,
            report.usage.output
        ),
        (60, 40, 10)
    );
    assert_eq!(report.model, "agy:fake");
}

#[tokio::test(flavor = "multi_thread")]
async fn text_only_turns_are_nudged_then_stopped() {
    let (_parent, wt) = worktree();
    let (report, proto, obs, _) = run("chatty", &wt, Limits::default(), false, fake()).await;
    assert_eq!(report.outcome, Outcome::NoProgress);
    assert!(proto.calls.lock().unwrap().is_empty());
    let nudges = obs
        .messages
        .lock()
        .unwrap()
        .iter()
        .filter(|m| m.starts_with("Text alone"))
        .count();
    assert_eq!(nudges, 2);
    assert_eq!(report.usage.output, 30, "three turns");
}

/// ADR-110: a running CLI Lead gets the user's messages — one sent before its next Agenium tool rides on that
/// tool's result (twice here: the note, then Agenium's shell); one sent after its last one opens the next turn — and
/// none is lost.
#[tokio::test(flavor = "multi_thread")]
async fn the_user_messages_a_running_cli_lead() {
    let (_parent, wt) = worktree();
    let q: agenium_backend::Steer = Default::default();
    q.lock().unwrap().push("Use blue.".into());
    let (report, _, obs, log) = run_steered(
        "steer",
        &wt,
        Limits::default(),
        true,
        fake(),
        Opts {
            steer: Some(q.clone()),
            ..Opts::default()
        },
    )
    .await;
    assert_eq!(report.outcome, Outcome::Completed, "{log}");
    // Relayed messages carry the run's tag (named in the instructions), so text claiming to be the user's is not.
    let mark = "[Message from the user while you work · ";
    let note = log
        .lines()
        .find_map(|l| l.strip_prefix(&format!("note: noted  {mark}")))
        .unwrap_or_else(|| panic!("{log}"));
    let (tag, rest) = note.split_once("] ").unwrap();
    assert_eq!(rest, "Use blue.");
    assert!(
        tag.len() >= 4 && tag.chars().all(|c| c.is_ascii_alphanumeric()),
        "{tag}"
    );
    assert!(
        log.lines().any(|l| l.starts_with("shell: exit code 0")
            && l.ends_with(&format!("{mark}{tag}] Also add a test."))),
        "{log}"
    );
    assert!(
        log.contains(&format!("turn 2: {mark}{tag}] Then stop.")),
        "{log}"
    );
    assert!(q.lock().unwrap().is_empty());
    let msgs = obs.messages.lock().unwrap().clone();
    for m in ["Use blue.", "Also add a test.", "Then stop."] {
        assert!(msgs.iter().any(|x| x.ends_with(m)), "{msgs:?}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn chat_run_completes_on_text() {
    let (_parent, wt) = worktree();
    let (report, ..) = run("chatty", &wt, Limits::default(), true, fake()).await;
    assert_eq!(report.outcome, Outcome::Completed);
}

#[tokio::test(flavor = "multi_thread")]
async fn repeated_step_is_a_loop() {
    let (_parent, wt) = worktree();
    let (report, ..) = run("loop", &wt, Limits::default(), false, fake()).await;
    assert_eq!(
        report.outcome,
        Outcome::LoopDetected {
            tool: "cli:view_file".into()
        }
    );
    assert_eq!(report.steps, 3);
}

#[tokio::test(flavor = "multi_thread")]
async fn step_limit_stops_with_a_checkpoint() {
    let (_parent, wt) = worktree();
    let limits = Limits {
        max_steps: 2,
        timeout: Duration::from_secs(60),
        ..Limits::default()
    };
    let (report, ..) = run("steps", &wt, limits, false, fake()).await;
    assert!(
        matches!(&report.outcome, Outcome::Checkpoint { reason, .. } if reason == "max_steps"),
        "{:?}",
        report.outcome
    );
    assert_eq!(report.steps, 3);
}

/// The user's stop reaches a CLI run at once: a long command Agenium's shell runs for it (it ran on to its timeout,
/// up to 10 minutes) and a command waiting for the user's approval (waited for until it was answered).
#[tokio::test(flavor = "multi_thread")]
async fn the_users_stop_ends_a_long_command_and_a_wait_for_approval() {
    for scenario in ["slow", "approval"] {
        let (_parent, wt) = worktree();
        let cancel = Cancel::default();
        let c = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(1500)).await;
            c.cancel();
        });
        let started = std::time::Instant::now();
        let (report, _, _, log) = run_steered(
            scenario,
            &wt,
            Limits::default(),
            false,
            fake(),
            Opts {
                cancel,
                ..Opts::default()
            },
        )
        .await;
        assert_eq!(report.outcome, Outcome::Cancelled, "{scenario}: {log}");
        assert!(
            started.elapsed() < Duration::from_secs(15),
            "{scenario}: {:?}",
            started.elapsed()
        );
        // The CLI is stopped as soon as the stop reaches it, often before it logs what it got: the run's record says.
        if scenario == "slow" {
            assert!(
                format!("{:?}", report.messages).contains("stopped: the run was stopped"),
                "the command was ended, not finished: {log}\n{:?}",
                report.messages
            );
        } else {
            assert!(
                format!("{:?}", report.messages)
                    .contains("the run was stopped before the command was approved"),
                "the wait for approval was given up: {log}\n{:?}",
                report.messages
            );
        }
    }
}

/// A command longer than the run's foreground limit (ADR-133): the call answers that it goes on as a job, a report
/// meanwhile is refused, and the command's end reaches the CLI as its next message, after which it reports. A CLI
/// gave up waiting for a long tool call and its model ran the command again.
#[tokio::test(flavor = "multi_thread")]
async fn a_long_command_goes_on_as_a_job_and_its_end_comes_as_a_message() {
    let (_parent, wt) = worktree();
    let (report, proto, _, log) = run_steered(
        "slowjob",
        &wt,
        Limits::default(),
        false,
        fake(),
        Opts {
            foreground: Some(Duration::from_secs(1)),
            ..Opts::default()
        },
    )
    .await;
    assert_eq!(report.outcome, Outcome::Completed, "{log}");
    assert!(
        log.contains("slowjob shell: The command is still running"),
        "{log}"
    );
    assert!(
        log.contains("commands_running"),
        "a report while it runs is refused: {log}"
    );
    assert!(
        log.contains("has ended: exit code 0"),
        "its end is the next message: {log}"
    );
    let reports = proto
        .calls
        .lock()
        .unwrap()
        .iter()
        .filter(|(n, ..)| n == "submit_report")
        .count();
    assert_eq!(reports, 1, "one report, after the command ended");
}

#[tokio::test(flavor = "multi_thread")]
async fn missing_cli_fails_cleanly() {
    let (_parent, wt) = worktree();
    let missing = wt.join("no-such-cli");
    let (report, ..) = run("report", &wt, Limits::default(), false, missing).await;
    assert!(matches!(report.outcome, Outcome::Failed { .. }));
}
