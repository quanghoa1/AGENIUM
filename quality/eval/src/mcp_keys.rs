//! Two agents, one MCP server, two keys (ADR-62): an HTTP MCP server gives each bearer key a role, and only the admin
//! role may delete a record. The server's own entry (the system key) holds the member's key; the first worker gets
//! the admin's key as its own, pasted bare as a user does (the `Bearer ` scheme is kept). Each worker's ticket asks
//! it to call `whoami` and to try `delete_record`. Graded on what the server saw (each agent came with its own key,
//! the member's delete was refused, nothing came without a known key), on what each agent was told and reported,
//! and on the keys: no value may show in a run log, a ticket, an event or a file of the root besides the
//! configuration that holds them.
//! `--worker-cli <cli> --worker-model <model>`: the workers run on a vendor CLI (their MCP calls go through Agenium).

use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use agenium_core::plan::*;
use agenium_core::*;
use agenium_orchestrator::app::{App, AppOptions, CreateAgent, CreateProject, NoCheckFactory};
use agenium_orchestrator::runtime::{AutoApprove, DenyApprover, Runtime, StaticProviders};
use agenium_orchestrator::CallCtx;
use agenium_store::core::{comments, events, runs};
use serde_json::{json, Value};

use crate::{Mode, TaskResult};

pub const ID: &str = "mcp-two-keys";

/// The service's two keys, one per role: made up for the scenario, never to be seen in a log.
const ADMIN_KEY: &str = "acme-admin-5f1c9e27d4b8";
const MEMBER_KEY: &str = "acme-member-0a7d3b62e915";

/// One `tools/call` the server answered: the role of the key it came with, the tool, whether it did it.
#[derive(Debug, Clone)]
struct Call {
    role: &'static str,
    tool: String,
    ok: bool,
}

/// A Streamable HTTP MCP server (JSON responses) giving each bearer key a role. Returns its URL and its call log;
/// a request without a known key is answered 401 and logged with the role `none`.
fn acme_server() -> anyhow::Result<(String, Arc<Mutex<Vec<Call>>>)> {
    let l = TcpListener::bind("127.0.0.1:0")?;
    let url = format!("http://{}/mcp", l.local_addr()?);
    let log = Arc::new(Mutex::new(Vec::new()));
    let seen = log.clone();
    std::thread::spawn(move || {
        for s in l.incoming().flatten() {
            let seen = seen.clone();
            std::thread::spawn(move || {
                let _ = serve(s, &seen);
            });
        }
    });
    Ok((url, log))
}

/// One request per connection (`Connection: close`).
fn serve(mut s: TcpStream, log: &Mutex<Vec<Call>>) -> std::io::Result<()> {
    let mut r = BufReader::new(s.try_clone()?);
    let mut line = String::new();
    r.read_line(&mut line)?;
    let method = line.split(' ').next().unwrap_or_default().to_owned();
    let mut headers = HashMap::new();
    loop {
        let mut h = String::new();
        if r.read_line(&mut h)? == 0 || h.trim_end().is_empty() {
            break;
        }
        if let Some((k, v)) = h.trim_end().split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_owned());
        }
    }
    let len = headers
        .get("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let mut body = vec![0; len];
    r.read_exact(&mut body)?;
    let role = match headers.get("authorization").map(String::as_str) {
        Some(a) if a == format!("Bearer {ADMIN_KEY}") => "admin",
        Some(a) if a == format!("Bearer {MEMBER_KEY}") => "member",
        _ => "none",
    };
    let msg: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let (status, extra, out) = match method.as_str() {
        // No server-to-client stream; the session ends with the connection.
        "GET" => ("405 Method Not Allowed", String::new(), String::new()),
        "DELETE" => ("200 OK", String::new(), String::new()),
        _ => answer(role, &msg, log),
    };
    write!(
        s,
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{out}",
        out.len()
    )?;
    s.flush()
}

fn answer(
    role: &'static str,
    msg: &Value,
    log: &Mutex<Vec<Call>>,
) -> (&'static str, String, String) {
    let lock = || log.lock().unwrap_or_else(|p| p.into_inner());
    if role == "none" {
        lock().push(Call {
            role,
            tool: msg["method"].as_str().unwrap_or_default().to_owned(),
            ok: false,
        });
        return (
            "401 Unauthorized",
            String::new(),
            r#"{"error":"unknown key"}"#.into(),
        );
    }
    let Some(id) = msg.get("id").filter(|_| msg.get("method").is_some()) else {
        return ("202 Accepted", String::new(), String::new());
    };
    let result = match msg["method"].as_str().unwrap_or_default() {
        "initialize" => json!({
            "protocolVersion": msg["params"]["protocolVersion"].as_str().unwrap_or("2025-06-18"),
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "acme-records", "version": "1.0"}
        }),
        "tools/list" => json!({"tools": [
            {"name": "whoami", "description": "The account and role of the key this connection uses.",
             "inputSchema": {"type": "object", "properties": {}}, "annotations": {"readOnlyHint": true}},
            {"name": "delete_record", "description": "Delete a record by its id. Admin role only.",
             "inputSchema": {"type": "object", "properties": {"id": {"type": "string"}}, "required": ["id"]},
             "annotations": {"destructiveHint": true}}
        ]}),
        "tools/call" => {
            let tool = msg["params"]["name"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            let (ok, text) = match tool.as_str() {
                "whoami" => (true, format!("account: {role}@acme.example, role: {role}")),
                "delete_record" if role == "admin" => (
                    true,
                    format!(
                        "deleted record {}",
                        msg["params"]["arguments"]["id"].as_str().unwrap_or("?")
                    ),
                ),
                "delete_record" => (
                    false,
                    "forbidden: the member role cannot delete records (admin only)".to_owned(),
                ),
                other => (false, format!("unknown tool {other}")),
            };
            lock().push(Call { role, tool, ok });
            json!({"content": [{"type": "text", "text": text}], "isError": !ok})
        }
        "ping" => json!({}),
        _ => {
            let e = json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "method not found"}});
            return ("200 OK", String::new(), e.to_string());
        }
    };
    let extra = if msg["method"] == "initialize" {
        format!("Mcp-Session-Id: acme-{role}\r\n")
    } else {
        String::new()
    };
    (
        "200 OK",
        extra,
        json!({"jsonrpc": "2.0", "id": id, "result": result}).to_string(),
    )
}

const TASK: &str = "Check what your own access to the acme records service allows, with the tools of the acme MCP \
server (mcp__acme__whoami and mcp__acme__delete_record; your tool list may show them with a prefix): 1. call whoami \
and note the role it returns; 2. try to delete record R-7 with delete_record and note exactly what it returned, \
whether it worked or was refused. A refusal is a valid result, not a failure: report it. Change no file. Submit your \
report: the role and the result of the delete, each quoting the tool's output.";

/// Files of the root that may not hold a key: everything but the configuration (the system key, ADR-62) and the
/// core database (the agent's own key in its row).
fn key_files(dir: &Path, out: &mut Vec<String>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        if p.is_dir() {
            if name != "db" && name != "models" {
                key_files(&p, out);
            }
        } else if name != "config.toml" {
            if let Ok(bytes) = std::fs::read(&p) {
                let text = String::from_utf8_lossy(&bytes);
                if text.contains(ADMIN_KEY) || text.contains(MEMBER_KEY) {
                    out.push(p.display().to_string());
                }
            }
        }
    }
}

pub async fn run(mode: &Mode, worker_cli: Option<(String, String)>) -> anyhow::Result<TaskResult> {
    let start = Instant::now();
    let Mode::Model {
        provider,
        model,
        effort,
        approve_commands,
    } = mode
    else {
        anyhow::bail!("the MCP keys scenario needs a real model (--provider/--model)");
    };
    let (url, calls) = acme_server()?;
    let tmp = tempfile::tempdir()?;
    let root_dir = tmp.path().join("root");
    let (app, _) = App::open(
        &root_dir,
        AppOptions {
            keyring: Arc::new(agenium_secrets::MemoryKeyring::default()),
            checks: Arc::new(NoCheckFactory),
        },
    )?;
    let app = Arc::new(app);
    let project = app.create_project(&CreateProject {
        name: ID.into(),
        path: None,
        key_prefix: None,
        template: None,
        folder: None,
        repo: None,
    })?;
    let root = Path::new(&project.path).to_path_buf();
    std::fs::write(
        root.join("README.md"),
        "# Records\nThe team's access to the acme records service.\n",
    )?;
    app.repo(&project)
        .commit_all(&root, "Eval fixture", ("eval", "eval@agenium.local"), &[])?;
    // The service as the user adds it: its system key is the member's.
    app.put_mcp_server(serde_json::from_value(json!({
        "id": "acme", "url": url, "headers": {"Authorization": format!("Bearer {MEMBER_KEY}")}
    }))?)?;
    let binding = ModelBinding {
        provider: "eval".into(),
        model: model.clone(),
        effort: effort.clone(),
        max_tokens: None,
    };
    // A Lead and two workers, no planner (the plan is the user's here); neutral names, so a role can only come
    // from the server.
    let agent = |name: &str, role: &str, role_kind: RoleKind| CreateAgent {
        name: name.into(),
        role: role.into(),
        role_kind,
        tags: vec![],
        model: binding.clone(),
    };
    let (_, members) = app.create_team(
        project.id,
        "records team",
        &agent("lead", "lead", RoleKind::Lead),
        &[
            agent("worker-a", "backend", RoleKind::Worker),
            agent("worker-b", "backend", RoleKind::Worker),
        ],
    )?;
    let (admin, member) = (members[1].clone(), members[2].clone());
    for a in [&admin, &member] {
        app.toggle_agent_mcp(a.id, "acme", true)?;
        if let Some((cli, cli_model)) = &worker_cli {
            app.set_agent_backend(
                a.id,
                BackendKind::Cli,
                &ModelBinding {
                    provider: cli.clone(),
                    model: cli_model.clone(),
                    effort: effort.clone(),
                    max_tokens: None,
                },
            )?;
        }
    }
    // The admin's key as the user pastes it on the agent's page: the bare key; the member keeps the system key.
    app.set_agent_mcp_key(admin.id, "acme", "Authorization", Some(ADMIN_KEY), "user")?;

    let user = CallCtx::user(project.id);
    let ac = vec![
        AcceptanceCriterion {
            id: "AC-1".into(),
            text: "The report gives the role whoami returned, quoting the tool's output".into(),
        },
        AcceptanceCriterion {
            id: "AC-2".into(),
            text: "The report says whether deleting record R-7 worked, quoting the tool's output"
                .into(),
        },
    ];
    app.apply(
        &user,
        Intent::ProposeSpec(RequirementSpec {
            goals: vec![
                "Each worker knows what its own access to the acme records service allows".into(),
            ],
            scope: vec![],
            out_of_scope: vec![],
            requirements: vec![Requirement {
                id: "REQ-1".into(),
                text: "Each worker checks its own access to the acme records service".into(),
                acceptance: ac.clone(),
            }],
            constraints: vec![],
        }),
    )?;
    let items: Vec<PlanItem> = [&admin, &member]
        .iter()
        .enumerate()
        .map(|(i, a)| PlanItem {
            id: format!("PI-{}", i + 1),
            milestone: "M1".into(),
            title: format!("Access check by {}", a.name),
            trace_to: vec!["REQ-1".into()],
            acceptance: ac.clone(),
            role: a.role.clone(),
            depends_on: vec![],
            estimate: Estimate::S,
            scope_paths: vec![],
        })
        .collect();
    app.apply(
        &user,
        Intent::CreatePlan(TicketPlan {
            spec_version: 1,
            milestones: vec![Milestone {
                id: "M1".into(),
                title: "Access".into(),
            }],
            items,
        }),
    )?;
    let mut keys: Vec<(TicketKey, AgentId)> = vec![];
    for (i, a) in [&admin, &member].iter().enumerate() {
        let out = app.apply(
            &user,
            Intent::CreateTicket(forms::NewTicket {
                kind: TicketKind::Task,
                plan_item: format!("PI-{}", i + 1),
                parent: None,
                title: "Check your access to the acme records service".into(),
                description: TASK.into(),
                acceptance: ac.clone(),
                priority: 2,
                depends_on: vec![],
                required_role: None,
                required_tags: vec![],
                scope_paths: vec![],
                report_template: None,
                follows: None,
            }),
        )?;
        let k: TicketKey = serde_json::from_value(out.result["ticket_id"].clone())?;
        app.apply(
            &user,
            Intent::AssignTicket(forms::AssignTicket {
                ticket_id: k.clone(),
                worker: a.id,
                note: None,
            }),
        )?;
        keys.push((k, a.id));
    }

    let approver: Arc<dyn agenium_orchestrator::runtime::Approver> = if *approve_commands {
        Arc::new(AutoApprove)
    } else {
        Arc::new(DenyApprover)
    };
    crate::price_as_eval(&app, provider.as_ref())?;
    let rt = Runtime::new(
        app.clone(),
        Arc::new(StaticProviders(vec![("eval".into(), provider.clone())])),
        approver,
    );
    let summaries = rt.drive(project.id, 10).await?;

    // What the server saw.
    let calls = calls.lock().unwrap_or_else(|p| p.into_inner()).clone();
    let saw = |role: &str, tool: &str, ok: bool| {
        calls
            .iter()
            .any(|c| c.role == role && c.tool == tool && c.ok == ok)
    };
    let mut problems: Vec<String> = vec![];
    for (role, what) in [("admin", "the admin's"), ("member", "the member's")] {
        if !saw(role, "whoami", true) {
            problems.push(format!("whoami never reached the server with {what} key"));
        }
    }
    if !saw("admin", "delete_record", true) {
        problems
            .push("the admin's delete_record did not reach the server with the admin's key".into());
    }
    if !saw("member", "delete_record", false) {
        problems.push("the member's delete_record was not tried with the member's key".into());
    }
    let unknown = calls.iter().filter(|c| c.role == "none").count();
    if unknown > 0 {
        problems.push(format!(
            "{unknown} request(s) reached the server without a known key"
        ));
    }

    // What each agent was told by the server, and what it reported.
    let tickets = app.tickets(project.id)?;
    let mut seen_by: BTreeMap<String, Value> = BTreeMap::new();
    for (k, agent) in &keys {
        let (name, role, other) = if *agent == admin.id {
            (&admin.name, "admin", "member")
        } else {
            (&member.name, "member", "admin")
        };
        let mut told = vec![];
        for s in summaries.iter().filter(|s| &s.agent == name) {
            for e in app.store().read(|c| runs::log_entries(c, s.run))? {
                let tool = e.data["tool"].as_str().unwrap_or_default();
                if e.kind == "tool" && tool.contains("acme") {
                    told.push(format!(
                        "{tool}: {}",
                        e.data["result"]
                            .as_str()
                            .unwrap_or_default()
                            .chars()
                            .take(160)
                            .collect::<String>()
                    ));
                }
            }
        }
        if !told.iter().any(|t| t.contains(&format!("role: {role}"))) {
            problems.push(format!("{name} was never told it has the {role} role"));
        }
        if told.iter().any(|t| t.contains(&format!("role: {other}"))) {
            problems.push(format!("{name} was told it has the {other} role"));
        }
        let t = tickets
            .iter()
            .find(|t| &t.key == k)
            .ok_or_else(|| anyhow::anyhow!("{k} is gone"))?;
        let report = app
            .store()
            .read(|c| comments::for_ticket(c, t.id))?
            .into_iter()
            .rev()
            .find_map(|c| match c.body {
                CommentBody::Report(r) => Some(r),
                _ => None,
            });
        let said = report
            .as_ref()
            .map(|r| {
                std::iter::once(r.summary.clone())
                    .chain(r.acceptance.iter().map(|a| a.evidence.clone()))
                    .collect::<Vec<_>>()
                    .join(" ")
                    .to_lowercase()
            })
            .unwrap_or_default();
        let refused = [
            "forbidden",
            "refus",
            "denied",
            "not allowed",
            "cannot",
            "can't",
            "not permitted",
        ];
        match (&report, role) {
            (None, _) => problems.push(format!("{k} ({name}) has no report")),
            (Some(_), "admin") if !said.contains("admin") || !said.contains("deleted") => problems
                .push(format!(
                    "{k}: {name}'s report does not say admin and deleted"
                )),
            (Some(_), "member")
                if !said.contains("member") || !refused.iter().any(|w| said.contains(w)) =>
            {
                problems.push(format!(
                    "{k}: {name}'s report does not say member and refused"
                ))
            }
            _ => {}
        }
        seen_by.insert(
            name.clone(),
            json!({"ticket": k.to_string(), "role": role, "status": t.status.to_string(), "told": told}),
        );
    }

    // The keys: shown in the app and kept in the configuration, never in a log, a ticket, an event or another file.
    let mut leaks: Vec<String> = vec![];
    let has_key = |s: &str| s.contains(ADMIN_KEY) || s.contains(MEMBER_KEY);
    for s in &summaries {
        if has_key(&s.text) {
            leaks.push(format!("{} {} run text", s.agent, s.kind));
        }
        for e in app.store().read(|c| runs::log_entries(c, s.run))? {
            if has_key(&e.data.to_string()) {
                leaks.push(format!("{} {} run log ({})", s.agent, s.kind, e.kind));
            }
        }
    }
    for t in &tickets {
        for c in app.store().read(|c| comments::for_ticket(c, t.id))? {
            if has_key(&serde_json::to_string(&c.body)?) {
                leaks.push(format!("{} comment", t.key));
            }
        }
    }
    for e in app.store().read(|c| events::since(c, 0, 100_000))? {
        if has_key(&e.payload.to_string()) {
            leaks.push(format!("event {}", e.kind));
        }
    }
    key_files(&root_dir, &mut leaks);
    if !leaks.is_empty() {
        problems.push(format!("a key shows in: {}", leaks.join(", ")));
    }

    let closed = tickets
        .iter()
        .filter(|t| keys.iter().any(|(k, _)| k == &t.key))
        .all(|t| t.status == TicketStatus::Closed);
    let (mut tool_errors, mut error_codes, mut errors) = (0u32, BTreeMap::new(), vec![]);
    let mut tools = BTreeMap::new();
    for s in &summaries {
        for e in app.store().read(|c| runs::log_entries(c, s.run))? {
            if e.kind != "tool" {
                continue;
            }
            let tool = e.data["tool"].as_str().unwrap_or_default();
            *tools.entry(format!("{}/{tool}", s.kind)).or_insert(0) += 1;
            if e.data["ok"] == false {
                tool_errors += 1;
                let res = e.data["result"].as_str().unwrap_or_default();
                *error_codes
                    .entry(res.split(':').next().unwrap_or_default().to_owned())
                    .or_insert(0) += 1;
                errors.push(
                    format!("{}/{tool}: {res}", s.kind)
                        .chars()
                        .take(300)
                        .collect(),
                );
            }
        }
    }
    let usage = summaries.iter().fold(UsageSummary::default(), |mut u, s| {
        u.add(&s.usage);
        u
    });
    Ok(TaskResult {
        id: ID.into(),
        passed: problems.is_empty(),
        closed,
        detail: if problems.is_empty() {
            "ok".into()
        } else {
            problems.join("; ")
        },
        runs: summaries.len(),
        steps: summaries.iter().map(|s| s.steps).sum(),
        tool_errors,
        error_codes,
        errors,
        tools,
        outcomes: summaries
            .iter()
            .map(|s| {
                format!(
                    "{}:{}:{}:{}",
                    s.kind,
                    s.agent,
                    s.ticket.as_ref().map(|k| k.to_string()).unwrap_or_default(),
                    s.outcome
                )
            })
            .collect(),
        reopened: tickets.iter().map(|t| t.reopen_count).sum(),
        tokens_in: usage.tokens_in,
        tokens_cached: usage.cache_read,
        tokens_out: usage.tokens_out,
        cost_usd: usage.estimated_cost_usd,
        elapsed_ms: start.elapsed().as_millis() as u64,
        learning: None,
        discipline: Some(json!({
            "server_calls": calls
                .iter()
                .map(|c| format!("{}:{}:{}", c.role, c.tool, if c.ok { "ok" } else { "refused" }))
                .collect::<Vec<_>>(),
            "agents": seen_by,
            "key_leaks": leaks,
            "workers_on": worker_cli.as_ref().map_or_else(|| "api".to_owned(), |(c, m)| format!("{c} {m}")),
        })),
    })
}
