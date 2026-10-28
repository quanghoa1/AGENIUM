//! `agenium`: headless front-end over the orchestrator. Every state change goes through the same App/Runtime API
//! the desktop app uses.

mod admin;
mod agent;
mod args;
mod mcp;
mod project;
mod provider;
mod role;
mod skill;
mod team;
mod ticket;
mod work;

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
        Cmd::Setup { .. }
        | Cmd::Support { .. }
        | Cmd::Report { .. }
        | Cmd::Doctor
        | Cmd::Secret(_)
        | Cmd::Backup { .. }
        | Cmd::Restore { .. } => admin::run(&cli).await?,
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
        Cmd::Project(_) => project::run(&cli).await?,
        Cmd::Agent(ac) => agent::run(&cli, ac).await?,
        Cmd::Skill(sc) => skill_cmd(&cli, sc).await?,
        Cmd::Mcp(mc) => mcp_cmd(&cli, mc).await?,
        Cmd::Models { action } => models_cmd(&cli, action).await?,
        Cmd::Chat { .. }
        | Cmd::Run { .. }
        | Cmd::Board { .. }
        | Cmd::Inbox { .. }
        | Cmd::Plan { .. }
        | Cmd::Answer { .. }
        | Cmd::Permission { .. }
        | Cmd::Approve { .. }
        | Cmd::Decline { .. }
        | Cmd::Events { .. } => work::run(&cli).await?,
        Cmd::Ticket(tc) => ticket::run(&cli, tc).await?,
        Cmd::Team(tc) => team::run(&cli, tc).await?,
        Cmd::Role(rc) => role::run(&cli, rc).await?,
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
