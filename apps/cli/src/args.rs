//! Command-line arguments (clap). Split out of `main.rs`.

use clap::{Parser, Subcommand};

use crate::*;

#[derive(Parser)]
#[command(
    name = "agenium",
    version,
    about = "Agenium: AI agents that work through tickets (headless CLI)"
)]
pub(crate) struct Cli {
    /// Agenium root (default: AGENIUM_HOME, then the root chosen at setup, then ~/.agenium).
    #[arg(long, global = true)]
    pub(crate) root: Option<PathBuf>,
    #[command(subcommand)]
    pub(crate) cmd: Cmd,
}

#[derive(Subcommand)]
pub(crate) enum Cmd {
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
pub(crate) enum ProviderCmd {
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
pub(crate) enum SecretCmd {
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
pub(crate) enum ProjectCmd {
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
    /// Close a done project: nothing runs on it any more and its team is set free to take another project (only
    /// you close a project; giving it a team again reopens it).
    Close {
        project: String,
    },
    /// Reopen a closed project: by default the team that worked on it takes it (now when free, else once it is
    /// free); `--team` gives it a free team now.
    Reopen {
        project: String,
        /// A free team to put on it now.
        #[arg(long)]
        team: Option<String>,
    },
    /// Project setting `review_before_merge` (Solo agents wait for you before merging).
    ReviewBeforeMerge {
        project: String,
        #[arg(value_parser = ["on", "off"])]
        value: String,
    },
}

#[derive(Subcommand)]
pub(crate) enum AgentCmd {
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
pub(crate) enum McpCmd {
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
pub(crate) enum SkillCmd {
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
pub(crate) enum TeamCmd {
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
pub(crate) enum RoleCmd {
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
pub(crate) enum TicketCmd {
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
