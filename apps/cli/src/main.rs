//! `agenium`: headless front-end over the orchestrator. Every state change goes through the same App/Runtime API
//! the desktop app uses.

mod agent;
mod args;
mod mcp;
mod provider;
mod role;
mod skill;
mod team;
mod ticket;

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use agenium_config::{ApprovalPolicy, ProviderConfig, ProviderKind};
use agenium_core::forms::PermissionDecision;
use agenium_core::*;
use agenium_orchestrator::app::{App, AppOptions, CheckFactory, CreateAgent, CreateProject};
use agenium_orchestrator::maintenance::BackupRequest;
use agenium_orchestrator::runtime::{Approval, Approver, ConfiguredProviders, Runtime};
use agenium_store::core::{comments, projects::Project, runs};
use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use clap::Parser;

use args::*;
use mcp::mcp_cmd;
use skill::skill_cmd;

/// Interactive approval on the terminal (M1-7: no command runs without the user's approval).
struct TerminalApprover;

/// One question on the terminal at a time: an agent's command and a pre-merge check asked together would otherwise
/// both read the next line, and a "y" typed for one could approve the other.
static PROMPT: Mutex<()> = Mutex::new(());

fn ask(prompt: &str) -> String {
    let _one = PROMPT.lock().unwrap_or_else(|e| e.into_inner());
    let mut out = std::io::stderr();
    let _ = write!(out, "{prompt}");
    let _ = out.flush();
    let mut line = String::new();
    let _ = std::io::stdin().lock().read_line(&mut line);
    line.trim().to_lowercase()
}

fn interactive() -> bool {
    use std::io::IsTerminal;
    std::io::stdin().is_terminal()
}

#[async_trait]
impl Approver for TerminalApprover {
    async fn approve_command(
        &self,
        _: ProjectId,
        ticket: Option<&TicketKey>,
        agent: &str,
        command: &str,
        cwd: &Path,
    ) -> Approval {
        if !interactive() {
            return Approval::Deny("no terminal to ask the user".into());
        }
        let prompt = format!(
            "\n[approval] {agent} on {} wants to run:\n    {command}\n  in {}\n  [y]es / [a]lways for this project / [N]o: ",
            ticket.map(|t| t.to_string()).unwrap_or_else(|| "-".into()),
            cwd.display()
        );
        let answer = tokio::task::block_in_place(|| ask(&prompt));
        match answer.as_str() {
            "y" | "yes" => Approval::Allow,
            "a" | "always" => Approval::AllowAlways,
            _ => Approval::Deny("declined at the terminal".into()),
        }
    }
}

/// Pre-merge checks configured in `agenium.toml` run only after the user approves them at the terminal.
struct ApprovedChecks;

struct ApprovedCheck {
    command: String,
}

impl agenium_vcs::CheckRunner for ApprovedCheck {
    fn run(&self, worktree: &Path) -> agenium_vcs::CheckOutcome {
        if !interactive() {
            return agenium_vcs::CheckOutcome {
                ok: false,
                log: format!(
                    "pre-merge check `{}` needs approval and no terminal is attached",
                    self.command
                ),
            };
        }
        let a = ask(&format!(
            "\n[approval] pre-merge check:\n    {}\n  in {}\n  run it? [y/N]: ",
            self.command,
            worktree.display()
        ));
        if a != "y" && a != "yes" {
            return agenium_vcs::CheckOutcome {
                ok: false,
                log: "the user declined the pre-merge check".into(),
            };
        }
        agenium_orchestrator::check::run_check(&self.command, worktree)
    }
}

impl CheckFactory for ApprovedChecks {
    fn for_project(
        &self,
        _: &Project,
        m: &agenium_orchestrator::manifest::ProjectManifest,
    ) -> Arc<dyn agenium_vcs::CheckRunner> {
        if m.checks.pre_merge.trim().is_empty() {
            Arc::new(agenium_vcs::NoCheck)
        } else {
            Arc::new(ApprovedCheck {
                command: m.checks.pre_merge.clone(),
            })
        }
    }
}

fn root(cli: &Cli) -> Result<PathBuf> {
    match &cli.root {
        Some(r) => Ok(r.clone()),
        None => Ok(agenium_config::resolve_root()?.0),
    }
}

fn open(root: &Path) -> Result<Arc<App>> {
    if agenium_config::setup::needs_setup(root) {
        bail!(
            "{} is not set up yet: run `agenium setup` first",
            root.display()
        );
    }
    // The embedding model is required (ADR-47): without it memory would recall by shared words only.
    let models = agenium_config::RootLayout::new(root).models();
    if !agenium_memory::local::installed(&agenium_memory::local::BGE_SMALL_EN, &models) {
        bail!(
            "the embedding model is not installed in {}: run `agenium models download`",
            models.display()
        );
    }
    let keyring: Arc<dyn agenium_secrets::KeyringBackend> =
        Arc::new(agenium_secrets::OsKeyring::default());
    let (app, report) = App::open(
        root,
        AppOptions {
            keyring,
            checks: Arc::new(ApprovedChecks),
        },
    )?;
    for a in &report.actions {
        eprintln!("[reconcile] {a}");
    }
    Ok(Arc::new(app))
}

/// Downloads the pinned model files into `<root>/models/<id>/` (resumable per file: a verified file is kept),
/// checking size and sha256 before a file is moved into place.
async fn models_cmd(cli: &Cli, action: &str) -> Result<()> {
    use agenium_memory::local::{self, BGE_SMALL_EN as SPEC};
    let r = root(cli)?;
    let models = agenium_config::RootLayout::new(&r).models();
    let dir = SPEC.dir(&models);
    match action {
        "status" => {
            if local::installed(&SPEC, &models) {
                println!("{} installed in {}", SPEC.id, dir.display());
            } else {
                println!(
                    "{} not installed ({} MB): run `agenium models download` (required: agents do not run without it)",
                    SPEC.id,
                    SPEC.total_bytes() >> 20
                );
            }
        }
        "verify" => {
            let bad = local::verify(&SPEC, &models);
            if bad.is_empty() {
                println!("{}: every file matches its sha256", SPEC.id);
            } else {
                bail!("{}: files missing or corrupt: {}", SPEC.id, bad.join(", "));
            }
        }
        _ => {
            let http = agenium_llm::http::Http::default_client()?;
            local::install(&SPEC, &models, &http, &|m| println!("{m}"))
                .await
                .map_err(|e| anyhow!(e))?;
            println!(
                "{} installed; agent memories are re-embedded on their next run",
                SPEC.id
            );
        }
    }
    Ok(())
}

/// Applies an intent as the user (override of the Lead, recorded as such) and prints the result.
fn user_intent(app: &App, project: ProjectId, intent: Intent) -> Result<()> {
    intent
        .validate()
        .map_err(|e| anyhow!("invalid: {}", serde_json::to_string(&e).unwrap_or_default()))?;
    let out = app.apply(&agenium_orchestrator::CallCtx::user(project), intent)?;
    println!("{}", out.result);
    Ok(())
}

fn team_named(app: &App, name: &str) -> Result<agenium_core::TeamId> {
    let found: Vec<_> = app
        .teams()?
        .into_iter()
        .filter(|(t, _)| t.name == name || t.id.to_string() == name)
        .collect();
    match found.as_slice() {
        [(t, _)] => Ok(t.id),
        [] => bail!("no team named {name:?} (see `agenium team list`)"),
        _ => bail!("several teams are named {name:?}: use the team id"),
    }
}

fn project(app: &App, name: &str) -> Result<Project> {
    app.projects()?
        .into_iter()
        .find(|p| p.name == name || p.id.to_string() == name)
        .ok_or_else(|| anyhow!("no project named {name:?} (see `agenium project list`)"))
}

fn runtime(app: Arc<App>) -> Runtime {
    let mut rt = Runtime::new(
        app.clone(),
        Arc::new(ConfiguredProviders::new(app)),
        Arc::new(TerminalApprover),
    );
    rt.live = Some(Arc::new(
        |_run: &RunId, e: &agenium_llm::ChatEvent| match e {
            agenium_llm::ChatEvent::TextDelta(t) => {
                print!("{t}");
                let _ = std::io::stdout().flush();
            }
            agenium_llm::ChatEvent::ToolCall(c) => {
                let args = c.args.to_string();
                eprintln!(
                    "\n  > {} {}",
                    c.name,
                    args.chars().take(160).collect::<String>()
                );
            }
            agenium_llm::ChatEvent::Restart => {
                eprintln!("\n  (connection problem, retrying the turn)")
            }
            _ => {}
        },
    ));
    rt
}

fn print_runs(rs: &[agenium_orchestrator::runtime::RunSummary]) {
    for r in rs {
        println!(
            "\n{} run {} on {} by {}: {} ({} steps, {} in / {} out tokens{})",
            r.kind,
            r.run,
            r.ticket
                .as_ref()
                .map(|k| k.to_string())
                .unwrap_or_else(|| "chat".into()),
            r.agent,
            r.outcome,
            r.steps,
            r.usage.tokens_in,
            r.usage.tokens_out,
            if r.usage.estimated_cost_usd > 0.0 {
                format!(", ~${:.4}", r.usage.estimated_cost_usd)
            } else {
                String::new()
            }
        );
    }
}

fn read_secret_stdin() -> Result<String> {
    if interactive() {
        eprint!("value (input is not hidden; prefer piping it): ");
    }
    let mut s = String::new();
    std::io::stdin().lock().read_line(&mut s)?;
    let s = s.trim_end_matches(['\r', '\n']).to_owned();
    if s.is_empty() {
        bail!("empty value");
    }
    Ok(s)
}

fn pairs(items: &[String]) -> Result<std::collections::BTreeMap<String, String>> {
    items
        .iter()
        .map(|i| {
            i.split_once('=')
                .map(|(k, v)| (k.trim().to_owned(), v.to_owned()))
                .ok_or_else(|| anyhow!("expected KEY=VALUE, got {i:?}"))
        })
        .collect()
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("AGENIUM_LOG")
                .unwrap_or_else(|_| "warn".into()),
        )
        .with_writer(std::io::stderr)
        .init();
    let cli = Cli::parse();
    match &cli.cmd {
        Cmd::Setup { ui_language } => {
            let default_root = dirs_home().join(".agenium");
            // --root is the root chosen in the wizard (remembered in the bootstrap file); AGENIUM_HOME is a
            // per-process override: set up there, but remember nothing machine-wide.
            let env_root = std::env::var_os(agenium_config::root::ENV_HOME)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from);
            let remember = cli.root.is_some() || env_root.is_none();
            let input = agenium_config::setup::SetupInput {
                root: cli.root.clone().or(env_root),
                ui_language: ui_language.clone(),
            };
            let bootstrap = agenium_config::Bootstrap::default_path().filter(|_| remember);
            let out =
                agenium_config::setup::complete_setup(&input, &default_root, bootstrap.as_deref())?;
            println!("Agenium root: {}", out.root.display());
            for c in out.report.warnings() {
                println!("  warning: {} = {} ({})", c.name, c.value, c.message);
            }
            println!(
                "Suggested parallel agents: {}",
                out.report.suggested_parallel
            );
            // The embedding model is required (ADR-47): memory recalls and learns by meaning with it.
            use agenium_memory::local::{self, BGE_SMALL_EN as SPEC};
            let models = agenium_config::RootLayout::new(&out.root).models();
            if !local::installed(&SPEC, &models) {
                println!(
                    "Installing the embedding model ({} MB, once)",
                    SPEC.total_bytes() >> 20
                );
                let http = agenium_llm::http::Http::default_client()?;
                local::install(&SPEC, &models, &http, &|m| println!("  {m}"))
                    .await
                    .map_err(|e| anyhow!("{e}; run `agenium models download` to try again"))?;
            }
        }
        Cmd::Support { out } => {
            let app = open(&root(&cli)?)?;
            let dir = app.support_bundle(out)?;
            println!("support bundle written to {}", dir.display());
        }
        Cmd::Report {
            project: pname,
            out,
        } => {
            let app = open(&root(&cli)?)?;
            let p = project(&app, pname)?;
            let md = app.project_report_md(p.id)?;
            match out {
                Some(f) => {
                    std::fs::write(f, &md)?;
                    println!("report written to {}", f.display());
                }
                None => println!("{md}"),
            }
        }
        Cmd::Doctor => {
            let r = root(&cli)?;
            let rep = agenium_config::sysreq::check(&r);
            println!("root: {}\nos: {}", r.display(), rep.os);
            for c in &rep.checks {
                println!("  [{:?}] {} = {} ({})", c.level, c.name, c.value, c.message);
            }
            let sb = agenium_sandbox::detect();
            println!("sandbox: {:?} - {}", sb.backend, sb.reason);
            if sb.backend.is_some() {
                println!("sandbox network: {}", agenium_sandbox::network_report());
            }
            println!("shell: {}", agenium_sandbox::Shell::detect().describe());
            println!("agent CLIs (CliBackend, M2.5; Agenium never logs in for you):");
            for c in agenium_cli_backend::detect_all() {
                let login = match c.login {
                    agenium_cli_backend::Login::LoggedIn => "logged in",
                    agenium_cli_backend::Login::NotLoggedIn => "not logged in",
                    agenium_cli_backend::Login::Unknown => "login unknown",
                };
                let driven = if agenium_cli_backend::driver(c.kind).is_some() {
                    ", usable"
                } else {
                    ", not driven yet"
                };
                match &c.version {
                    Some(v) => println!(
                        "  {:<9} {v:<12} {login}{driven}{}",
                        c.kind.program(),
                        c.note
                            .as_deref()
                            .map(|n| format!(" ({n})"))
                            .unwrap_or_default()
                    ),
                    None => println!(
                        "  {:<9} {}",
                        c.kind.program(),
                        c.note.as_deref().unwrap_or("-")
                    ),
                }
            }
            let models = agenium_config::RootLayout::new(&r).models();
            if agenium_memory::local::installed(&agenium_memory::local::BGE_SMALL_EN, &models) {
                println!("embedding model: bge-small-en-v1.5 (local)");
            } else {
                println!("embedding model: not installed, memory uses keyword-level hashing (agenium models download)");
            }
            match agenium_secrets::OsKeyring::status() {
                Ok(()) => println!("keyring: available"),
                Err(e) => println!("keyring: {e}"),
            }
            if !agenium_config::setup::needs_setup(&r) {
                let app = open(&r)?;
                let cfg = app.config();
                println!(
                    "approval policy: {:?}{}",
                    cfg.sandbox.approval,
                    match (cfg.sandbox.approval, sb.backend.is_some()) {
                        (ApprovalPolicy::Never, false) =>
                            " (YOLO is refused: no sandbox backend, fail-closed)",
                        (ApprovalPolicy::Sandboxed, true) =>
                            " (commands run inside the sandbox without asking)",
                        (ApprovalPolicy::Sandboxed, false) =>
                            " (no sandbox backend: commands need your approval)",
                        _ => "",
                    }
                );
                println!("secrets: {}", app.secrets().describe());
                println!(
                    "projects: {}, agents: {}",
                    app.projects()?.len(),
                    app.agents()?.len()
                );
            }
        }
        Cmd::Provider(ProviderCmd::Presets) => {
            for pr in agenium_config::presets::PRESETS {
                println!(
                    "{:<18} {:<30} {:<58} {}{}",
                    pr.id,
                    pr.name,
                    pr.base_url.unwrap_or("(adapter default)"),
                    if pr.key_env.is_empty() {
                        "no key"
                    } else {
                        pr.key_env
                    },
                    if pr.note.is_empty() {
                        String::new()
                    } else {
                        format!(" — {}", pr.note)
                    }
                );
            }
        }
        Cmd::Provider(pc) => provider::run(&cli, pc).await?,
        Cmd::Secret(sc) => {
            let app = open(&root(&cli)?)?;
            match sc {
                SecretCmd::Set { name } => {
                    let v = read_secret_stdin()?;
                    app.secrets().set(name, v.as_bytes())?;
                    println!("stored {name}");
                }
                SecretCmd::List => app.secrets().list()?.iter().for_each(|n| println!("{n}")),
                SecretCmd::Delete { name } => {
                    app.secrets().delete(name)?;
                    println!("deleted {name}");
                }
            }
        }
        Cmd::Project(pc) => {
            let app = open(&root(&cli)?)?;
            match pc {
                ProjectCmd::Create {
                    name,
                    path,
                    prefix,
                    template,
                    repo,
                } => {
                    let p = app.create_project(&CreateProject {
                        name: name.clone(),
                        path: path.clone(),
                        key_prefix: prefix.clone(),
                        template: template.clone(),
                        folder: None,
                        repo: repo.clone(),
                    })?;
                    println!("project {} at {}", p.name, p.path);
                }
                ProjectCmd::List => {
                    for p in app.projects()? {
                        let state = app.run_state(p.id)?;
                        println!(
                            "{}\t{}\t{state:?}\t{}\t{}",
                            p.name, p.status, p.key_prefix, p.path
                        );
                    }
                }
                ProjectCmd::Pause { project: name } => {
                    let p = project(&app, name)?;
                    app.set_project_status(
                        p.id,
                        agenium_store::core::projects::ProjectStatus::Paused,
                    )?;
                    println!(
                        "{} paused (runs in progress stop at their next step in the app)",
                        p.name
                    );
                }
                ProjectCmd::Resume { project: name } => {
                    let p = project(&app, name)?;
                    app.set_project_status(
                        p.id,
                        agenium_store::core::projects::ProjectStatus::Active,
                    )?;
                    println!("{} goes on", p.name);
                }
                ProjectCmd::ReviewBeforeMerge {
                    project: name,
                    value,
                } => {
                    let p = project(&app, name)?;
                    app.set_project_setting(
                        p.id,
                        "review_before_merge",
                        serde_json::json!(value == "on"),
                    )?;
                    println!("review_before_merge = {value}");
                }
            }
        }
        Cmd::Agent(ac) => agent::run(&cli, ac).await?,
        Cmd::Skill(sc) => skill_cmd(&cli, sc).await?,
        Cmd::Mcp(mc) => mcp_cmd(&cli, mc).await?,
        Cmd::Models { action } => models_cmd(&cli, action).await?,
        Cmd::Chat {
            project: pname,
            message,
        } => {
            let app = open(&root(&cli)?)?;
            let p = project(&app, pname)?;
            let rt = runtime(app);
            let r = rt.chat(p.id, message).await?;
            println!();
            if r.outcome != "Completed" && !r.text.is_empty() {
                eprintln!("{}", r.text);
            }
            print_runs(&[r]);
        }
        Cmd::Run {
            project: pname,
            rounds,
        } => {
            let app = open(&root(&cli)?)?;
            let p = project(&app, pname)?;
            let rt = runtime(app.clone());
            let cancel = rt.cancel.clone();
            tokio::spawn(async move {
                if tokio::signal::ctrl_c().await.is_ok() {
                    eprintln!("\nstopping after the current step (Ctrl+C)...");
                    cancel.cancel();
                }
            });
            let rs = rt.drive(p.id, *rounds).await?;
            print_runs(&rs);
            if rs.is_empty() {
                println!("nothing to run (no ready tickets or reviews)");
            }
            // What still waits for the user in this project (answered and other projects' items are not shown).
            for n in app.needs_you(p.id)? {
                let on = n.ticket.map(|t| format!("{t}: ")).unwrap_or_default();
                println!("needs you: {on}{}", n.text);
            }
        }
        Cmd::Ticket(tc) => ticket::run(&cli, tc).await?,
        Cmd::Board { project: pname } => {
            let app = open(&root(&cli)?)?;
            let p = project(&app, pname)?;
            for col in app.board(p.id)? {
                if col.cards.is_empty() {
                    continue;
                }
                println!("== {} ({})", col.status, col.cards.len());
                for c in &col.cards {
                    let mut flags = vec![];
                    if !c.blocked_by.is_empty() {
                        flags.push(format!(
                            "blocked by {}",
                            c.blocked_by
                                .iter()
                                .map(|k| k.to_string())
                                .collect::<Vec<_>>()
                                .join(",")
                        ));
                    }
                    if c.open_questions > 0 {
                        flags.push(format!("{} open question(s)", c.open_questions));
                    }
                    for (on, name) in [
                        (c.approved, "approved"),
                        (c.merged, "merged"),
                        (c.escalated, "ESCALATED"),
                    ] {
                        if on {
                            flags.push(name.into());
                        }
                    }
                    if c.reopen_count > 0 {
                        flags.push(format!("reopened x{}", c.reopen_count));
                    }
                    println!(
                        "  {}  P{}  {}  {}  [{}]{}",
                        c.key,
                        c.priority,
                        c.plan_item,
                        c.title,
                        c.assignee.as_deref().unwrap_or("-"),
                        if flags.is_empty() {
                            String::new()
                        } else {
                            format!("  ({})", flags.join("; "))
                        }
                    );
                }
            }
        }
        Cmd::Team(tc) => team::run(&cli, tc).await?,
        Cmd::Role(rc) => role::run(&cli, rc).await?,
        Cmd::Inbox { project: pname } => {
            let app = open(&root(&cli)?)?;
            let p = project(&app, pname)?;
            let items = app.lead_inbox(p.id)?;
            if items.is_empty() {
                println!("The Lead's inbox is empty (Solo agents send their questions to you).");
            }
            for (i, it) in items.iter().enumerate() {
                println!("{}. [{:?}] {}", i + 1, it.kind, it.text);
            }
        }
        Cmd::Plan { project: pname } => {
            let app = open(&root(&cli)?)?;
            let p = project(&app, pname)?;
            let v = app.plan_view(p.id)?;
            let names: std::collections::HashMap<String, String> = app
                .agents()?
                .into_iter()
                .map(|a| (format!("agent:{}", a.id), a.name))
                .collect();
            let who = |l: &str| names.get(l).cloned().unwrap_or_else(|| l.to_owned());
            match &v.spec {
                Some(s) => {
                    println!(
                        "Spec v{} ({})",
                        s.version,
                        match &s.approved_by {
                            Some(by) => format!("approved by {}, locked", who(by)),
                            None => format!(
                                "waits for your approval: agenium approve --project {} spec {}",
                                p.name, s.version
                            ),
                        }
                    );
                    for g in &s.body.goals {
                        println!("  goal: {g}");
                    }
                    for r in &s.body.requirements {
                        println!("  {}: {}", r.id, r.text);
                    }
                }
                None => println!("No spec yet: tell the Lead what you need (agenium chat)."),
            }
            if let Some(pl) = &v.plan {
                let state = match (&pl.approved_by, v.current_plan_version) {
                    (Some(by), _) => format!("approved by {}", who(by)),
                    (None, Some(cur)) => format!(
                        "waits for your approval (v{cur} stays in force): agenium approve --project {} plan {}",
                        p.name, pl.version
                    ),
                    (None, None) => format!("waits for your approval: agenium approve --project {} plan {}", p.name, pl.version),
                };
                println!(
                    "Plan v{} for spec v{} ({state})",
                    pl.version, pl.spec_version
                );
                if let Some(r) = &pl.reason {
                    println!("  revision reason: {r}");
                }
                for i in &v.items {
                    println!(
                        "  [{}] {} {}  {}  {}",
                        if i.done { "x" } else { " " },
                        i.milestone,
                        i.id,
                        i.title,
                        i.tickets
                            .iter()
                            .map(|(k, s)| format!("{k}:{s}"))
                            .collect::<Vec<_>>()
                            .join(" ")
                    );
                }
            }
        }
        Cmd::Answer {
            project: pname,
            ticket,
            question,
            answer,
            option,
        } => {
            let app = open(&root(&cli)?)?;
            let p = project(&app, pname)?;
            match app.route_user_answer(
                p.id,
                &ticket.parse().map_err(|e: IdParseError| anyhow!(e))?,
                question.parse().map_err(|e: IdParseError| anyhow!(e))?,
                answer,
                *option,
            )? {
                agenium_orchestrator::engine::UserAnswer::Answered(out) => {
                    println!("{}", out.result)
                }
                // In a team the user answers the Lead, who answers on the ticket (ADR-122).
                agenium_orchestrator::engine::UserAnswer::ToLead(message) => {
                    let rt = runtime(app);
                    let r = rt.chat(p.id, &message).await?;
                    println!();
                    print_runs(&[r]);
                }
            }
        }
        Cmd::Permission {
            project: pname,
            ticket,
            request,
            decision,
            always,
        } => {
            let app = open(&root(&cli)?)?;
            let p = project(&app, pname)?;
            let d = if decision == "approve" {
                PermissionDecision::Approve
            } else {
                PermissionDecision::Deny
            };
            let out = app.decide_permission(
                p.id,
                &ticket.parse().map_err(|e: IdParseError| anyhow!(e))?,
                request.parse().map_err(|e: IdParseError| anyhow!(e))?,
                d,
                *always,
                None,
            )?;
            println!("{}", out.result);
        }
        Cmd::Approve {
            project: pname,
            what,
            target,
        } => {
            let app = open(&root(&cli)?)?;
            let p = project(&app, pname)?;
            match what.as_str() {
                "spec" => app.approve_spec(p.id, target.parse()?)?,
                "plan" => app.approve_plan(p.id, target.parse()?)?,
                "role" => {
                    let r = app.decide_role_proposal(p.id, target, true, None, None)?;
                    if let Some(a) = r.agent {
                        println!("{} joined the team", app.agent(a)?.name);
                    }
                }
                _ => {
                    app.user_approve_merge(
                        p.id,
                        &target.parse().map_err(|e: IdParseError| anyhow!(e))?,
                    )?;
                }
            }
            println!("approved {what} {target}");
        }
        Cmd::Decline {
            project: pname,
            what: _,
            target,
            note,
        } => {
            let app = open(&root(&cli)?)?;
            let p = project(&app, pname)?;
            app.decide_role_proposal(p.id, target, false, None, note.as_deref())?;
            println!("declined role proposal {target}");
        }
        Cmd::Events { since, follow } => {
            let app = open(&root(&cli)?)?;
            let mut sub = app.subscribe(*since);
            for e in sub.catch_up().map_err(|e| anyhow!("{e:?}"))? {
                println!(
                    "{}\t{}\t{}\t{}\t{}",
                    e.seq, e.at, e.actor, e.kind, e.payload
                );
            }
            if *follow {
                eprintln!("(following; Ctrl+C to stop)");
                while let Ok(e) = sub.recv().await {
                    println!(
                        "{}\t{}\t{}\t{}\t{}",
                        e.seq, e.at, e.actor, e.kind, e.payload
                    );
                }
            }
        }
        Cmd::Backup { kind } => {
            let app = open(&root(&cli)?)?;
            if kind == "list" {
                for m in agenium_store::backup::list_sets(&app.backups_dir())? {
                    println!(
                        "{}\t{:?}\t{}\t{} db(s)",
                        m.id,
                        m.kind,
                        m.created_at,
                        m.dbs.len()
                    );
                }
                return Ok(());
            }
            let req = match kind.as_str() {
                "full" => BackupRequest::Full,
                "incr" => BackupRequest::Incremental,
                _ => BackupRequest::Scheduled,
            };
            match app.backup(req, chrono::Utc::now())? {
                Some(m) => println!(
                    "backup {} ({:?}) in {}",
                    m.id,
                    m.kind,
                    app.backups_dir().display()
                ),
                None => println!("no backup due"),
            }
            let cfg = app.config().backup;
            let pruned = agenium_store::backup::prune(
                &app.backups_dir(),
                agenium_store::backup::Retention {
                    hourly: cfg.hourly_keep as usize,
                    daily: cfg.daily_keep as usize,
                    weekly: cfg.weekly_keep as usize,
                },
            )?;
            if !pruned.is_empty() {
                println!("pruned {} old set(s)", pruned.len());
            }
        }
        Cmd::Restore { set, into } => {
            let r = root(&cli)?;
            let app = open(&r)?;
            let m = agenium_orchestrator::maintenance::restore_into_new_root(
                &r,
                &app.backups_dir(),
                set,
                into,
            )?;
            println!(
                "restored {} into {} (content hashes verified)",
                m.id,
                into.display()
            );
        }
    }
    Ok(())
}

fn dirs_home() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn agenium_backend_text(t: &Ticket) -> String {
    let mut s = format!(
        "{} [{}] {}\nplan item {} | priority P{} | template {}\n",
        t.key, t.status, t.title, t.plan_item, t.priority, t.report_template
    );
    s.push_str(&format!(
        "approved={} merged={} has_code={} reopened={} escalated={}\n",
        t.approved, t.merged, t.has_code, t.reopen_count, t.escalated
    ));
    for a in &t.acceptance {
        s.push_str(&format!("  {}: {}\n", a.id, a.text));
    }
    s
}
