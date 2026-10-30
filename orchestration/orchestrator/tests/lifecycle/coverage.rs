//! ADR-147: plan coverage — every requirement of the agreed spec has a plan item, and a cancelled ticket's work is
//! accounted for (carried on by a replacement that closes, or waived by the user) before its item and the project are
//! done.

use super::*;
use agenium_core::plan::Requirement;
use agenium_orchestrator::views::RunState;
use agenium_store::core::runs;

/// Works, reviews and closes a ticket that carries no code.
fn close_no_code(f: &Fixture, a: AgentId, k: &TicketKey) {
    f.app.start_run(f.project.id, k, "mock", "m").unwrap();
    f.app
        .apply(
            &ctx(f, Actor::Worker(a)),
            Intent::SubmitReport(report(k.as_str())),
        )
        .unwrap();
    for r in f.app.store().read(runs::unfinished).unwrap() {
        f.app
            .finish_run(r.id, "Completed", 1, &UsageSummary::default())
            .unwrap();
    }
    f.app
        .apply(&ctx(f, Actor::Lead(a)), Intent::ReviewTicket(approve(k)))
        .unwrap();
    f.app
        .apply(
            &ctx(f, Actor::Lead(a)),
            Intent::CloseTicket(CloseTicket {
                ticket_id: k.clone(),
                reason: "Done and checked".into(),
            }),
        )
        .unwrap();
}

fn item(f: &Fixture, id: &str) -> agenium_orchestrator::views::ItemProgress {
    f.app
        .plan_view(f.project.id)
        .unwrap()
        .items
        .into_iter()
        .find(|i| i.id == id)
        .unwrap()
}

fn state(f: &Fixture) -> RunState {
    f.app.run_state(f.project.id).unwrap()
}

#[test]
fn a_plan_must_cover_every_requirement() {
    let f = fixture();
    let (_, a) = f
        .app
        .create_solo(
            f.project.id,
            &agent_spec("ada", "solo", RoleKind::Lead, &["rust"]),
        )
        .unwrap();
    let lead = Actor::Lead(a.id);
    let mut s = spec();
    s.requirements.push(Requirement {
        id: "REQ-2".into(),
        text: "A farewell file exists".into(),
        acceptance: vec![AcceptanceCriterion {
            id: "AC-1".into(),
            text: "bye.txt says bye".into(),
        }],
    });
    f.app.apply(&ctx(&f, lead), Intent::ProposeSpec(s)).unwrap();
    let e = f
        .app
        .apply(&ctx(&f, lead), Intent::CreatePlan(plan(&[("PI-1", &[])])))
        .unwrap_err();
    assert!(
        e.to_string().contains("no plan item traces to REQ-2"),
        "{e}"
    );
    let mut p = plan(&[("PI-1", &[]), ("PI-2", &[])]);
    p.items[1].trace_to = vec!["REQ-2".into()];
    f.app.apply(&ctx(&f, lead), Intent::CreatePlan(p)).unwrap();
}

#[test]
fn a_lead_cancel_waits_for_a_replacement_that_closes() {
    let f = fixture();
    let a = solo_with_plan(&f, &[("PI-1", &[])]);
    let lead = Actor::Lead(a.id);
    let k1 = started(&f, a.id, "PI-1", "Write the greeting", &[], &["a/**"]);
    let k2 = started(&f, a.id, "PI-1", "Translate the greeting", &[], &["b/**"]);
    close_no_code(&f, a.id, &k1);

    // Half the item's work is cancelled with nothing to carry it on: the item is not done, the user is asked.
    f.app.apply(&ctx(&f, lead), cancel(k2.as_str())).unwrap();
    assert!(!item(&f, "PI-1").done);
    assert_eq!(item(&f, "PI-1").unaccounted, vec![k2.clone()]);
    assert_ne!(
        state(&f),
        RunState::Done,
        "one closed ticket is not the item"
    );
    let asked = f
        .app
        .store()
        .read(|c| events::since(c, 0, 1000))
        .unwrap()
        .iter()
        .filter_map(|e| e.decode())
        .any(|e| matches!(e, DomainEvent::NeedsUser { reason, .. } if reason.contains("no ticket carries its work on")));
    assert!(asked);
    let inbox = f.app.lead_inbox(f.project.id).unwrap();
    assert!(
        inbox
            .iter()
            .any(|i| i.text.contains(k2.as_str())
                && i.text.contains("whose work no ticket carries on")),
        "{inbox:?}"
    );

    // A ticket that follows it carries the work on: the item is done once that one closes.
    let mut nt = new_ticket("PI-1", "Translate the greeting again", &[], &["b/**"]);
    nt.follows = Some(k2.clone());
    let out = f
        .app
        .apply(&ctx(&f, lead), Intent::CreateTicket(nt))
        .unwrap();
    let k3: TicketKey = serde_json::from_value(out.result["ticket_id"].clone()).unwrap();
    assert!(item(&f, "PI-1").unaccounted.is_empty());
    assert!(!item(&f, "PI-1").done, "until the replacement closes");
    f.app
        .apply(
            &ctx(&f, lead),
            Intent::AssignTicket(AssignTicket {
                ticket_id: k3.clone(),
                worker: a.id,
                note: None,
            }),
        )
        .unwrap();
    close_no_code(&f, a.id, &k3);
    assert!(item(&f, "PI-1").done);
    assert_eq!(state(&f), RunState::Done);
}

#[test]
fn the_users_cancel_or_waiver_accounts_for_the_work() {
    let f = fixture();
    let a = solo_with_plan(&f, &[("PI-1", &[]), ("PI-2", &[])]);
    let lead = Actor::Lead(a.id);
    let k1 = started(&f, a.id, "PI-1", "Write the greeting", &[], &["a/**"]);
    let k2 = started(&f, a.id, "PI-1", "Polish the greeting", &[], &["b/**"]);
    let k3 = started(&f, a.id, "PI-2", "Write the farewell", &[], &["c/**"]);
    close_no_code(&f, a.id, &k1);

    // The user's own cancel is a waiver: nothing waits for it.
    f.app
        .apply(&ctx(&f, Actor::User), cancel(k2.as_str()))
        .unwrap();
    assert!(item(&f, "PI-1").done);

    // The Lead cancels PI-2's only ticket: the item waits; the user waives it, and the project is done.
    assert!(
        f.app.waive_cancelled(f.project.id, &k3, None).is_err(),
        "not cancelled yet"
    );
    f.app.apply(&ctx(&f, lead), cancel(k3.as_str())).unwrap();
    assert_eq!(item(&f, "PI-2").unaccounted, vec![k3.clone()]);
    assert_ne!(state(&f), RunState::Done);
    f.app
        .waive_cancelled(f.project.id, &k3, Some("the farewell is out of scope now"))
        .unwrap();
    let p2 = item(&f, "PI-2");
    assert!(p2.waived && !p2.done && p2.unaccounted.is_empty());
    assert_eq!(state(&f), RunState::Done);
}

/// ADR-147: the handover report says what was agreed and delivered — coverage per requirement, the result per
/// criterion with what checked it, the review and who made it (a Solo agent's own review is said so), cancellations —
/// and still names who worked once the project is closed and its team set free.
#[test]
fn the_handover_report_is_complete_and_outlives_the_team() {
    let f = fixture();
    let a = solo_with_plan(&f, &[("PI-1", &[])]);
    let lead = Actor::Lead(a.id);
    let k1 = started(&f, a.id, "PI-1", "Write the greeting", &[], &["a/**"]);
    let k2 = started(&f, a.id, "PI-1", "Translate the greeting", &[], &["b/**"]);
    close_no_code(&f, a.id, &k1);
    f.app.apply(&ctx(&f, lead), cancel(k2.as_str())).unwrap();
    f.app.waive_cancelled(f.project.id, &k2, None).unwrap();
    assert_eq!(state(&f), RunState::Done);
    let file = f.app.save_handover(f.project.id).unwrap();
    assert!(file.ends_with("handover-v1.md"));
    let md = std::fs::read_to_string(&file).unwrap();
    for part in [
        "## Requirements and coverage",
        "Covered by: PI-1 (done)",
        "### AGN-1 Write the greeting (Closed)",
        "AC-1 Met: deliverables/hello.txt — check: no check cited",
        "Review: approved by ada (self-review, Solo)",
        "Cancelled: No longer needed — waived",
        "## Usage",
    ] {
        assert!(md.contains(part), "missing {part:?}:\n{md}");
    }
    assert!(f
        .app
        .save_handover(f.project.id)
        .unwrap()
        .ends_with("handover-v2.md"));

    f.app.close_project(f.project.id).unwrap();
    let md = f.app.project_report_md(f.project.id).unwrap();
    assert!(
        md.contains("| AGN-1 | Write the greeting | Closed | ada |"),
        "the author outlives the team:\n{md}"
    );
}
