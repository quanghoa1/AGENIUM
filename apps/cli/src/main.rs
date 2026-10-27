//! `agenium`: headless front-end over the orchestrator. Every state change goes through the same App/Runtime API
//! the desktop app uses.

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
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "agenium",
    version,
    about = "Agenium: AI agents that work through tickets (headless CLI)"
)]
struct Cli {
    /// Agenium root (default: AGENIUM_HOME, then the root chosen at setup, then ~/.agenium).
    #[arg(long, global = true)]
    root: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// First-run setup: create the root, config.toml and (for a custom root) the bootstrap file.
    Setup {
        /// UI language: en | vi. Agents answer in the language you write in; tickets and plans are English.
        #[arg(long, default_value = "en")]
        ui_language: String,
    },
    /// System requirements, sandbox, keyring and reconcile report.
    Doctor,
    /// Write a support bundle (diagnostics, no secret values) to a folder.
    Support {
        #[arg(long, default_value = ".")]
        out: PathBuf,
    },
    /// Print a Markdown report of a project (goals, plan progress, tickets).
    Report {
        #[arg(long)]
        project: String,
        /// Write to this file instead of stdout.
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Model providers (API keys go to the encrypted vault, never to config.toml).
    #[command(subcommand)]
    Provider(ProviderCmd),
    /// Secrets in the vault.
    #[command(subcommand)]
    Secret(SecretCmd),
    #[command(subcommand)]
    Project(ProjectCmd),
    #[command(subcommand)]
    Agent(AgentCmd),
    /// Teams: compose from pool agents or create from a template, membership, Lead, roster.
    #[command(subcommand)]
    Team(TeamCmd),
    /// Roles agents can take (your own and template presets).
    #[command(subcommand)]
    Role(RoleCmd),
    /// Skills (SKILL.md folders) of agents and roles.
    #[command(subcommand)]
    Skill(SkillCmd),
    /// MCP servers (config.toml) and connection tests.
    #[command(subcommand)]
    Mcp(McpCmd),
    /// Local embedding model for agent memory (download, verify).
    Models {
        #[arg(value_parser = ["status", "download", "verify"], default_value = "status")]
        action: String,
    },
    /// Talk to the project's Lead/Solo agent (one turn).
    Chat {
        #[arg(long)]
        project: String,
        message: String,
    },
    /// Run the scheduler: work runs for ready tickets and Lead reviews, until idle.
    Run {
        #[arg(long)]
        project: String,
        #[arg(long, default_value_t = 20)]
        rounds: usize,
    },
    #[command(subcommand)]
    Ticket(TicketCmd),
    /// Tickets by status (Kanban columns), with blockers, open questions and flags.
    Board {
        #[arg(long)]
        project: String,
    },
    /// Requirement spec and plan (versions, approval) with progress per plan item.
    Plan {
        #[arg(long)]
        project: String,
    },
    /// The Lead's inbox (team mode), most urgent first; `agenium run` lets the Lead handle it.
    Inbox {
        #[arg(long)]
        project: String,
    },
    /// Answer an open question on a ticket.
    Answer {
        #[arg(long)]
        project: String,
        ticket: String,
        question: String,
        answer: String,
        #[arg(long)]
        option: Option<usize>,
    },
    /// Decide a permission request (`--always` adds it to the project allowlist).
    Permission {
        #[arg(long)]
        project: String,
        ticket: String,
        request: String,
        #[arg(value_parser = ["approve", "deny"])]
        decision: String,
        #[arg(long)]
        always: bool,
    },
    /// Approve a requirement spec or plan version, a merge, or a role the Lead proposed, waiting for you.
    Approve {
        #[arg(long)]
        project: String,
        #[arg(value_parser = ["spec", "plan", "merge", "role"])]
        what: String,
        /// Version (spec/plan), ticket key (merge) or proposal id (role, e.g. RP-01...).
        target: String,
    },
    /// Decline a role the Lead proposed (it sees your reason).
    Decline {
        #[arg(long)]
        project: String,
        #[arg(value_parser = ["role"])]
        what: String,
        /// Proposal id (RP-...).
        target: String,
        #[arg(long)]
        note: Option<String>,
    },
    /// Event log (resync source of every UI).
    Events {
        #[arg(long, default_value_t = 0)]
        since: u64,
        #[arg(long)]
        follow: bool,
    },
    /// Backups (hourly incremental / daily full; retention 24/7/4).
    Backup {
        #[arg(value_parser = ["auto", "full", "incr", "list"], default_value = "auto")]
        kind: String,
    },
    /// Restore a backup set into a new, empty root.
    Restore { set: String, into: PathBuf },
}

#[derive(Subcommand)]
enum ProviderCmd {
    /// Add or replace a provider. The key is read from --key-env or stdin. A known vendor needs no --kind or
    /// --base-url: name it with --preset (or use its preset id as the provider id; see `agenium provider presets`);
    /// its usual key variable (e.g. GROQ_API_KEY) is read when set.
    Add {
        id: String,
        #[arg(long, value_parser = ["anthropic", "gemini", "openai_compat"])]
        kind: Option<String>,
        /// A known vendor (`agenium provider presets`).
        #[arg(long)]
        preset: Option<String>,
        #[arg(long)]
        base_url: Option<String>,
        /// Environment variable holding the API key.
        #[arg(long)]
        key_env: Option<String>,
        /// Read the API key from stdin.
        #[arg(long)]
        key_stdin: bool,
        #[arg(long, default_value_t = 4)]
        max_concurrency: u32,
    },
    List,
    /// Known vendors: adapter, endpoint and the usual key variable (ADR-107).
    Presets,
    /// List models from the provider's API.
    Models {
        id: String,
    },
    /// Remove a provider no agent uses (its key leaves the vault too).
    Remove {
        id: String,
    },
    /// Local model servers answering on this machine (Ollama, LM Studio) and their models.
    Local,
}

#[derive(Subcommand)]
enum SecretCmd {
    /// Store a secret read from stdin.
    Set {
        name: String,
    },
    List,
    Delete {
        name: String,
    },
}

#[derive(Subcommand)]
enum ProjectCmd {
    /// Create a project (git repository + agenium.toml); adopts an existing folder with agenium.toml.
    Create {
        name: String,
        #[arg(long)]
        path: Option<PathBuf>,
        #[arg(long)]
        prefix: Option<String>,
        #[arg(long)]
        template: Option<String>,
        /// A git repository to clone into the projects folder.
        #[arg(long, conflicts_with = "path")]
        repo: Option<String>,
    },
    List,
    /// Stop a project's team (otherwise it runs from the agreed spec to the end).
    Pause {
        project: String,
    },
    /// Let a stopped project's team go on.
    Resume {
        project: String,
    },
    /// Project setting `review_before_merge` (Solo agents wait for you before merging).
    ReviewBeforeMerge {
        project: String,
        #[arg(value_parser = ["on", "off"])]
        value: String,
    },
}

#[derive(Subcommand)]
enum AgentCmd {
    /// Create an agent in the pool (no team yet, ADR-61): role, its own model, library skills, MCP servers.
    Create {
        #[arg(long)]
        name: String,
        /// Role id (`agenium role list`: your roles and the template presets).
        #[arg(long)]
        role: String,
        /// API provider id, or the CLI program with --cli.
        #[arg(long)]
        provider: String,
        #[arg(long, default_value = "")]
        model: String,
        #[arg(long, default_value = "high")]
        effort: String,
        /// Run through a vendor CLI (--provider names it: claude, codex, agy, grok, opencode).
        #[arg(long)]
        cli: bool,
        /// Capability tag (repeatable; default: the role's tags).
        #[arg(long = "tag")]
        tags: Vec<String>,
        /// Library skill to enable (repeatable).
        #[arg(long = "skill")]
        skills: Vec<String>,
        /// MCP server to enable (repeatable).
        #[arg(long = "mcp")]
        mcp: Vec<String>,
    },
    /// Create a Solo agent (Lead + Worker in one) for a project.
    Solo {
        #[arg(long)]
        project: String,
        #[arg(long)]
        name: String,
        #[arg(long)]
        provider: String,
        #[arg(long)]
        model: String,
        #[arg(long, default_value = "high")]
        effort: String,
    },
    List,
    /// Change an agent's model (memory, MCP and skills stay with the agent).
    Model {
        name: String,
        #[arg(long)]
        provider: String,
        #[arg(long)]
        model: String,
        #[arg(long, default_value = "high")]
        effort: String,
    },
    /// Run an agent through a vendor CLI you are logged in to (CliBackend, ADR-56); `agent model` switches back.
    Cli {
        name: String,
        /// CLI program (the CLIs `doctor` lists as usable: claude, codex, agy, grok, opencode).
        #[arg(long)]
        cli: String,
        /// The CLI's model (default: the CLI's own default).
        #[arg(long, default_value = "")]
        model: String,
    },
    /// Put an agent back to Idle after you fixed its quota/connectivity.
    Resume {
        name: String,
    },
    /// Library skills the agent enables (replaces the list; no names = none). Its role's and its own skills are
    /// always on.
    Skills {
        name: String,
        skills: Vec<String>,
    },
    /// Bind an MCP server to an agent (or --remove it).
    Mcp {
        name: String,
        #[arg(long, required_unless_present = "remove")]
        server: Option<String>,
        /// KEY=SECRET_NAME for ${cred:KEY} placeholders (repeatable).
        #[arg(long = "cred")]
        creds: Vec<String>,
        /// Only these tools (comma separated MCP names).
        #[arg(long, value_delimiter = ',')]
        tools: Option<Vec<String>>,
        #[arg(long, conflicts_with = "server")]
        remove: Option<String>,
    },
    /// Export an agent (profile, SOUL.md, skills, memory, conversations) to JSONL; `.zst` compresses.
    Export {
        name: String,
        file: PathBuf,
        /// Export even if secret-looking strings are found in its data.
        #[arg(long)]
        allow_secret_findings: bool,
    },
    /// Import an exported agent as the Solo agent of a project.
    Import {
        file: PathBuf,
        #[arg(long)]
        project: String,
        /// New name (default: the exported name).
        #[arg(long)]
        name: Option<String>,
    },
    /// Delete an agent for good: memory, skills, SOUL.md and its folder (blocked while it has open tickets).
    Delete {
        name: String,
        /// Type the agent's name again to confirm (asked interactively when missing).
        #[arg(long)]
        confirm: Option<String>,
        /// Export the agent to this file first.
        #[arg(long)]
        export: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum McpCmd {
    /// Add or replace a server: --command (stdio) or --url (Streamable HTTP). Values may use
    /// ${secret:<name>} (vault) or ${cred:<KEY>} (the agent's binding).
    Add {
        id: String,
        #[arg(long, conflicts_with = "url", required_unless_present = "url")]
        command: Option<String>,
        #[arg(long = "arg", allow_hyphen_values = true)]
        args: Vec<String>,
        #[arg(long)]
        url: Option<String>,
        /// KEY=VALUE (repeatable).
        #[arg(long = "env")]
        env: Vec<String>,
        /// Name=Value (repeatable).
        #[arg(long = "header")]
        headers: Vec<String>,
        #[arg(long, default_value_t = 120)]
        timeout: u64,
        /// OAuth client id registered for Agenium at the server's authorization server (only when it does not
        /// register clients itself; then sign in with `agenium mcp login`).
        #[arg(long)]
        oauth_client_id: Option<String>,
        /// Its secret, for a confidential client: `${secret:<name>}` (vault).
        #[arg(long)]
        oauth_client_secret: Option<String>,
        /// Scope to ask for (repeatable).
        #[arg(long = "oauth-scope")]
        oauth_scopes: Vec<String>,
        /// Fixed port of the sign-in redirect (http://127.0.0.1:<port>/callback).
        #[arg(long)]
        oauth_callback_port: Option<u16>,
    },
    /// Sign in to an HTTP server that asks for OAuth: opens the browser, keeps the tokens in the vault.
    Login {
        id: String,
        /// Scope to ask for (repeatable); default: what the server asks for.
        #[arg(long = "scope")]
        scopes: Vec<String>,
        /// Print the sign-in URL instead of opening the browser.
        #[arg(long)]
        no_browser: bool,
    },
    /// Forget the server's OAuth tokens.
    Logout {
        id: String,
    },
    /// Import pasted JSON ({"mcpServers": {...}}) from a file or "-" for stdin; secrets move to the vault.
    Import {
        file: String,
    },
    List,
    Remove {
        id: String,
    },
    /// Connect, list the tools, disconnect (with an agent's credentials when --agent is given).
    Test {
        id: String,
        #[arg(long)]
        agent: Option<String>,
    },
}

#[derive(Subcommand)]
enum SkillCmd {
    /// Skills an agent can use (role skills + its own), and folders that failed validation.
    List {
        #[arg(long)]
        agent: String,
    },
    /// Print a skill's instructions as the agent sees them (load_skill).
    Show {
        #[arg(long)]
        agent: String,
        name: String,
    },
    /// The system's skill library (agents enable these one by one).
    Library,
    /// Import a skill from a folder, a .zip or an https URL of a .zip.
    Import {
        source: String,
        /// Install for one agent.
        #[arg(long, conflicts_with_all = ["role", "library"], required_unless_present_any = ["role", "library"])]
        agent: Option<String>,
        /// Install for every agent of a role.
        #[arg(long, conflicts_with = "library")]
        role: Option<String>,
        /// Install into the system library.
        #[arg(long)]
        library: bool,
        #[arg(long)]
        replace: bool,
    },
    /// Delete a skill folder.
    Remove {
        name: String,
        #[arg(long, conflicts_with_all = ["role", "library"], required_unless_present_any = ["role", "library"])]
        agent: Option<String>,
        #[arg(long, conflicts_with = "library")]
        role: Option<String>,
        #[arg(long)]
        library: bool,
        /// Library skill still enabled by agents: switch it off for them.
        #[arg(long)]
        force: bool,
    },
    /// Check a skill folder without installing it.
    Validate { dir: PathBuf },
}

#[derive(Subcommand)]
enum TeamCmd {
    /// List team templates (built-in and <root>/templates).
    Templates,
    /// Print a template as TOML (start of a custom template).
    Template { id: String },
    /// Validate a template file and store it in <root>/templates (same id as a built-in overrides it).
    Import { file: PathBuf },
    /// Create the project's team from a template: one agent per role slot, named [<prefix>-]<role>[-n] (a taken name gets the next free number).
    Create {
        #[arg(long)]
        project: String,
        #[arg(long)]
        template: String,
        /// Model of every agent as provider/model (an API provider id from config.toml).
        #[arg(long)]
        model: String,
        /// Model of the Lead (provider/model), when it differs.
        #[arg(long)]
        lead_model: Option<String>,
        /// Model of the planner (provider/model), when the template has one and it should differ (a strong model:
        /// it only drafts plan v1).
        #[arg(long)]
        planner_model: Option<String>,
        #[arg(long, default_value = "high")]
        effort: String,
        /// Agent name prefix (default: none).
        #[arg(long)]
        prefix: Option<String>,
    },
    /// The project's team: Lead and workers with role, tags, status, model.
    Show {
        #[arg(long)]
        project: String,
    },
    /// Put another team member in the Lead slot (it must have no open tickets); the old Lead becomes a worker.
    Lead {
        #[arg(long)]
        project: String,
        /// Agent name.
        agent: String,
    },
    /// Every team and the agents in the pool.
    List,
    /// Compose a team from pool agents (ADR-61). `suggest` shows the Lead the system would pick.
    Compose {
        #[arg(long)]
        name: String,
        /// The Lead (agent name); omitted = the suggested one.
        #[arg(long)]
        lead: Option<String>,
        /// Worker (repeatable; the planner too, its role says so).
        #[arg(long = "member")]
        members: Vec<String>,
        /// Project to work on now.
        #[arg(long)]
        project: Option<String>,
        /// One agent as Lead and worker.
        #[arg(long)]
        solo: bool,
    },
    /// Add a pool agent to a team as a worker.
    Add { team: String, agent: String },
    /// Send a worker back to the pool.
    Remove { team: String, agent: String },
    /// Point a team at a project (`--none` frees it).
    Assign {
        team: String,
        #[arg(long, required_unless_present = "none")]
        project: Option<String>,
        #[arg(long)]
        none: bool,
    },
    /// Dissolve a team: its members go back to the pool.
    Dissolve { team: String },
}

#[derive(Subcommand)]
enum RoleCmd {
    /// Your roles and the template presets.
    List,
    /// Write a role (ROLE.md and role.toml).
    Put {
        id: String,
        /// File with the role's description (ROLE.md).
        #[arg(long)]
        prompt_file: PathBuf,
        #[arg(long = "tag")]
        tags: Vec<String>,
        /// Write scope (worktree-relative glob, repeatable).
        #[arg(long = "path")]
        allowed_paths: Vec<String>,
        #[arg(long)]
        planner: bool,
        /// Meant to lead a team (suggestion only).
        #[arg(long)]
        lead: bool,
    },
    Remove {
        id: String,
    },
    /// Roles the project's Lead proposed (decide with `approve role <id>` / `decline role <id>`).
    Proposals {
        #[arg(long)]
        project: String,
    },
}

#[derive(Subcommand)]
enum TicketCmd {
    List {
        #[arg(long)]
        project: String,
    },
    Show {
        #[arg(long)]
        project: String,
        key: String,
    },
    /// Add a User comment (steer the work).
    Comment {
        #[arg(long)]
        project: String,
        key: String,
        text: String,
    },
    /// Audit trail of a ticket (or of the whole project without KEY): who did what, when, applied or refused.
    Audit {
        #[arg(long)]
        project: String,
        key: Option<String>,
        #[arg(long, value_parser = ["table", "md", "jsonl"], default_value = "table")]
        format: String,
        /// Write to this file instead of stdout.
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Change the priority (0 = P0 highest ..= 3).
    Priority {
        #[arg(long)]
        project: String,
        key: String,
        priority: u8,
    },
    /// Assign a New ticket, or move a ticket that has not started (Assigned) or was Reopened, to an agent.
    Assign {
        #[arg(long)]
        project: String,
        key: String,
        /// Agent name.
        agent: String,
        #[arg(long)]
        note: Option<String>,
    },
    /// Review a Resolved ticket in place of the Lead. Reopen needs feedback per unmet criterion: --feedback AC-1="..."
    Review {
        #[arg(long)]
        project: String,
        key: String,
        #[arg(value_parser = ["approve", "reopen"])]
        decision: String,
        #[arg(long)]
        feedback: Vec<String>,
    },
    /// Close a Resolved ticket (code tickets must be merged).
    Close {
        #[arg(long)]
        project: String,
        key: String,
        reason: String,
    },
    /// Cancel a ticket that has not ended (stops its run).
    Cancel {
        #[arg(long)]
        project: String,
        key: String,
        reason: String,
    },
}

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

async fn mcp_cmd(cli: &Cli, mc: &McpCmd) -> Result<()> {
    let app = open(&root(cli)?)?;
    match mc {
        McpCmd::Add {
            id,
            command,
            args,
            url,
            env,
            headers,
            timeout,
            oauth_client_id,
            oauth_client_secret,
            oauth_scopes,
            oauth_callback_port,
        } => {
            let oauth = (oauth_client_id.is_some()
                || oauth_client_secret.is_some()
                || !oauth_scopes.is_empty()
                || oauth_callback_port.is_some())
            .then(|| agenium_config::McpOAuthConfig {
                client_id: oauth_client_id.clone(),
                client_secret: oauth_client_secret.clone(),
                scopes: oauth_scopes.clone(),
                callback_port: *oauth_callback_port,
            });
            app.put_mcp_server(agenium_config::McpServerConfig {
                id: id.clone(),
                command: command.clone(),
                args: args.clone(),
                env: pairs(env)?,
                url: url.clone(),
                headers: pairs(headers)?,
                timeout_secs: *timeout,
                oauth,
            })?;
            println!("MCP server {id} saved (test it: agenium mcp test {id})");
        }
        McpCmd::Import { file } => {
            let text = if file == "-" {
                let mut s = String::new();
                std::io::Read::read_to_string(&mut std::io::stdin(), &mut s)?;
                s
            } else {
                std::fs::read_to_string(file)?
            };
            let r = app.import_mcp_json(&text)?;
            println!("imported: {}", r.servers.join(", "));
        }
        McpCmd::List => {
            let hub = agenium_orchestrator::mcp::hub(app.clone());
            for m in app.config().mcp {
                let target = match (&m.command, &m.url) {
                    (Some(c), _) => format!("stdio: {c} {}", m.args.join(" ")),
                    (_, Some(u)) if hub.signed_in(&m) => format!("http: {u} (signed in)"),
                    (_, Some(u)) => format!("http: {u}"),
                    _ => "?".into(),
                };
                println!("{}	{}", m.id, target.trim_end());
            }
        }
        McpCmd::Remove { id } => {
            app.remove_mcp_server(id)?;
            println!("removed {id}");
        }
        McpCmd::Login {
            id,
            scopes,
            no_browser,
        } => {
            let cfg = app
                .config()
                .mcp_server(id)
                .cloned()
                .ok_or_else(|| anyhow!("no MCP server {id}"))?;
            let hub = agenium_orchestrator::mcp::hub(app.clone());
            let no_browser = *no_browser;
            let open = move |url: &str| {
                if no_browser || !agenium_mcp::oauth::open_browser(url) {
                    println!("Open this URL to sign in:\n{url}");
                } else {
                    println!("Signing in through your browser… (if it did not open: {url})");
                }
            };
            let t = hub.login(&cfg, scopes, &open).await?;
            println!(
                "signed in to {id} (client {}{})",
                t.client_id,
                t.scope
                    .as_deref()
                    .map(|s| format!(", scope {s}"))
                    .unwrap_or_default()
            );
        }
        McpCmd::Logout { id } => {
            agenium_orchestrator::mcp::hub(app.clone())
                .logout(id)
                .await?;
            println!("signed out of {id}");
        }
        McpCmd::Test { id, agent } => {
            let cfg = app
                .config()
                .mcp_server(id)
                .cloned()
                .ok_or_else(|| anyhow!("no MCP server {id}"))?;
            let binding = match agent {
                Some(n) => agenium_mcp::parse_bindings(&app.agent_named(n)?.mcp)?
                    .into_iter()
                    .find(|b| &b.server == id)
                    .ok_or_else(|| anyhow!("{n} is not bound to {id}"))?,
                None => agenium_mcp::Binding {
                    server: id.clone(),
                    credentials: Default::default(),
                    keys: Default::default(),
                    tools: None,
                },
            };
            let hub = agenium_orchestrator::mcp::hub(app.clone());
            let (info, protocol, tools) = hub.test(&cfg, &binding).await?;
            println!(
                "connected: {} {} (protocol {protocol})",
                info["name"].as_str().unwrap_or("?"),
                info["version"].as_str().unwrap_or("")
            );
            for t in tools {
                let ro = if t.read_only { " [read-only]" } else { "" };
                println!(
                    "  {}{ro}: {}",
                    t.name,
                    t.description.lines().next().unwrap_or("")
                );
            }
        }
    }
    Ok(())
}

fn skill_owner(
    app: &App,
    agent: &Option<String>,
    role: &Option<String>,
) -> Result<agenium_skills::Owner> {
    Ok(match (agent, role) {
        (Some(a), _) => agenium_skills::Owner::Agent(app.agent_named(a)?.id.to_string()),
        (None, Some(r)) => {
            if !agenium_skills::valid_name(r) {
                bail!("invalid role id {r:?}");
            }
            agenium_skills::Owner::Role(r.clone())
        }
        (None, None) => agenium_skills::Owner::Library,
    })
}

/// Downloads an https .zip (at most the skill size limit) into a temporary file.
async fn download_zip(url: &str, dir: &Path) -> Result<PathBuf> {
    if !url.starts_with("https://") {
        bail!("only https URLs are accepted");
    }
    let mut resp = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()?
        .get(url)
        .send()
        .await?
        .error_for_status()?;
    let path = dir.join("skill-download.zip");
    let mut f = std::fs::File::create(&path)?;
    let mut total = 0u64;
    while let Some(chunk) = resp.chunk().await? {
        total += chunk.len() as u64;
        if total > agenium_skills::MAX_TOTAL_BYTES {
            bail!(
                "the download is larger than {} MiB",
                agenium_skills::MAX_TOTAL_BYTES >> 20
            );
        }
        f.write_all(&chunk)?;
    }
    Ok(path)
}

async fn skill_cmd(cli: &Cli, sc: &SkillCmd) -> Result<()> {
    if let SkillCmd::Validate { dir } = sc {
        let s = agenium_skills::validate(dir, agenium_skills::Origin::Agent)?;
        println!(
            "ok: {} - {} ({} files)",
            s.meta.name,
            s.meta.description,
            s.files.len()
        );
        return Ok(());
    }
    let app = open(&root(cli)?)?;
    let reg = app.skill_registry();
    match sc {
        SkillCmd::List { agent } => {
            let a = app.agent_named(agent)?;
            let set = reg.for_agent(&a.id.to_string(), &a.role, &a.skills);
            if set.is_empty() {
                println!("(no skills)");
            }
            for s in &set.skills {
                let origin = serde_json::to_value(s.meta.origin)?;
                println!(
                    "{}	{}	{}	{}",
                    s.meta.name,
                    origin.as_str().unwrap_or_default(),
                    s.meta.version.as_deref().unwrap_or("-"),
                    s.meta.description
                );
            }
            for p in &set.problems {
                println!("INVALID	{}	{}", p.dir.display(), p.error);
            }
            if !a.skills.is_empty() {
                println!("(library skills enabled: {})", a.skills.join(", "));
            }
        }
        SkillCmd::Show { agent, name } => {
            let a = app.agent_named(agent)?;
            let set = reg.for_agent(&a.id.to_string(), &a.role, &a.skills);
            let s = set
                .get(name)
                .ok_or_else(|| anyhow!("{agent} has no skill {name:?}"))?;
            println!("{}", s.instructions());
        }
        SkillCmd::Library => {
            let set = reg.library();
            if set.is_empty() {
                println!("(the library is empty: agenium skill import <folder|zip> --library)");
            }
            let agents = app.agents()?;
            for s in &set.skills {
                let users: Vec<&str> = agents
                    .iter()
                    .filter(|a| a.skills.contains(&s.meta.name))
                    .map(|a| a.name.as_str())
                    .collect();
                println!(
                    "{}\t{}\t{}\t{}",
                    s.meta.name,
                    s.meta.version.as_deref().unwrap_or("-"),
                    s.meta.description,
                    if users.is_empty() {
                        "-".to_owned()
                    } else {
                        users.join(",")
                    }
                );
            }
            for p in &set.problems {
                println!("INVALID\t{}\t{}", p.dir.display(), p.error);
            }
        }
        SkillCmd::Import {
            source,
            agent,
            role,
            library: _,
            replace,
        } => {
            let owner = skill_owner(&app, agent, role)?;
            let tmp;
            let src = if source.starts_with("http://") || source.starts_with("https://") {
                tmp = std::env::temp_dir().join(format!("agenium-skill-{}", std::process::id()));
                std::fs::create_dir_all(&tmp)?;
                let r = download_zip(source, &tmp).await;
                if r.is_err() {
                    let _ = std::fs::remove_dir_all(&tmp);
                }
                r?
            } else {
                PathBuf::from(source)
            };
            let r = reg.import(&src, &owner, *replace);
            if source.starts_with("https://") {
                let _ = std::fs::remove_dir_all(src.parent().unwrap_or(&src));
            }
            let m = r?;
            println!("imported {} into {}", m.name, m.dir.display());
        }
        SkillCmd::Remove {
            name,
            agent,
            role,
            library: _,
            force,
        } => {
            match skill_owner(&app, agent, role)? {
                agenium_skills::Owner::Library => app.remove_library_skill(name, *force)?,
                owner => reg.remove(&owner, name)?,
            }
            println!("removed {name}");
        }
        SkillCmd::Validate { .. } => unreachable!(),
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
        Cmd::Provider(pc) => {
            let app = open(&root(&cli)?)?;
            match pc {
                ProviderCmd::Add {
                    id,
                    kind,
                    preset,
                    base_url,
                    key_env,
                    key_stdin,
                    max_concurrency,
                } => {
                    let known = match preset {
                        Some(n) => Some(agenium_config::presets::preset(n).ok_or_else(|| {
                            anyhow!("no preset {n:?} (see agenium provider presets)")
                        })?),
                        None if kind.is_none() => agenium_config::presets::preset(id),
                        None => None,
                    };
                    let kind = match (kind.as_deref(), known) {
                        (Some(k), _) => k.to_owned(),
                        (None, Some(pr)) => match pr.kind {
                            ProviderKind::Anthropic => "anthropic",
                            ProviderKind::Gemini => "gemini",
                            _ => "openai_compat",
                        }
                        .to_owned(),
                        (None, None) => bail!(
                            "give --kind, or --preset with a known vendor (agenium provider presets)"
                        ),
                    };
                    let base_url = base_url
                        .clone()
                        .or_else(|| known.and_then(|pr| pr.base_url.map(str::to_owned)));
                    if let Some(ph) = base_url
                        .as_deref()
                        .and_then(agenium_config::presets::placeholder)
                    {
                        bail!(
                            "the base URL {} has {ph} to fill in: pass --base-url with your own value{}",
                            base_url.as_deref().unwrap_or(""),
                            known
                                .map(|pr| format!(" ({})", pr.note))
                                .unwrap_or_default()
                        );
                    }
                    // The vendor's usual variable is read only when it is set (its value is never printed).
                    let usual = known
                        .map(|pr| pr.key_env)
                        .filter(|v| !v.is_empty() && std::env::var(v).is_ok());
                    let key = match (key_env.as_deref().or(usual), key_stdin) {
                        (Some(v), _) => Some(
                            std::env::var(v)
                                .with_context(|| format!("environment variable {v} is not set"))?,
                        ),
                        (None, true) => Some(read_secret_stdin()?),
                        (None, false) => None,
                    };
                    if key.is_none() && known.is_some_and(|pr| !pr.key_env.is_empty()) {
                        eprintln!(
                            "note: no key given; set {} or pass --key-env/--key-stdin",
                            known.map(|pr| pr.key_env).unwrap_or_default()
                        );
                    }
                    let secret_name = format!("provider/{id}");
                    if let Some(k) = &key {
                        app.secrets().set(&secret_name, k.as_bytes())?;
                    }
                    let kind = match kind.as_str() {
                        "anthropic" => ProviderKind::Anthropic,
                        "gemini" => ProviderKind::Gemini,
                        _ => ProviderKind::OpenaiCompat,
                    };
                    let pc = ProviderConfig {
                        body: vec![],
                        effort_field: None,
                        headers: Default::default(),
                        id: id.clone(),
                        kind,
                        base_url: base_url.clone(),
                        api_key_ref: if key.is_some() {
                            secret_name
                        } else {
                            String::new()
                        },
                        max_concurrency: *max_concurrency,
                        models: vec![],
                        model_caps: vec![],
                    };
                    app.update_config(|c| {
                        c.providers.retain(|p| p.id != pc.id);
                        c.providers.push(pc);
                    })?;
                    println!(
                        "provider {id} saved{}",
                        if key.is_some() {
                            " (key stored in the vault)"
                        } else {
                            ""
                        }
                    );
                }
                ProviderCmd::Remove { id } => {
                    app.remove_provider(id)?;
                    println!("provider {id} removed");
                }
                ProviderCmd::Local => {
                    let http =
                        agenium_llm::http::Http::default_client().map_err(|e| anyhow!("{e}"))?;
                    let found = agenium_llm::local::discover(&http).await;
                    if found.is_empty() {
                        println!(
                            "no local model server answers (Ollama on 11434, LM Studio on 1234)"
                        );
                    }
                    for s in found {
                        println!("{}\t{}\t{}", s.kind, s.base_url, s.models.join(", "));
                        println!(
                            "  add: agenium provider add {} --kind openai_compat --base-url {}",
                            s.kind, s.base_url
                        );
                    }
                }
                ProviderCmd::Presets => unreachable!("listed without a root"),
                ProviderCmd::List => {
                    for p in app.config().providers {
                        println!(
                            "{}\t{:?}\t{}\t{}",
                            p.id,
                            p.kind,
                            p.base_url.unwrap_or_default(),
                            if p.api_key_ref.is_empty() {
                                "no key"
                            } else {
                                "key in vault"
                            }
                        );
                    }
                }
                ProviderCmd::Models { id } => {
                    let ps = ConfiguredProviders::new(app.clone());
                    let p = agenium_orchestrator::runtime::ProviderSource::provider(&ps, id)?;
                    let pc = app.config().provider(id).cloned();
                    let flag = |b: Option<bool>| match b {
                        Some(true) => "yes",
                        Some(false) => "no",
                        None => "?",
                    };
                    for mut m in p.list_models().await.map_err(|e| anyhow!("{e}"))? {
                        if let Some(pc) = &pc {
                            agenium_orchestrator::runtime::apply_caps(pc, &mut m);
                        }
                        println!(
                            "{}\ttools {}\tvision {}\t{}{}",
                            m.id,
                            flag(m.supports_tools),
                            flag(m.supports_vision),
                            m.context_window
                                .map(|c| format!("{c} tokens"))
                                .unwrap_or_default(),
                            if m.supports_tools == Some(false) {
                                "\t(no tool calling: Agenium agents cannot use it)"
                            } else {
                                ""
                            }
                        );
                    }
                }
            }
        }
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
        Cmd::Agent(ac) => {
            let app = open(&root(&cli)?)?;
            match ac {
                AgentCmd::Solo {
                    project: pname,
                    name,
                    provider,
                    model,
                    effort,
                } => {
                    let p = project(&app, pname)?;
                    if app.config().provider(provider).is_none() {
                        bail!("provider {provider:?} is not configured (agenium provider add ...)");
                    }
                    let binding = ModelBinding {
                        provider: provider.clone(),
                        model: model.clone(),
                        effort: effort.parse().map_err(|e: String| anyhow!(e))?,
                        max_tokens: None,
                    };
                    let (_, a) = app.create_solo(
                        p.id,
                        &CreateAgent {
                            name: name.clone(),
                            role: "solo".into(),
                            role_kind: RoleKind::Lead,
                            tags: vec![],
                            model: binding,
                        },
                    )?;
                    println!("solo agent {} ({}) on {}", a.name, a.id, p.name);
                }
                AgentCmd::Create {
                    name,
                    role,
                    provider,
                    model,
                    effort,
                    cli: via_cli,
                    tags,
                    skills,
                    mcp,
                } => {
                    let a = app.create_agent(&agenium_orchestrator::registry::NewAgent {
                        name: name.clone(),
                        role: role.clone(),
                        tags: (!tags.is_empty()).then(|| tags.clone()),
                        model: ModelBinding {
                            provider: provider.clone(),
                            model: model.clone(),
                            effort: effort.parse().map_err(|e: String| anyhow!(e))?,
                            max_tokens: None,
                        },
                        backend: if *via_cli {
                            agenium_core::BackendKind::Cli
                        } else {
                            agenium_core::BackendKind::Api
                        },
                        skills: skills.clone(),
                        mcp: mcp
                            .iter()
                            .map(|s| agenium_mcp::Binding {
                                server: s.clone(),
                                credentials: Default::default(),
                                keys: Default::default(),
                                tools: None,
                            })
                            .collect(),
                    })?;
                    println!(
                        "agent {} ({}) created in the pool, role {}; next: agenium team compose ...",
                        a.name, a.id, a.role
                    );
                }
                AgentCmd::List => {
                    for a in app.agents()? {
                        println!(
                            "{}\t{}\t{}\t{}{}/{} ({})",
                            a.name,
                            a.id,
                            a.status,
                            if a.backend == BackendKind::Cli {
                                "cli:"
                            } else {
                                ""
                            },
                            a.model.provider,
                            a.model.model,
                            a.model.effort.as_str()
                        );
                    }
                }
                AgentCmd::Model {
                    name,
                    provider,
                    model,
                    effort,
                } => {
                    let a = app
                        .agents()?
                        .into_iter()
                        .find(|a| &a.name == name)
                        .ok_or_else(|| anyhow!("no agent {name}"))?;
                    app.set_agent_backend(
                        a.id,
                        BackendKind::Api,
                        &ModelBinding {
                            provider: provider.clone(),
                            model: model.clone(),
                            effort: effort.parse().map_err(|e: String| anyhow!(e))?,
                            max_tokens: None,
                        },
                    )?;
                    println!("{name} now uses {provider}/{model}");
                }
                AgentCmd::Cli { name, cli, model } => {
                    let a = app
                        .agents()?
                        .into_iter()
                        .find(|a| &a.name == name)
                        .ok_or_else(|| anyhow!("no agent {name}"))?;
                    app.set_agent_backend(
                        a.id,
                        BackendKind::Cli,
                        &ModelBinding {
                            provider: cli.clone(),
                            model: model.clone(),
                            effort: a.model.effort,
                            max_tokens: None,
                        },
                    )?;
                    println!("{name} now runs through the {cli} CLI");
                    let driven = agenium_cli_backend::CliKind::ALL
                        .into_iter()
                        .find(|k| k.program() == cli)
                        .and_then(agenium_cli_backend::driver)
                        .is_some();
                    if !driven {
                        println!("note: Agenium does not drive {cli} yet; its runs will fail (see `agenium doctor`)");
                    }
                }
                AgentCmd::Skills { name, skills } => {
                    let a = app.agent_named(name)?;
                    app.set_agent_skills(a.id, skills)?;
                    if skills.is_empty() {
                        println!("{name} uses only the skills of role {} and its own", a.role);
                    } else {
                        println!("{name} enables the library skills: {}", skills.join(", "));
                    }
                }
                AgentCmd::Mcp {
                    name,
                    server,
                    creds,
                    tools,
                    remove,
                } => {
                    let a = app.agent_named(name)?;
                    let mut bindings = agenium_mcp::parse_bindings(&a.mcp)?;
                    if let Some(r) = remove {
                        bindings.retain(|b| &b.server != r);
                    }
                    if let Some(s) = server {
                        bindings.retain(|b| &b.server != s);
                        bindings.push(agenium_mcp::Binding {
                            server: s.clone(),
                            credentials: pairs(creds)?,
                            keys: Default::default(),
                            tools: tools.clone(),
                        });
                    }
                    app.set_agent_mcp(a.id, &bindings)?;
                    let list: Vec<&str> = bindings.iter().map(|b| b.server.as_str()).collect();
                    println!(
                        "{name} MCP servers: {}",
                        if list.is_empty() {
                            "(none)".into()
                        } else {
                            list.join(", ")
                        }
                    );
                }
                AgentCmd::Export {
                    name,
                    file,
                    allow_secret_findings,
                } => {
                    let a = app.agent_named(name)?;
                    let s = app.export_agent(
                        a.id,
                        file,
                        app.embedder().as_ref(),
                        &agenium_orchestrator::agent_io::ExportOptions {
                            allow_secret_findings: *allow_secret_findings,
                        },
                    )?;
                    println!(
                        "exported {name} to {} ({} files, {} memory rows)",
                        s.path.display(),
                        s.files,
                        s.memory_rows
                    );
                }
                AgentCmd::Import {
                    file,
                    project: pname,
                    name,
                } => {
                    let p = project(&app, pname)?;
                    let a = app.import_agent(
                        file,
                        &agenium_orchestrator::agent_io::ImportRequest {
                            project: p.id,
                            name: name.clone(),
                        },
                        app.embedder().as_ref(),
                    )?;
                    if app.config().provider(&a.model.provider).is_none() {
                        println!(
                            "note: provider {:?} is not configured here; add it or run `agenium agent model {} ...`",
                            a.model.provider, a.name
                        );
                    }
                    println!(
                        "imported {} ({}) as the Solo agent of {}",
                        a.name, a.id, p.name
                    );
                }
                AgentCmd::Delete {
                    name,
                    confirm,
                    export,
                } => {
                    let a = app.agent_named(name)?;
                    if let Some(file) = export {
                        app.export_agent(a.id, file, app.embedder().as_ref(), &Default::default())?;
                        println!("exported {name} to {}", file.display());
                    }
                    let typed = match confirm {
                        Some(c) => c.clone(),
                        None if interactive() => {
                            eprintln!(
                                "This permanently deletes {name}'s memory, learned skills, SOUL.md history and folder."
                            );
                            let mut out = std::io::stderr();
                            let _ = write!(out, "Type the agent's name to confirm: ");
                            let _ = out.flush();
                            let mut line = String::new();
                            std::io::stdin().lock().read_line(&mut line)?;
                            line.trim().to_owned()
                        }
                        None => bail!("pass --confirm <name> to delete without a terminal"),
                    };
                    let r = app.delete_agent(a.id, &typed)?;
                    println!("deleted {}", r.name);
                    for p in r.pending {
                        println!("still locked, removed at next start: {p}");
                    }
                }
                AgentCmd::Resume { name } => {
                    let a = app
                        .agents()?
                        .into_iter()
                        .find(|a| &a.name == name)
                        .ok_or_else(|| anyhow!("no agent {name}"))?;
                    app.set_agent_status(a.id, AgentStatus::Idle)?;
                    println!("{name} is Idle");
                }
            }
        }
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
        Cmd::Ticket(tc) => {
            let app = open(&root(&cli)?)?;
            match tc {
                TicketCmd::List { project: pname } => {
                    let p = project(&app, pname)?;
                    for t in app.tickets(p.id)? {
                        println!(
                            "{}\t{}\t{}{}{}\t{}",
                            t.key,
                            t.status,
                            if t.approved { "approved " } else { "" },
                            if t.merged { "merged " } else { "" },
                            t.plan_item,
                            t.title
                        );
                    }
                }
                TicketCmd::Show {
                    project: pname,
                    key,
                } => {
                    let p = project(&app, pname)?;
                    let t =
                        app.ticket(p.id, &key.parse().map_err(|e: IdParseError| anyhow!(e))?)?;
                    println!("{}", agenium_backend_text(&t));
                    for c in app.store().read(|c| comments::for_ticket(c, t.id))? {
                        println!(
                            "[{}] {} {}: {}",
                            c.created_at,
                            c.kind,
                            c.id,
                            serde_json::to_string(&c.body)?
                                .chars()
                                .take(400)
                                .collect::<String>()
                        );
                    }
                    for r in app.store().read(|c| runs::for_ticket(c, t.id))? {
                        println!(
                            "run {} {} {} steps={} tokens={}/{}",
                            r.id,
                            r.started_at,
                            r.outcome.unwrap_or_else(|| "live".into()),
                            r.steps,
                            r.usage.tokens_in,
                            r.usage.tokens_out
                        );
                    }
                }
                TicketCmd::Comment {
                    project: pname,
                    key,
                    text,
                } => {
                    let p = project(&app, pname)?;
                    app.user_comment(
                        p.id,
                        &key.parse().map_err(|e: IdParseError| anyhow!(e))?,
                        text,
                    )?;
                    println!("comment added");
                }
                TicketCmd::Audit {
                    project: pname,
                    key,
                    format,
                    out,
                } => {
                    let p = project(&app, pname)?;
                    let k: Option<TicketKey> = key
                        .as_deref()
                        .map(|k| k.parse().map_err(|e: IdParseError| anyhow!(e)))
                        .transpose()?;
                    let entries = app.audit(p.id, k.as_ref())?;
                    let text = match format.as_str() {
                        "md" => agenium_orchestrator::views::audit_markdown(
                            &k.as_ref().map(|k| k.to_string()).unwrap_or(p.name.clone()),
                            &entries,
                        ),
                        "jsonl" => entries
                            .iter()
                            .map(|e| serde_json::to_string(e).map(|l| l + "\n"))
                            .collect::<Result<String, _>>()?,
                        _ => entries
                            .iter()
                            .map(|e| {
                                format!(
                                    "{}\t{}\t{}\t{}\t{}\t{}\n",
                                    e.at,
                                    e.actor,
                                    e.ticket.as_ref().map(|k| k.to_string()).unwrap_or_default(),
                                    e.action,
                                    if e.accepted { "applied" } else { "REFUSED" },
                                    e.detail.to_string().chars().take(200).collect::<String>()
                                )
                            })
                            .collect(),
                    };
                    match out {
                        Some(path) => {
                            std::fs::write(path, text)?;
                            println!("{} entries written to {}", entries.len(), path.display());
                        }
                        None => print!("{text}"),
                    }
                }
                TicketCmd::Priority {
                    project: pname,
                    key,
                    priority,
                } => {
                    let p = project(&app, pname)?;
                    let t = app.user_set_priority(
                        p.id,
                        &key.parse().map_err(|e: IdParseError| anyhow!(e))?,
                        *priority,
                    )?;
                    println!("{} is P{}", t.key, t.priority);
                }
                TicketCmd::Assign {
                    project: pname,
                    key,
                    agent,
                    note,
                } => {
                    let p = project(&app, pname)?;
                    let a = app
                        .agents()?
                        .into_iter()
                        .find(|a| &a.name == agent)
                        .ok_or_else(|| anyhow!("no agent named {agent}"))?;
                    user_intent(
                        &app,
                        p.id,
                        Intent::AssignTicket(forms::AssignTicket {
                            ticket_id: key.parse().map_err(|e: IdParseError| anyhow!(e))?,
                            worker: a.id,
                            note: note.clone(),
                        }),
                    )?;
                }
                TicketCmd::Review {
                    project: pname,
                    key,
                    decision,
                    feedback,
                } => {
                    let p = project(&app, pname)?;
                    let ac_feedback = feedback
                        .iter()
                        .map(|f| {
                            let (ac, text) = f
                                .split_once('=')
                                .ok_or_else(|| anyhow!("--feedback takes AC-n=\"text\""))?;
                            Ok(forms::AcFeedback {
                                ac_id: ac.trim().to_owned(),
                                feedback: text.trim().to_owned(),
                            })
                        })
                        .collect::<Result<Vec<_>>>()?;
                    user_intent(
                        &app,
                        p.id,
                        Intent::ReviewTicket(forms::ReviewDecision {
                            ticket_id: key.parse().map_err(|e: IdParseError| anyhow!(e))?,
                            decision: if decision == "approve" {
                                forms::ReviewVerdict::Approve
                            } else {
                                forms::ReviewVerdict::Reopen
                            },
                            ac_feedback,
                            required_changes: vec![],
                        }),
                    )?;
                }
                TicketCmd::Close {
                    project: pname,
                    key,
                    reason,
                } => {
                    let p = project(&app, pname)?;
                    user_intent(
                        &app,
                        p.id,
                        Intent::CloseTicket(forms::CloseTicket {
                            ticket_id: key.parse().map_err(|e: IdParseError| anyhow!(e))?,
                            reason: reason.clone(),
                        }),
                    )?;
                }
                TicketCmd::Cancel {
                    project: pname,
                    key,
                    reason,
                } => {
                    let p = project(&app, pname)?;
                    user_intent(
                        &app,
                        p.id,
                        Intent::CancelTicket(forms::CancelTicket {
                            ticket_id: key.parse().map_err(|e: IdParseError| anyhow!(e))?,
                            reason: reason.clone(),
                        }),
                    )?;
                }
            }
        }
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
        Cmd::Team(tc) => {
            let app = open(&root(&cli)?)?;
            match tc {
                TeamCmd::Templates => {
                    for t in app.team_templates()? {
                        let roles: Vec<String> = t
                            .roles
                            .iter()
                            .map(|r| {
                                if r.count > 1 {
                                    format!("{} x{}", r.id, r.count)
                                } else {
                                    r.id.clone()
                                }
                            })
                            .collect();
                        println!(
                            "{}@{}	{}	[{}]	{}",
                            t.id,
                            t.version,
                            t.name,
                            roles.join(", "),
                            t.description
                        );
                    }
                }
                TeamCmd::Template { id } => print!("{}", app.team_template(id)?.to_toml()),
                TeamCmd::Import { file } => {
                    let t = app.import_team_template(file)?;
                    println!("team template {}@{} stored", t.id, t.version);
                }
                TeamCmd::Create {
                    project: pname,
                    template,
                    model,
                    lead_model,
                    planner_model,
                    effort,
                    prefix,
                } => {
                    let p = project(&app, pname)?;
                    let binding = |pm: &str| -> Result<ModelBinding> {
                        let (provider, model) = pm
                            .split_once('/')
                            .ok_or_else(|| anyhow!("{pm:?}: use provider/model"))?;
                        if app.config().provider(provider).is_none() {
                            bail!("provider {provider:?} is not configured (agenium provider add ...)");
                        }
                        Ok(ModelBinding {
                            provider: provider.into(),
                            model: model.into(),
                            effort: effort.parse().map_err(|e: String| anyhow!(e))?,
                            max_tokens: None,
                        })
                    };
                    let m = binding(model)?;
                    let lm = lead_model.as_deref().map(binding).transpose()?;
                    let pm = planner_model.as_deref().map(binding).transpose()?;
                    let out = app.create_team_from_template(
                        p.id,
                        template,
                        &m,
                        lm.as_ref(),
                        pm.as_ref(),
                        prefix.as_deref().unwrap_or(""),
                    )?;
                    println!("team created on {} from {template}:", p.name);
                    for (name, role) in &out.agents {
                        println!("  {name}	{role}");
                    }
                    for s in &out.missing_skills {
                        println!("  note: skill not installed yet: {s}");
                    }
                    println!("Next: agenium chat --project {} \"<your request>\" (the Lead plans; you approve spec and plan)", p.name);
                }
                TeamCmd::Lead {
                    project: pname,
                    agent,
                } => {
                    let p = project(&app, pname)?;
                    let a = app
                        .agents()?
                        .into_iter()
                        .find(|a| &a.name == agent)
                        .ok_or_else(|| anyhow!("no agent named {agent}"))?;
                    app.set_team_lead(p.id, a.id)?;
                    println!("{} is the Lead of {} now", a.name, p.name);
                }
                TeamCmd::List => {
                    for (t, members) in app.teams()? {
                        let project = match t.project {
                            Some(p) => app.project(p)?.name,
                            None => "-".into(),
                        };
                        println!(
                            "{}{}\tproject {project}",
                            t.name,
                            if t.solo { " (solo)" } else { "" }
                        );
                        for a in members {
                            let lead = if t.lead == Some(a.id) { " (Lead)" } else { "" };
                            println!(
                                "  {}{lead}\t{}\t{}/{}",
                                a.name, a.role, a.model.provider, a.model.model
                            );
                        }
                    }
                    let pool: Vec<_> = app
                        .agents()?
                        .into_iter()
                        .filter(|a| a.team.is_none())
                        .collect();
                    println!(
                        "pool: {}",
                        if pool.is_empty() {
                            "-".to_owned()
                        } else {
                            pool.iter()
                                .map(|a| format!("{} ({})", a.name, a.role))
                                .collect::<Vec<_>>()
                                .join(", ")
                        }
                    );
                }
                TeamCmd::Compose {
                    name,
                    lead,
                    members,
                    project: pname,
                    solo,
                } => {
                    let members: Vec<_> = members
                        .iter()
                        .map(|m| app.agent_named(m).map(|a| a.id))
                        .collect::<std::result::Result<_, _>>()?;
                    let lead = match lead {
                        Some(l) => app.agent_named(l)?.id,
                        None => {
                            let s = app.suggest_lead(&members)?.ok_or_else(|| {
                                anyhow!("no agent stands out as Lead: pass --lead")
                            })?;
                            println!(
                                "suggested Lead: {} ({})",
                                app.agent(s.agent)?.name,
                                s.reasons.join(", ")
                            );
                            s.agent
                        }
                    };
                    let members: Vec<_> = members.into_iter().filter(|m| *m != lead).collect();
                    let project = pname.as_deref().map(|n| project(&app, n)).transpose()?;
                    let t = app.compose_team(&agenium_orchestrator::registry::NewTeam {
                        name: name.clone(),
                        project: project.map(|p| p.id),
                        lead,
                        members,
                        solo: *solo,
                    })?;
                    println!("team {} composed ({})", t.name, t.id);
                }
                TeamCmd::Add { team, agent } => {
                    let t = team_named(&app, team)?;
                    app.add_team_member(t, app.agent_named(agent)?.id)?;
                    println!("{agent} joined {team}");
                }
                TeamCmd::Remove { team, agent } => {
                    let t = team_named(&app, team)?;
                    app.remove_team_member(t, app.agent_named(agent)?.id)?;
                    println!("{agent} is back in the pool");
                }
                TeamCmd::Assign {
                    team,
                    project: pname,
                    none: _,
                } => {
                    let t = team_named(&app, team)?;
                    let p = pname.as_deref().map(|n| project(&app, n)).transpose()?;
                    app.set_team_project(t, p.as_ref().map(|p| p.id))?;
                    println!(
                        "{team} now works on {}",
                        p.map(|p| p.name).unwrap_or_else(|| "no project".into())
                    );
                }
                TeamCmd::Dissolve { team } => {
                    let t = team_named(&app, team)?;
                    app.dissolve_team(t)?;
                    println!("{team} dissolved; its members are in the pool");
                }
                TeamCmd::Show { project: pname } => {
                    let p = project(&app, pname)?;
                    let (team, members) = app
                        .team_of_project(p.id)?
                        .ok_or_else(|| anyhow!("{} has no team", p.name))?;
                    println!(
                        "{} ({}{})",
                        team.name,
                        if team.solo { "solo, " } else { "" },
                        team.template
                    );
                    for a in members {
                        println!(
                            "  {}{}	{}	[{}]	{}	{}/{}",
                            a.name,
                            if team.lead == Some(a.id) {
                                " (Lead)"
                            } else if app.is_planner_role(&a.role) {
                                " (planner)"
                            } else {
                                ""
                            },
                            a.role,
                            a.tags.join(","),
                            a.status,
                            a.model.provider,
                            a.model.model
                        );
                    }
                }
            }
        }
        Cmd::Role(rc) => {
            let app = open(&root(&cli)?)?;
            match rc {
                RoleCmd::Proposals { project: pname } => {
                    let p = project(&app, pname)?;
                    for r in app.role_proposals(p.id)? {
                        println!(
                            "{}	{}	{:?}	{}: {}",
                            r.id, r.body.role_id, r.status, r.body.title, r.body.description
                        );
                    }
                }
                RoleCmd::List => {
                    for r in app.roles()? {
                        let flags = [
                            r.lead.then_some("lead"),
                            r.planner.then_some("planner"),
                            (!r.exists).then_some("preset"),
                        ]
                        .into_iter()
                        .flatten()
                        .collect::<Vec<_>>()
                        .join(",");
                        println!(
                            "{}	{}	[{}]	{} agent(s)	{}",
                            r.id,
                            if r.template.is_empty() {
                                "yours"
                            } else {
                                &r.template
                            },
                            r.tags.join(","),
                            r.agents,
                            flags
                        );
                    }
                }
                RoleCmd::Put {
                    id,
                    prompt_file,
                    tags,
                    allowed_paths,
                    planner,
                    lead,
                } => {
                    let prompt = std::fs::read_to_string(prompt_file)
                        .with_context(|| prompt_file.display().to_string())?;
                    let r = app.put_role(&agenium_orchestrator::registry::RoleDef {
                        id: id.clone(),
                        prompt,
                        tags: tags.clone(),
                        allowed_paths: allowed_paths.clone(),
                        planner: *planner,
                        lead: *lead,
                    })?;
                    println!("role {} saved", r.id);
                }
                RoleCmd::Remove { id } => {
                    app.remove_role(id)?;
                    println!("role {id} removed");
                }
            }
        }
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
