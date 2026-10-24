//! M3-5 read models and user overrides: board columns, priority/assignee overrides, audit trail (committed events
//! and refused calls), and the board at 500 tickets.

mod common;

use std::time::Instant;

use agenium_core::forms::*;
use agenium_core::*;
use agenium_orchestrator::views::{audit_markdown, COLUMNS};
use common::*;
use serde_json::json;

fn column(
    board: &[agenium_orchestrator::views::Column],
    s: TicketStatus,
) -> &[agenium_orchestrator::views::Card] {
    &board.iter().find(|c| c.status == s).unwrap().cards
}

#[test]
fn board_overrides_and_audit() {
    let f = fixture();
    let a = solo_with_plan(&f, &[("PI-1", &[]), ("PI-2", &["PI-1"])]);
    let k1 = started(&f, a.id, "PI-1", "First", &[], &["src/a/**"]);
    let lead = lead_of(&f);
    f.app
        .apply(
            &ctx(&f, lead),
            Intent::CreateTicket(new_ticket("PI-2", "Second", &["AGN-1"], &["src/b/**"])),
        )
        .unwrap();

    // User override: priority, recorded as an event and a System comment.
    let t2 = f
        .app
        .user_set_priority(f.project.id, &key("AGN-2"), 0)
        .unwrap();
    assert_eq!(t2.priority, 0);
    assert!(f
        .app
        .user_set_priority(f.project.id, &key("AGN-2"), 4)
        .is_err());

    let board = f.app.board(f.project.id).unwrap();
    assert_eq!(
        board.iter().map(|c| c.status).collect::<Vec<_>>(),
        COLUMNS.to_vec()
    );
    let new = column(&board, TicketStatus::New);
    assert_eq!(new.len(), 1);
    assert_eq!(new[0].key, key("AGN-2"));
    assert_eq!(new[0].priority, 0);
    assert_eq!(new[0].blocked_by, vec![key("AGN-1")]);
    let assigned = column(&board, TicketStatus::Assigned);
    assert_eq!(assigned[0].key, k1);
    assert_eq!(assigned[0].assignee.as_deref(), Some("ada"));
    assert!(assigned[0].blocked_by.is_empty());

    let pv = f.app.plan_view(f.project.id).unwrap();
    assert_eq!(pv.spec.as_ref().map(|s| s.version), Some(1));
    assert_eq!(pv.current_plan_version, Some(1));
    let items: Vec<_> = pv
        .items
        .iter()
        .map(|i| (i.id.as_str(), i.done, i.tickets.clone()))
        .collect();
    assert_eq!(
        items,
        vec![
            ("PI-1", false, vec![(k1.clone(), TicketStatus::Assigned)]),
            ("PI-2", false, vec![(key("AGN-2"), TicketStatus::New)]),
        ]
    );

    // A refused protocol call is part of the audit trail (it is only in the run log).
    let run = f.app.start_run(f.project.id, &k1, "mock", "m").unwrap().run;
    f.app
        .run_log(
            run,
            "tool",
            json!({"tool": "submit_report", "args": {"ticket_id": "AGN-1"}, "ok": false, "result": "invalid: missing AC-1"}),
        )
        .unwrap();
    f.app
        .run_log(
            run,
            "tool",
            json!({"tool": "read_file", "args": {"path": "x"}, "ok": false, "result": "not found"}),
        )
        .unwrap();

    let all = f.app.audit(f.project.id, None).unwrap();
    let actions: Vec<&str> = all.iter().map(|e| e.action.as_str()).collect();
    for k in [
        "SpecProposed",
        "PlanProposed",
        "TicketCreated",
        "TicketAssigned",
        "TicketUpdated",
        "RunStarted",
    ] {
        assert!(actions.contains(&k), "{k} missing in {actions:?}");
    }
    let upd = all.iter().find(|e| e.action == "TicketUpdated").unwrap();
    assert_eq!(
        (upd.actor.as_str(), upd.ticket.clone()),
        ("user", Some(key("AGN-2")))
    );
    let created = all.iter().find(|e| e.action == "TicketCreated").unwrap();
    assert_eq!(created.actor, "ada", "agent ids are shown by name");
    assert!(
        created.intent_id.is_some(),
        "the originating intent is kept"
    );
    let refused: Vec<_> = all.iter().filter(|e| !e.accepted).collect();
    assert_eq!(
        refused.len(),
        1,
        "only protocol calls, not work-tool errors"
    );
    assert_eq!(refused[0].action, "submit_report");
    assert_eq!(refused[0].detail["error"], "invalid: missing AC-1");
    assert!(all.windows(2).all(|w| w[0].at <= w[1].at), "time order");

    let one = f.app.audit(f.project.id, Some(&key("AGN-2"))).unwrap();
    assert!(one.iter().all(|e| e.ticket == Some(key("AGN-2"))));
    let md = audit_markdown("AGN-1", &f.app.audit(f.project.id, Some(&k1)).unwrap());
    assert!(
        md.contains("| refused |") && md.contains("TicketAssigned"),
        "{md}"
    );
}

#[test]
fn user_reassigns_an_assigned_ticket_before_its_run() {
    let f = fixture();
    let (_, members) = f
        .app
        .create_team(
            f.project.id,
            "team",
            &agent_spec("lead", "lead", RoleKind::Lead, &[]),
            &[
                agent_spec("w1", "dev", RoleKind::Worker, &[]),
                agent_spec("w2", "dev", RoleKind::Worker, &[]),
            ],
        )
        .unwrap();
    let (lead, w1, w2) = (Actor::Lead(members[0].id), members[1].id, members[2].id);
    f.app
        .apply(&ctx(&f, lead), Intent::ProposeSpec(spec()))
        .unwrap();
    f.app.approve_spec(f.project.id, 1).unwrap();
    f.app
        .apply(&ctx(&f, lead), Intent::CreatePlan(plan(&[("PI-1", &[])])))
        .unwrap();
    f.app.approve_plan(f.project.id, 1).unwrap();
    let k = started(&f, w1, "PI-1", "Task", &[], &[]);
    let reassign = |actor: Actor| {
        f.app.apply(
            &ctx(&f, actor),
            Intent::AssignTicket(AssignTicket {
                ticket_id: k.clone(),
                worker: w2,
                note: None,
            }),
        )
    };
    assert!(
        reassign(lead).is_err(),
        "the Lead cannot move an Assigned ticket"
    );
    reassign(Actor::User).unwrap();
    let t = ticket(&f, k.as_str());
    assert_eq!((t.status, t.assignee), (TicketStatus::Assigned, Some(w2)));
}

#[test]
fn board_of_500_tickets() {
    let f = fixture();
    let a = solo_with_plan(&f, &[("PI-1", &[])]);
    let lead = Actor::Lead(a.id);
    for i in 0..500 {
        let mut nt = new_ticket("PI-1", &format!("Task {i}"), &[], &[]);
        nt.priority = (i % 4) as u8;
        nt.scope_paths = vec![format!("src/m{i}/**")];
        f.app
            .apply(&ctx(&f, lead), Intent::CreateTicket(nt))
            .unwrap();
    }
    let start = Instant::now();
    let board = f.app.board(f.project.id).unwrap();
    let took = start.elapsed();
    let new = column(&board, TicketStatus::New);
    assert_eq!(new.len(), 500);
    assert!(new.windows(2).all(|w| w[0].priority <= w[1].priority));
    eprintln!("board of 500 tickets: {took:?}");
    assert!(took.as_millis() < 2000, "board took {took:?}");
}
