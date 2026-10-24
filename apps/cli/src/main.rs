//! `agenium`: headless front-end over the orchestrator (the desktop UI comes later). Every state change goes
//! through the same App/Runtime API the UI will use.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use agenium_config::{ApprovalPolicy, ProviderConfig, ProviderKind};
use agenium_core::forms::PermissionDecision;
use agenium_core::*;
use agenium_orchestrator::app::{App, AppOptions, CheckFactory, CreateAgent, CreateProject};
use agenium_orchestrator::maintenance::BackupRequest;
use agenium_orchestrator::runtime::{Approval, Approver, ConfiguredProviders, Runtime};
use agenium_store::core::{comments, events, projects::Project, runs};
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
        #[arg(long, default_value = "en")]
        ui_language: String,
        /// Language you talk to the Lead/Solo agent in (everything below the Lead is English).
        #[arg(long, default_value = "en")]
        user_language: String,
    },
    /// System requirements, sandbox, keyring and reconcile report.
    Doctor,
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
    /// Approve a requirement spec or plan version waiting for you.
    Approve {
        #[arg(long)]
        project: String,
        #[arg(value_parser = ["spec", "plan", "merge"])]
        what: String,
        /// Version (spec/plan) or ticket key (merge).
        target: String,
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
    /// Add or replace a provider. The key is read from --key-env or stdin.
    Add {
        id: String,
        #[arg(long, value_parser = ["anthropic", "gemini", "openai_compat"])]
        kind: String,
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
    /// List models from the provider's API.
    Models {
        id: String,
    },
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
    },
    List,
    /// Project setting `review_before_merge` (Solo agents wait for you before merging).
    ReviewBeforeMerge {
        project: String,
        #[arg(value_parser = ["on", "off"])]
        value: String,
    },
}

#[derive(Subcommand)]
enum AgentCmd {
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
        /// CLI program (the CLIs `doctor` lists as usable: codex, agy).
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
    /// Restrict the agent to these skills (no names = every skill of its role and its own).
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
    /// Import a skill from a folder, a .zip or an https URL of a .zip.
    Import {
        source: String,
        /// Install for one agent.
        #[arg(long, conflicts_with = "role", required_unless_present = "role")]
        agent: Option<String>,
        /// Install for every agent of a role.
        #[arg(long)]
        role: Option<String>,
        #[arg(long)]
        replace: bool,
    },
    /// Delete a skill folder.
    Remove {
        name: String,
        #[arg(long, conflicts_with = "role", required_unless_present = "role")]
        agent: Option<String>,
        #[arg(long)]
        role: Option<String>,
    },
    /// Check a skill folder without installing it.
    Validate { dir: PathBuf },
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
}

/// Interactive approval on the terminal (M1-7: no command runs without the user's approval).
struct TerminalApprover {
    lock: Mutex<()>,
}

fn ask(prompt: &str) -> String {
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
        let answer = {
            let _g = self.lock.lock().unwrap_or_else(|e| e.into_inner());
            tokio::task::block_in_place(|| ask(&prompt))
        };
        match answer.as_str() {
            "y" | "yes" => Approval::Allow,
            "a" | "always" => Approval::AllowAlways,
            _ => Approval::Deny("declined at the terminal".into()),
        }
    }
}

/// Pre-merge checks configured in `agenium.toml` run only after approval (no sandbox before M2).
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
        let ex = agenium_sandbox::Executor::new(agenium_sandbox::Shell::detect());
        let spec = agenium_sandbox::CommandSpec {
            command: self.command.clone(),
            cwd: worktree.into(),
            timeout: std::time::Duration::from_secs(1800),
            env: Default::default(),
        };
        let policy = agenium_sandbox::SandboxPolicy::workspace(std::path::Path::new(worktree));
        let r = match tokio::runtime::Handle::try_current() {
            Ok(h) => h.block_on(ex.run(&spec, &policy)),
            Err(_) => tokio::runtime::Runtime::new()
                .map_err(agenium_sandbox::ExecError::Io)
                .and_then(|rt| rt.block_on(ex.run(&spec, &policy))),
        };
        match r {
            Ok(o) => agenium_vcs::CheckOutcome {
                ok: o.exit_code == Some(0) && !o.timed_out,
                log: format!("{}\n{}", o.stdout, o.stderr),
            },
            Err(e) => agenium_vcs::CheckOutcome {
                ok: false,
                log: e.to_string(),
            },
        }
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
                    "{} not installed ({} MB): run `agenium models download`; until then memory uses keyword-level hashing",
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
            std::fs::create_dir_all(&dir)?;
            let client = reqwest::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(30))
                .build()?;
            for f in SPEC.files {
                let dst = dir.join(f.name);
                if matches!(local::sha256_file(&dst), Ok(h) if h == f.sha256) {
                    println!("{}: ok", f.name);
                    continue;
                }
                println!("{}: downloading {} KB ...", f.name, f.bytes >> 10);
                let part = dir.join(format!("{}.partial", f.name));
                let mut resp = client.get(SPEC.url(f)).send().await?.error_for_status()?;
                let mut out = std::fs::File::create(&part)?;
                let mut n = 0u64;
                while let Some(chunk) = resp.chunk().await? {
                    n += chunk.len() as u64;
                    if n > f.bytes {
                        bail!("{}: larger than expected", f.name);
                    }
                    out.write_all(&chunk)?;
                }
                out.sync_all()?;
                drop(out);
                let h = local::sha256_file(&part)?;
                if n != f.bytes || h != f.sha256 {
                    let _ = std::fs::remove_file(&part);
                    bail!("{}: checksum mismatch (got {h}, {n} bytes)", f.name);
                }
                std::fs::rename(&part, &dst)?;
                println!("{}: verified", f.name);
            }
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
        } => {
            app.put_mcp_server(agenium_config::McpServerConfig {
                id: id.clone(),
                command: command.clone(),
                args: args.clone(),
                env: pairs(env)?,
                url: url.clone(),
                headers: pairs(headers)?,
                timeout_secs: *timeout,
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
            for s in r.secrets {
                println!("moved a literal secret to the vault as {s}");
            }
        }
        McpCmd::List => {
            for m in app.config().mcp {
                let target = match (&m.command, &m.url) {
                    (Some(c), _) => format!("stdio: {c} {}", m.args.join(" ")),
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
                    tools: None,
                },
            };
            let hub = agenium_mcp::McpHub::new(
                agenium_orchestrator::mcp::secret_fn(app.clone()),
                app.layout().mcp_work(),
            );
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
        (None, None) => bail!("pass --agent or --role"),
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
                let origin = serde_json::to_value(&s.meta.origin)?;
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
                println!("(binding: {})", a.skills.join(", "));
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
        SkillCmd::Import {
            source,
            agent,
            role,
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
        SkillCmd::Remove { name, agent, role } => {
            let owner = skill_owner(&app, agent, role)?;
            reg.remove(&owner, name)?;
            println!("removed {name}");
        }
        SkillCmd::Validate { .. } => unreachable!(),
    }
    Ok(())
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
        Arc::new(TerminalApprover {
            lock: Mutex::new(()),
        }),
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
        Cmd::Setup {
            ui_language,
            user_language,
        } => {
            let default_root = dirs_home().join(".agenium");
            let input = agenium_config::setup::SetupInput {
                root: cli.root.clone(),
                ui_language: ui_language.clone(),
                user_language: user_language.clone(),
            };
            let out = agenium_config::setup::complete_setup(
                &input,
                &default_root,
                agenium_config::Bootstrap::default_path().as_deref(),
            )?;
            println!("Agenium root: {}", out.root.display());
            for c in out.report.warnings() {
                println!("  warning: {} = {} ({})", c.name, c.value, c.message);
            }
            println!(
                "Suggested parallel agents: {}",
                out.report.suggested_parallel
            );
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
                    if cfg.sandbox.approval == ApprovalPolicy::Never && sb.backend.is_none() {
                        " (YOLO is refused: no sandbox backend, fail-closed)"
                    } else {
                        ""
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
        Cmd::Provider(pc) => {
            let app = open(&root(&cli)?)?;
            match pc {
                ProviderCmd::Add {
                    id,
                    kind,
                    base_url,
                    key_env,
                    key_stdin,
                    max_concurrency,
                } => {
                    let key = match (key_env, key_stdin) {
                        (Some(v), _) => Some(
                            std::env::var(v)
                                .with_context(|| format!("environment variable {v} is not set"))?,
                        ),
                        (None, true) => Some(read_secret_stdin()?),
                        (None, false) => None,
                    };
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
                    for m in p.list_models().await.map_err(|e| anyhow!("{e}"))? {
                        println!(
                            "{}\t{}",
                            m.id,
                            m.context_window
                                .map(|c| format!("{c} tokens"))
                                .unwrap_or_default()
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
                } => {
                    let p = app.create_project(&CreateProject {
                        name: name.clone(),
                        path: path.clone(),
                        key_prefix: prefix.clone(),
                        template: template.clone(),
                    })?;
                    println!("project {} at {}", p.name, p.path);
                }
                ProjectCmd::List => {
                    for p in app.projects()? {
                        println!("{}\t{}\t{}\t{}", p.name, p.status, p.key_prefix, p.path);
                    }
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
                        println!("{name} uses every skill of role {} and its own", a.role);
                    } else {
                        println!("{name} is restricted to: {}", skills.join(", "));
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
            let needs = app
                .store()
                .read(|c| events::recent_of_kind(c, "NeedsUser", 5))?;
            for e in needs {
                println!(
                    "needs you: {}",
                    e.payload["reason"].as_str().unwrap_or_default()
                );
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
            let out = app.user_answer(
                p.id,
                &ticket.parse().map_err(|e: IdParseError| anyhow!(e))?,
                question.parse().map_err(|e: IdParseError| anyhow!(e))?,
                answer,
                *option,
            )?;
            println!("{}", out.result);
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
                _ => {
                    app.user_approve_merge(
                        p.id,
                        &target.parse().map_err(|e: IdParseError| anyhow!(e))?,
                    )?;
                }
            }
            println!("approved {what} {target}");
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
