//! Workers that get stuck: the Lead is asked how they go on, and a stopped ticket goes on without waiting for the
//! other runs of the round.

use super::*;

/// A worker that repeats the same failing call is told to change course once; when it repeats it again its run stops
/// and its Lead is asked how it goes on (a blocking Blocker question on the ticket, in the Lead's inbox), instead of
/// the ticket going quiet (2026-11-19).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_worker_stuck_on_the_same_call_asks_its_lead() {
    let f = fixture();
    let p = Arc::new(ScriptedProvider::new("mock"));
    let (app, rt) = runtime(&f, p.clone(), Arc::new(AutoApprove));
    app.update_config(|c| c.memory.learn = false).unwrap();
    let (project, lead, worker) = team_project(&app, "stuck");
    let as_lead = agenium_orchestrator::CallCtx {
        project,
        actor: Actor::Lead(lead),
        intent_id: None,
        run: None,
    };
    app.apply(&as_lead, Intent::ProposeSpec(spec())).unwrap();
    app.approve_spec(project, 1).unwrap();
    app.apply(&as_lead, Intent::CreatePlan(plan(&[("PI-1", &[])])))
        .unwrap();
    app.apply(
        &as_lead,
        Intent::CreateTicket(new_ticket("PI-1", "t", &[], &[])),
    )
    .unwrap();
    app.apply(
        &as_lead,
        Intent::AssignTicket(agenium_core::forms::AssignTicket {
            ticket_id: key("AGN-1"),
            worker,
            note: None,
        }),
    )
    .unwrap();
    // The same read of a missing file, four times: the third is refused with a warning, the fourth stops the run.
    for _ in 0..4 {
        p.push_calls("", vec![("read_file", json!({"path": "missing.txt"}))]);
    }
    let s = rt.run_ticket(project, &key("AGN-1")).await.unwrap();
    assert_eq!(s.outcome, "Stopped:loop:read_file");
    assert_eq!(
        app.ticket(project, &key("AGN-1")).unwrap().status,
        TicketStatus::Pending,
        "the ticket waits for the answer"
    );
    let inbox = app.lead_inbox(project).unwrap();
    let asked = inbox
        .iter()
        .find(|i| i.text.contains("Agenium stopped my run"))
        .unwrap_or_else(|| panic!("the Lead is asked: {inbox:?}"));
    assert!(
        asked.text.contains("Blocker") && asked.text.contains("read_file"),
        "{}",
        asked.text
    );
    assert!(
        asked.text.contains("missing.txt"),
        "the last failing call is shown: {}",
        asked.text
    );
}

/// A worker's run that stopped with its ticket still in progress goes on in the same round, while another worker's
/// long run keeps the round open: it waited for the whole round to end (2026-11-19: 45 minutes, its worker idle).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_stopped_ticket_goes_on_while_another_run_is_long() {
    let f = fixture();
    let p = Arc::new(ScriptedProvider::new("mock"));
    let (app, rt) = runtime(&f, p.clone(), Arc::new(AutoApprove));
    app.update_config(|c| c.memory.learn = false).unwrap();
    let project = app
        .create_project(&agenium_orchestrator::CreateProject {
            name: "two".into(),
            path: None,
            key_prefix: None,
            template: None,
            folder: None,
            repo: None,
        })
        .unwrap()
        .id;
    let (_, members) = app
        .create_team(
            project,
            "two team",
            &agent_spec("lead", "lead", RoleKind::Lead, &[]),
            &[
                agent_spec("api-1", "api", RoleKind::Worker, &["rust"]),
                agent_spec("web-1", "web", RoleKind::Worker, &["web"]),
            ],
        )
        .unwrap();
    let (lead, api, web) = (members[0].id, members[1].id, members[2].id);
    let as_lead = agenium_orchestrator::CallCtx {
        project,
        actor: Actor::Lead(lead),
        intent_id: None,
        run: None,
    };
    app.apply(&as_lead, Intent::ProposeSpec(spec())).unwrap();
    app.approve_spec(project, 1).unwrap();
    app.apply(
        &as_lead,
        Intent::CreatePlan(plan(&[("PI-1", &[]), ("PI-2", &[])])),
    )
    .unwrap();
    for (item, worker) in [("PI-1", api), ("PI-2", web)] {
        let k = app
            .apply(
                &as_lead,
                Intent::CreateTicket(new_ticket(item, item, &[], &[])),
            )
            .unwrap()
            .result["ticket_id"]
            .as_str()
            .unwrap()
            .to_owned();
        app.apply(
            &as_lead,
            Intent::AssignTicket(agenium_core::forms::AssignTicket {
                ticket_id: key(&k),
                worker,
                note: None,
            }),
        )
        .unwrap();
    }
    // Every model call answers with words only (the run stops for no progress), but the very first runs a command of
    // a few seconds: that run keeps the round open while the other worker's stops at once.
    let wait = match agenium_sandbox::Shell::detect() {
        agenium_sandbox::Shell::Bash(_) => "sleep 5",
        agenium_sandbox::Shell::PowerShell(_) => "Start-Sleep -Seconds 5",
        agenium_sandbox::Shell::Cmd => "powershell -NoProfile -Command Start-Sleep -Seconds 5",
    };
    let first = Arc::new(std::sync::atomic::AtomicBool::new(true));
    for _ in 0..40 {
        let first = first.clone();
        p.push(Box::new(move |_| {
            Ok(if first.swap(false, std::sync::atomic::Ordering::SeqCst) {
                calls_response("", vec![("shell", json!({ "command": wait }))])
            } else {
                calls_response("thinking out loud", vec![])
            })
        }));
    }
    let _ = rt.work(project).await.unwrap();
    let runs_of = |k: &str| {
        let t = app.ticket(project, &key(k)).unwrap();
        app.store().read(|c| runs::for_ticket(c, t.id)).unwrap()
    };
    // The slow run is the one whose first run ended last; the other worker's ticket went on before it ended.
    let (one, two) = (runs_of("AGN-1"), runs_of("AGN-2"));
    let (slow, quick) = if one[0].ended_at > two[0].ended_at {
        (one, two)
    } else {
        (two, one)
    };
    assert!(
        quick.len() >= 2,
        "the stopped ticket went on: {} run(s)",
        quick.len()
    );
    assert!(
        quick[1].started_at.as_str() < slow[0].ended_at.as_deref().unwrap(),
        "its second run started while the long run went on: {} vs {:?}",
        quick[1].started_at,
        slow[0].ended_at
    );
}
