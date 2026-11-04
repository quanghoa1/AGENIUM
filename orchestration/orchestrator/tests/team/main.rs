#[path = "../common/mod.rs"]
mod common;

use agenium_core::forms::*;
use agenium_core::*;
use agenium_orchestrator::*;
use agenium_store::core::runs;
use common::*;

mod judge;
mod lead;
mod memory_revert;
mod roles;
mod skills;
mod teams;

struct Team {
    f: Fixture,
    lead: AgentId,
    web: AgentId,
    api: AgentId,
}

fn team() -> Team {
    let f = fixture();
    // Project network allowlist (manifest) so the Lead can approve requests inside it.
    let mut m = f.app.manifest(&f.project).unwrap();
    m.network.allowlist = vec!["crates.io".into(), "*.npmjs.org".into()];
    m.save(std::path::Path::new(&f.project.path)).unwrap();
    let (_, members) = f
        .app
        .create_team(
            f.project.id,
            "web team",
            &agent_spec("lead", "lead", RoleKind::Lead, &["planning"]),
            &[
                agent_spec("web-1", "web", RoleKind::Worker, &["web"]),
                agent_spec("api-1", "api", RoleKind::Worker, &["rust"]),
            ],
        )
        .unwrap();
    let (lead, web, api) = (members[0].id, members[1].id, members[2].id);
    f.app
        .apply(&ctx(&f, Actor::Lead(lead)), Intent::ProposeSpec(spec()))
        .unwrap();
    Team { f, lead, web, api }
}

fn planned(t: &Team) {
    t.f.app.approve_spec(t.f.project.id, 1).unwrap();
    t.f.app
        .apply(
            &ctx(&t.f, Actor::Lead(t.lead)),
            Intent::CreatePlan(plan(&[("PI-1", &[]), ("PI-2", &[])])),
        )
        .unwrap();
}

/// A one-request mock Jev answering `answer` (leaked: the mock outlives the test body's borrows).
/// A one-shot mock Jev: answers the first request with `answer` (the request body is not read).
async fn jev_once(answer: impl Into<String>) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let answer: String = answer.into();
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut sock, _) = l.accept().await.unwrap();
        let mut buf = [0u8; 65536];
        // Read until the request headers end; the body is irrelevant to the mock.
        loop {
            let n = sock.read(&mut buf).await.unwrap();
            if n == 0 || buf[..n].windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        let resp = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{answer}",
            answer.len()
        );
        sock.write_all(resp.as_bytes()).await.unwrap();
    });
    format!("http://{addr}")
}

fn judge_on(f: &Fixture, base: String, set: impl FnOnce(&mut agenium_config::JudgeSection)) {
    f.app
        .update_config(|c| {
            let mut j = agenium_config::JudgeSection {
                base_url: base,
                model: "jev-latest".into(),
                ..Default::default()
            };
            set(&mut j);
            c.judge = j;
        })
        .unwrap();
}

fn unchecked_report(
    t: &Ticket,
) -> (
    agenium_core::forms::WorkReport,
    agenium_orchestrator::judge::done_check::RunFacts,
) {
    let report = serde_json::from_value(serde_json::json!({
        "ticket_id": t.key.to_string(),
        "summary": "Fixed the parser; all tests pass.",
        "confidence": 0.9,
        "acceptance": t.acceptance.iter().map(|ac| serde_json::json!({"ac_id": ac.id, "status": "Met", "evidence": "fixed"})).collect::<Vec<_>>(),
    }))
    .unwrap();
    let facts = agenium_orchestrator::judge::done_check::run_facts(&[
        agenium_store::core::runs::LogEntry {
            n: 1,
            kind: "tool".into(),
            data: serde_json::json!({"tool": "edit_file", "args": {"path": "src/parser.rs"}, "ok": true, "result": "edited"}),
            at: String::new(),
        },
    ]);
    (report, facts)
}
