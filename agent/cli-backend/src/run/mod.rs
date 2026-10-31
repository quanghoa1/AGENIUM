//! One CliBackend run (ADR-56): the vendor CLI runs headless in its own YOLO mode with an Agenium-owned per-run home
//! and runs its own agent loop. Agenium keeps the four controls of decision 3c:
//! (1) protocol tools (and the agent's own MCP bindings) only through Agenium's MCP server for this run;
//! (2) permissions are Agenium's: either the CLI process runs inside Agenium's sandbox (`Control::Sandbox`), or the
//!     CLI's pre-tool hook asks Agenium before every tool call (`Control::Hook`);
//! (3) step / loop / time limits counted from the event stream (the process is stopped at a limit);
//! (4) a work run only ends through a protocol tool (`submit_report`, `ask_question`, ...); a turn that ends
//!     without one gets a nudge, then the run stops.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use agenium_backend::{Cancel, Effect, Observer, Outcome, Protocol, RunReport, WorkOrder};
use agenium_llm::{ChatEvent, Message, Part, Role, ToolSpec, Usage};
use agenium_mcp::server::{Server, ToolHost};
use agenium_mcp::{CallResult, ToolInfo};
use agenium_sandbox::service::{spawn_contained, Contained};
use agenium_sandbox::TreeGuard;
use agenium_tools::{Tool, ToolCtx, ToolError};
use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot};

use crate::driver::{Check, CheckCtx, Control, Driver, Event, Setup, Turns};

mod process;
mod prompt;
mod relogin;
mod session;
mod turns;

use bridge::{Bridge, Call};
pub(crate) use process::keep_tail;
use process::Proc;
use prompt::steered_mark;
pub use prompt::{isolated_home, render};
pub use session::SessionStore;
mod bridge;

/// Name of Agenium's MCP server as the CLI sees it.
pub const MCP_SERVER: &str = "agenium";
/// After a protocol tool ended the run, how long the CLI may take to finish its turn before it is stopped.
const END_GRACE: Duration = Duration::from_secs(30);
/// A per-turn process gets this long to exit after its turn ended.
const EXIT_GRACE: Duration = Duration::from_secs(15);
const STDERR_TAIL: usize = 16 * 1024;
/// The per-run homes in the temp folder.
const HOME_PREFIX: &str = "agenium-cli-home-";
/// How often a run's copy of its CLI's login and the user's file are made the same.
const LOGIN_SYNC: Duration = Duration::from_secs(5);
/// A per-run home this old belongs to no run (a run lasts 45 minutes at most): one Agenium did not remove because it
/// stopped with the run (a crash, a killed process).
const HOME_STALE: Duration = Duration::from_secs(24 * 3600);

/// Removes the per-run homes left behind: each holds a copy of a CLI login (OAuth refresh tokens) in clear.
fn sweep_old_homes(temp: &std::path::Path) {
    let Ok(rd) = std::fs::read_dir(temp) else {
        return;
    };
    for e in rd.flatten() {
        let age = e
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok());
        if age.is_some_and(|a| stale_home(&e.file_name().to_string_lossy(), a)) {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}

fn stale_home(name: &str, age: Duration) -> bool {
    name.starts_with(HOME_PREFIX) && age > HOME_STALE
}
/// The deny a hook gets when no decision came in time.
const LATE: &str =
    "no decision came in time (the approval was not given); ask again if the step is still needed";

const NUDGE: &str = "Text alone does not finish the ticket. Continue with your tools; when you are done call \
submit_report (MCP server \"agenium\") with one result per acceptance criterion, or ask_question if you are blocked.";

#[derive(Debug, Clone)]
pub struct Limits {
    /// Tool steps of the CLI.
    pub max_steps: u32,
    /// The same tool step (name + input) this many times in a row stops the run.
    pub same_call_limit: u32,
    /// Turns ended without a protocol tool tolerated (each gets a nudge) in a work run.
    pub nudge_limit: u32,
    /// Turns ended by refused actions tolerated (each gets the list of refusals).
    pub refusal_limit: u32,
    /// Wall-clock limit of the whole run.
    pub timeout: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_steps: 200,
            same_call_limit: 3,
            nudge_limit: 2,
            refusal_limit: 3,
            timeout: Duration::from_secs(45 * 60),
        }
    }
}

pub struct CliRun<'a> {
    pub driver: &'a dyn Driver,
    pub program: PathBuf,
    /// The CLI's model (empty = its default).
    pub model: String,
    /// The agent's reasoning effort (`None`: the CLI's own setting).
    pub effort: Option<agenium_core::Effort>,
    /// Added to the clean environment (the driver's home variables win).
    pub env: BTreeMap<String, String>,
    /// The ticket worktree.
    pub cwd: PathBuf,
    /// Shown as the run's model (`codex:gpt-6-luna`).
    pub label: String,
    pub protocol: &'a dyn Protocol,
    /// Protocol tools offered through the MCP server.
    pub protocol_specs: Vec<ToolSpec>,
    /// The Agenium tools the CLI works through over MCP (ADR-129): files, search, shell, … and the agent's own MCP
    /// bindings (their output marked untrusted).
    pub tools: Vec<Arc<dyn Tool>>,
    /// The run's tool context; its sandbox policy is the base of the CLI's (`Control::Sandbox`).
    pub ctx: &'a ToolCtx,
    /// Where the ticket's CLI session is kept between runs (a work run's; ADR-134).
    pub session: Option<SessionStore>,
    pub observer: &'a dyn Observer,
    pub cancel: Cancel,
    pub limits: Limits,
    /// Chat run: a turn that ends without a protocol tool completes the run.
    pub chat: bool,
    /// Nudge for a turn that ends without a protocol tool outside chat mode (`None`: the work-run nudge).
    pub nudge: Option<String>,
    /// Solo agents may start the CLI's own subagents; team members never do.
    pub subagents: bool,
    /// Only a work run writes files; chat, review, inbox and plan runs read ([`CheckCtx::read_only`]).
    pub read_only: bool,
    /// A `Control::Sandbox` CLI runs on the host: no sandbox backend works and the user accepted running without one
    /// (`[sandbox] unsandboxed`, ADR-117).
    pub uncontained: bool,
    /// Receives the vendor's subscription usage when the CLI reports it.
    pub on_limits: Option<Arc<dyn Fn(agenium_llm::vendor_usage::VendorUsage) + Send + Sync>>,
    /// The user's messages to this running Lead (ADR-104/110): handed over with the next result of an Agenium tool,
    /// or as the next turn when the CLI's turn ends first. `None` for workers.
    pub steer: Option<agenium_backend::Steer>,
}

/// Why Agenium stopped the CLI.
#[derive(Debug, Clone)]
enum Stop {
    Steps,
    Timeout,
    Loop(String),
    Cancelled,
}

#[derive(Default)]
struct State {
    steps: u32,
    usage: Usage,
    messages: Vec<Message>,
    text: String,
    recent: Vec<String>,
    /// Tool steps by id (results of some CLIs only carry the id).
    started: std::collections::HashMap<String, (String, Value)>,
    session: Option<String>,
    /// The run goes on in the CLI's session of an earlier run (ADR-134).
    resumed: bool,
    effect: Option<Effect>,
    ended_at: Option<Instant>,
    stop: Option<Stop>,
    /// This run's mark on the user's relayed messages (named in the instructions; a file or tool output cannot
    /// guess it, so text that merely claims to be from the user is not taken for a message).
    tag: String,
    /// ADR-129, the log shows a subagent's steps as its own. The subagent whose events are being handled (the id of
    /// the step that started it).
    parent: Option<String>,
    /// Subagents started in this run: the id of the step that started each, and its label (`subagent 1 · what`).
    subagents: std::collections::HashMap<String, String>,
    /// The subagent label of a tool step, by step id.
    labels: std::collections::HashMap<String, String>,
    /// Steps already logged when they started (a subagent's start, so its own steps follow it).
    logged: std::collections::HashSet<String>,
    /// Agenium tool calls a subagent announced in the stream, waiting for their MCP request: (tool, input, label).
    subagent_calls: Vec<(String, Value, String)>,
    /// The CLI's own tool calls the hook refused, logged then (their end in the stream is not logged again): the
    /// [`tool_key`] and the input, as a refused call may never end in the stream and must not be taken for a later
    /// call of the same tool.
    denied: Vec<(String, Value)>,
    /// The CLI's own tool calls its hook let through, by [`tool_key`]: a hooked CLI's step that ends without one did
    /// not run (its hook never reached Agenium, so the CLI blocked it).
    allowed: Vec<String>,
}

/// A tool's name as the hook and the stream both spell it (they differ in case and separators).
fn tool_key(name: &str) -> String {
    name.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// A tool call's arguments as the run's log shows them: with the subagent that made it, if one did (ADR-129).
fn labelled(args: &Value, label: Option<&str>) -> Value {
    match (label, args) {
        (None, a) => a.clone(),
        (Some(l), Value::Object(m)) => {
            let mut m = m.clone();
            m.insert("_subagent".into(), Value::String(l.to_owned()));
            Value::Object(m)
        }
        (Some(l), a) => serde_json::json!({"input": a, "_subagent": l}),
    }
}

/// How much of a tool result the run's conversation keeps (the memory transcript shows 600 characters of it).
const RECORDED_RESULT_CHARS: usize = 4000;

/// Whether a CLI's turn ended because its own subscription or free allowance is used up (OpenCode: "Free usage limit
/// reached"; Codex and Claude Code name the quota, the usage limit or the window that ran out: "5-hour limit
/// reached", "You've hit your limit"), rather than on an error a retry could pass (a rate limit is one).
fn out_of_quota(error: &str) -> bool {
    let e = error.to_ascii_lowercase();
    [
        "usage limit",
        "quota",
        "out of credits",
        "insufficient credit",
        "credit balance",
        "billing",
        "hour limit",
        "weekly limit",
        "hit your limit",
    ]
    .iter()
    .any(|k| e.contains(k))
}

/// Whether a CLI's turn ended because its login is gone (expired, revoked, never made): no retry passes either.
fn logged_out(error: &str) -> bool {
    let e = error.to_ascii_lowercase();
    [
        "not logged in",
        "please log in",
        "please run /login",
        "login required",
        "log in again",
        "invalid_grant",
        "authentication_error",
        "invalid api key",
        "oauth token has expired",
        "token has expired",
        "unauthorized",
        "401",
    ]
    .iter()
    .any(|k| e.contains(k))
}

/// The outcome of a turn that ended on an error: a used-up subscription waits for the user (the agent is set aside and
/// they are told); a lost login is a failure that says to log in again (it said nothing). A login another run refreshed
/// a moment before was already taken in and the turn run again (`relogin.rs`): this one is really gone, and the runs
/// stop after two and ask the user.
fn turn_error(e: String) -> Outcome {
    if out_of_quota(&e) {
        Outcome::QuotaExhausted { message: e }
    } else if logged_out(&e) {
        failed(format!(
            "the CLI's login has expired or was revoked: log in to it again ({e})"
        ))
    } else {
        failed(e)
    }
}

fn failed(e: impl std::fmt::Display) -> Outcome {
    Outcome::Failed {
        message: e.to_string(),
    }
}

impl CliRun<'_> {
    fn protocol_names(&self) -> Vec<String> {
        self.protocol_specs.iter().map(|s| s.name.clone()).collect()
    }

    pub async fn run(&self, order: &WorkOrder) -> RunReport {
        // A command Agenium's tools run for the CLI ends at the stop, not when it is done (the stop is otherwise seen
        // between the CLI's events, which wait for the command).
        let _stop = self.cancel.forward(&self.ctx.stop);
        let mut st = State {
            tag: ulid::Ulid::new().to_string()[20..].to_lowercase(),
            ..State::default()
        };
        let outcome = match self.drive(order, &mut st).await {
            Ok(o) | Err(o) => o,
        };
        RunReport {
            outcome,
            steps: st.steps,
            usage: st.usage,
            messages: st.messages,
            model: self.label.clone(),
        }
    }

    async fn drive(&self, order: &WorkOrder, st: &mut State) -> Result<Outcome, Outcome> {
        let control = self.driver.control();
        if control == Control::Sandbox
            && !self.uncontained
            && agenium_sandbox::detect().backend.is_none()
        {
            return Err(failed(
                "this CLI runs only inside Agenium's sandbox and no sandbox backend works on this machine",
            ));
        }
        let (tx, mut calls) = mpsc::channel::<Call>(8);
        let tools = self
            .protocol_specs
            .iter()
            .map(|s| ToolInfo {
                name: s.name.clone(),
                description: s.description.clone(),
                input_schema: s.parameters.clone(),
                read_only: false,
                destructive: None,
            })
            .chain(self.tools.iter().map(|t| ToolInfo {
                name: t.name().to_owned(),
                description: t.description().to_owned(),
                input_schema: t.parameters(),
                read_only: t.read_only(),
                destructive: None,
            }))
            .collect();
        let bridge = Bridge {
            tools,
            tx,
            hook_deadline: crate::driver::hook_deadline(self.driver),
            ended: self.driver.hook_answer(false, "the Agenium run has ended"),
            late: self.driver.hook_answer(false, LATE),
        };
        let server = Server::start(Arc::new(bridge), MCP_SERVER)
            .await
            .map_err(|e| failed(format!("cannot start the Agenium MCP server: {e}")))?;
        let base = self
            .driver
            .home_base()
            .filter(|b| std::fs::create_dir_all(b).is_ok())
            .unwrap_or_else(std::env::temp_dir);
        sweep_old_homes(&base);
        let mut home = tempfile::Builder::new()
            .prefix(HOME_PREFIX)
            .tempdir_in(&base)
            .map_err(|e| failed(format!("cannot create the CLI home: {e}")))?;
        // Diagnostics: `AGENIUM_KEEP_CLI_HOME=1` keeps the per-run home (the CLI's own logs) after the run.
        if std::env::var_os("AGENIUM_KEEP_CLI_HOME").is_some() {
            home.disable_cleanup(true);
            tracing::warn!(home = %home.path().display(), "per-run CLI home kept");
        }
        let (url, hook_url, authorization) =
            (server.url(), server.hook_url(), server.authorization());
        let setup = Setup {
            home: home.path(),
            worktree: &self.cwd,
            mcp_name: MCP_SERVER,
            mcp_url: &url,
            mcp_authorization: &authorization,
            hook_url: &hook_url,
            subagents: self.subagents,
            model: &self.model,
            run_for: self.limits.timeout,
        };
        // A refresh token this run comes to hold is free for another run once it ends (ADR-137).
        let _released = crate::login_access::Released(home.path().to_path_buf());
        // The ticket's CLI session of an earlier run comes back first; the run's own files are written over it.
        let cli = self.label.split(':').next().unwrap_or_default().to_owned();
        if let Some(id) = self
            .session
            .as_ref()
            .and_then(|s| s.restore(home.path(), self.driver, &cli))
        {
            self.observer.control(
                "resume",
                &format!("the CLI's session of the ticket's earlier run goes on ({id})"),
            );
            st.session = Some(id);
            st.resumed = true;
        }
        let home_env = self
            .driver
            .prepare(&setup)
            .map_err(|e| failed(format!("cannot prepare the CLI's per-run home: {e}")))?;
        let mut env = self.env.clone();
        env.extend(isolated_home(home.path(), home_env));

        // The run's login and the user's stay the same while it goes on (`Driver::finish`, every few seconds): a
        // login refreshed in one run reaches the others before they need it (ADR-56).
        let sync = async {
            loop {
                tokio::time::sleep(LOGIN_SYNC).await;
                if let Err(e) = self.driver.finish(&setup) {
                    tracing::warn!(error = %e, "CLI login sync failed");
                }
            }
        };
        let resync = || {
            if self.driver.promote_login(&setup) {
                self.observer.control(
                    "login",
                    "the access token ran out and no other run refreshes it: this run takes the refresh token",
                );
            }
            if let Err(e) = self.driver.finish(&setup) {
                tracing::warn!(error = %e, "CLI login sync failed");
            }
        };
        let outcome = tokio::select! {
            o = self.converse(order, st, &env, home.path(), &mut calls, &resync) => o,
            never = sync => never,
        };
        if let Err(e) = self.driver.finish(&setup) {
            tracing::warn!(error = %e, "CLI run cleanup failed");
        }
        if let Some(id) = st.session.as_deref() {
            for dir in self.driver.session_leftovers(&self.cwd, id) {
                let _ = std::fs::remove_dir(dir);
            }
        }
        // Kept for the ticket's next run, which resumes the session (ADR-134).
        if let Some(s) = &self.session {
            let total = order.messages.len() + st.messages.len();
            s.keep(home.path(), self.driver, &cli, st.session.as_deref(), total);
        }
        drop(server);
        Ok(outcome)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_used_up_subscription_is_a_quota_not_a_failure() {
        for e in [
            "opencode turn failed: Free usage limit reached. Connect a provider or subscribe to OpenCode Go",
            "codex turn failed: You've hit your usage limit",
            "insufficient_quota: out of credits",
        ] {
            assert!(out_of_quota(e), "{e}");
        }
        assert!(!out_of_quota("codex turn failed: stream disconnected"));
        // The windows of a subscription, not a rate limit a retry passes.
        assert!(out_of_quota("claude: 5-hour limit reached ∙ resets 3pm"));
        assert!(out_of_quota(
            "You've hit your limit · resets 11pm (Asia/Saigon)"
        ));
        assert!(!out_of_quota(
            "Rate limit reached for requests, retry after 2s"
        ));
    }

    #[test]
    fn only_a_day_old_per_run_home_is_swept() {
        let day = Duration::from_secs(24 * 3600);
        assert!(stale_home(
            "agenium-cli-home-Ab12",
            day + Duration::from_secs(1)
        ));
        assert!(!stale_home(
            "agenium-cli-home-Ab12",
            Duration::from_secs(45 * 60)
        ));
        assert!(!stale_home("someone-elses-dir", day * 2));
    }

    #[test]
    fn a_lost_login_says_to_log_in_again() {
        for e in [
            "the CLI exited before the turn ended (Invalid API key · Please run /login)",
            "codex turn failed: 401 Unauthorized: token has expired",
            "agy: not logged in",
        ] {
            match turn_error(e.into()) {
                Outcome::Failed { message } => {
                    assert!(message.contains("log in to it again"), "{message}")
                }
                o => panic!("{e}: {o:?}"),
            }
        }
        assert!(matches!(
            turn_error("You've hit your usage limit".into()),
            Outcome::QuotaExhausted { .. }
        ));
        assert!(matches!(
            turn_error("the CLI exited before the turn ended (segfault)".into()),
            Outcome::Failed { .. }
        ));
    }

    #[test]
    fn every_cli_is_told_to_work_through_agenium_tools() {
        let order = WorkOrder {
            system: vec![],
            messages: vec![],
        };
        let work: Vec<String> = [
            "read_file",
            "edit_file",
            "shell",
            "shell_output",
            "shell_stop",
            "mcp__docs__search",
        ]
        .map(String::from)
        .to_vec();
        let text = render(&order, &["submit_report".to_owned()], &work, false);
        assert!(text.contains("not your own"), "{text}");
        assert!(text.contains("`shell` for every command"));
        assert!(text.contains("`background`") && text.contains("`shell_output`"));
        assert!(text.contains("Your own shell is turned off"));
        assert!(text.contains("submit_report"));
        assert!(text.contains("`mcp__docs__search`"));
        assert!(!text.contains("subagent"));
        // Only the tools this run offers are named: a chat run (read tools only) is told it changes nothing.
        assert!(
            !text.contains("write_file") && !text.contains("changes no files"),
            "{text}"
        );
        let chat = render(
            &order,
            &["create_ticket".to_owned()],
            &["read_file".to_owned(), "grep".to_owned()],
            false,
        );
        assert!(
            chat.contains("`read_file` and `grep` to read") && chat.contains("changes no files"),
            "{chat}"
        );
        assert!(
            !chat.contains("edit_file") && !chat.contains("`shell`"),
            "{chat}"
        );
        // A Solo agent is told its own subagent tool stays on (Codex took "not your own" for it, 2026-10-28).
        let solo = render(&order, &["submit_report".to_owned()], &work, true);
        assert!(solo.contains("you may start your own subagents"), "{solo}");
    }
}
