//! Closing a project (ADR-130): only once its work is done; its team is set free; reopening is the user's own step.

use super::*;
use agenium_orchestrator::project_close::Taker;

/// The user closes a done project: nothing runs on it any more and its team is set free, members and Lead together,
/// to take another project. A project with work open cannot be closed; a closed one takes no team until the user
/// reopens it, which offers the team that worked on it.
#[test]
fn a_done_project_closes_and_sets_its_team_free() {
    use agenium_orchestrator::views::RunState;
    let f = fixture();
    let a = solo_with_plan(&f, &[("PI-1", &[])]);
    let k = started(&f, a.id, "PI-1", "Look something up", &[], &[]);
    let run = f.app.start_run(f.project.id, &k, "mock", "m").unwrap();
    let refused = |f: &Fixture| {
        matches!(
            f.app.close_project(f.project.id),
            Err(OrchError::Conflict(_))
        )
    };
    assert!(refused(&f), "work is open");
    f.app
        .apply(
            &ctx(&f, Actor::Worker(a.id)),
            Intent::SubmitReport(report(k.as_str())),
        )
        .unwrap();
    f.app
        .finish_run(run.run, "Completed", 1, &UsageSummary::default())
        .unwrap();
    let lead = Actor::Lead(a.id);
    f.app
        .apply(
            &ctx(&f, lead),
            Intent::ReviewTicket(ReviewDecision {
                ticket_id: k.clone(),
                decision: ReviewVerdict::Approve,
                ac_feedback: vec![],
                required_changes: vec![],
            }),
        )
        .unwrap();
    assert!(refused(&f), "a resolved ticket is still open");
    f.app
        .apply(
            &ctx(&f, lead),
            Intent::CloseTicket(CloseTicket {
                ticket_id: k.clone(),
                reason: "Answer recorded".into(),
            }),
        )
        .unwrap();
    assert_eq!(f.app.run_state(f.project.id).unwrap(), RunState::Done);

    let (team, _) = f.app.team_of_project(f.project.id).unwrap().unwrap();
    assert_eq!(
        f.app.close_project(f.project.id).unwrap(),
        (Some(team.id), None)
    );
    assert_eq!(f.app.run_state(f.project.id).unwrap(), RunState::Closed);
    assert!(f.app.team_of_project(f.project.id).unwrap().is_none());
    let (free, members) = f.app.team(team.id).unwrap();
    assert_eq!(
        (free.project, free.lead),
        (None, Some(a.id)),
        "the team stays whole, without a project"
    );
    assert_eq!(members.len(), 1);
    assert!(refused(&f), "closed already");

    // A closed project takes no team until it is reopened; reopening offers the team that worked on it.
    assert!(matches!(
        f.app.set_team_project(team.id, Some(f.project.id)),
        Err(OrchError::Conflict(_))
    ));
    assert_eq!(f.app.previous_team(f.project.id).unwrap(), Some(team.id));
    f.app
        .reopen_project(f.project.id, Taker::Team(team.id))
        .unwrap();
    assert_eq!(f.app.run_state(f.project.id).unwrap(), RunState::Done);
    assert_eq!(
        f.app.project(f.project.id).unwrap().status,
        agenium_store::core::projects::ProjectStatus::Active
    );
    assert_eq!(
        f.app
            .team_of_project(f.project.id)
            .unwrap()
            .map(|(t, _)| t.id),
        Some(team.id)
    );
    assert!(matches!(
        f.app.reopen_project(f.project.id, Taker::Nobody),
        Err(OrchError::Conflict(_))
    ));
}

/// A reopened project may wait for the team that worked on it while that team is busy on another project: it gets
/// the team as soon as the team is set free, here by the other project closing.
#[test]
fn a_reopened_project_waits_for_its_busy_team() {
    let f = fixture();
    let other = f
        .app
        .create_project(&agenium_orchestrator::app::CreateProject {
            name: "other".into(),
            path: None,
            key_prefix: Some("OTH".into()),
            template: None,
            folder: None,
            repo: None,
        })
        .unwrap();
    let a = solo_with_plan(&f, &[("PI-1", &[])]);
    let lead = Actor::Lead(a.id);
    let k = started(&f, a.id, "PI-1", "Look something up", &[], &[]);
    let run = f.app.start_run(f.project.id, &k, "mock", "m").unwrap();
    f.app
        .apply(
            &ctx(&f, Actor::Worker(a.id)),
            Intent::SubmitReport(report(k.as_str())),
        )
        .unwrap();
    f.app
        .finish_run(run.run, "Completed", 1, &UsageSummary::default())
        .unwrap();
    f.app
        .apply(
            &ctx(&f, lead),
            Intent::ReviewTicket(ReviewDecision {
                ticket_id: k.clone(),
                decision: ReviewVerdict::Approve,
                ac_feedback: vec![],
                required_changes: vec![],
            }),
        )
        .unwrap();
    f.app
        .apply(
            &ctx(&f, lead),
            Intent::CloseTicket(CloseTicket {
                ticket_id: k.clone(),
                reason: "Answer recorded".into(),
            }),
        )
        .unwrap();
    let (team, _) = f.app.team_of_project(f.project.id).unwrap().unwrap();
    f.app.close_project(f.project.id).unwrap();
    // The team goes on to the other project; the first one is reopened to wait for it.
    f.app.set_team_project(team.id, Some(other.id)).unwrap();
    f.app
        .reopen_project(f.project.id, Taker::WaitFor(team.id))
        .unwrap();
    assert_eq!(f.app.waits_for(f.project.id).unwrap(), Some(team.id));
    assert!(f.app.team_of_project(f.project.id).unwrap().is_none());
    // The other project has no work: freeing the team in Teams hands it over at once.
    f.app.set_team_project(team.id, None).unwrap();
    assert_eq!(
        f.app
            .team_of_project(f.project.id)
            .unwrap()
            .map(|(t, _)| t.id),
        Some(team.id)
    );
    assert_eq!(f.app.waits_for(f.project.id).unwrap(), None);
}

/// A project with nothing to do, a Solo team on it, ready to be closed: its one ticket reviewed and closed.
fn done(f: &Fixture) -> (agenium_store::core::agents::Agent, TeamId) {
    let a = solo_with_plan(f, &[("PI-1", &[])]);
    let lead = Actor::Lead(a.id);
    let k = started(f, a.id, "PI-1", "Look something up", &[], &[]);
    let run = f.app.start_run(f.project.id, &k, "mock", "m").unwrap();
    f.app
        .apply(
            &ctx(f, Actor::Worker(a.id)),
            Intent::SubmitReport(report(k.as_str())),
        )
        .unwrap();
    f.app
        .finish_run(run.run, "Completed", 1, &UsageSummary::default())
        .unwrap();
    for intent in [
        Intent::ReviewTicket(ReviewDecision {
            ticket_id: k.clone(),
            decision: ReviewVerdict::Approve,
            ac_feedback: vec![],
            required_changes: vec![],
        }),
        Intent::CloseTicket(CloseTicket {
            ticket_id: k.clone(),
            reason: "Answer recorded".into(),
        }),
    ] {
        f.app.apply(&ctx(f, lead), intent).unwrap();
    }
    let (team, _) = f.app.team_of_project(f.project.id).unwrap().unwrap();
    (a, team.id)
}

fn project(f: &Fixture, name: &str, prefix: &str) -> agenium_store::core::projects::Project {
    f.app
        .create_project(&agenium_orchestrator::app::CreateProject {
            name: name.into(),
            path: None,
            key_prefix: Some(prefix.into()),
            template: None,
            folder: None,
            repo: None,
        })
        .unwrap()
}

/// A done project the user stopped stays stopped: it is not closed until it goes on (a stop is a decision to look
/// at it first).
#[test]
fn a_stopped_project_is_not_closed() {
    let f = fixture();
    done(&f);
    f.app
        .set_project_status(
            f.project.id,
            agenium_store::core::projects::ProjectStatus::Paused,
        )
        .unwrap();
    assert!(matches!(
        f.app.close_project(f.project.id),
        Err(OrchError::Conflict(_))
    ));
    f.app
        .set_project_status(
            f.project.id,
            agenium_store::core::projects::ProjectStatus::Active,
        )
        .unwrap();
    f.app.close_project(f.project.id).unwrap();
}

/// Two projects waiting for one team take it in turn as it is set free; a project whose team was dissolved waits
/// for nobody.
#[test]
fn waiting_projects_take_their_team_in_turn() {
    let f = fixture();
    let (_, team) = done(&f);
    f.app.close_project(f.project.id).unwrap();
    let (b, c) = (project(&f, "b", "BBB"), project(&f, "c", "CCC"));
    for p in [&b, &c] {
        f.app
            .set_project_setting(p.id, "waits_for_team", serde_json::json!(team))
            .unwrap();
    }
    // The free team goes to the first project waiting for it; the other waits on.
    assert_eq!(f.app.hand_over(team).unwrap(), Some(b.id));
    assert_eq!(f.app.waits_for(b.id).unwrap(), None);
    assert_eq!(f.app.waits_for(c.id).unwrap(), Some(team));
    // Set free from b (nothing open there), it goes on to c at once.
    f.app.set_team_project(team, None).unwrap();
    assert_eq!(
        f.app.team_of_project(c.id).unwrap().map(|(t, _)| t.id),
        Some(team)
    );
    assert_eq!(f.app.waits_for(c.id).unwrap(), None);
    // A closed project is not given a team: it is reopened first.
    f.app.set_team_project(team, None).unwrap();
    assert!(matches!(
        f.app.set_team_project(team, Some(f.project.id)),
        Err(OrchError::Conflict(_))
    ));
    // A dissolved team leaves the project waiting for it waiting for nobody.
    let d = project(&f, "d", "DDD");
    f.app
        .set_project_setting(d.id, "waits_for_team", serde_json::json!(team))
        .unwrap();
    f.app.dissolve_team(team).unwrap();
    assert_eq!(f.app.waits_for(d.id).unwrap(), None);
}

/// A run of the team still going blocks the close, the Lead answering in the chat too (it marks no one Working):
/// it could still change the project.
#[test]
fn a_run_still_going_blocks_the_close() {
    let f = fixture();
    let (a, _) = done(&f);
    let run = agenium_core::RunId::new();
    f.app
        .store()
        .commit(None, "system", |w| {
            agenium_store::core::runs::insert(
                w.tx,
                &agenium_store::core::runs::Run {
                    id: run,
                    ticket: None,
                    agent: a.id,
                    provider: "mock".into(),
                    model: "m".into(),
                    started_at: "2026-10-28T00:00:00Z".into(),
                    ended_at: None,
                    outcome: None,
                    steps: 0,
                    usage: UsageSummary::default(),
                    heartbeat: None,
                },
            )
        })
        .unwrap();
    assert!(matches!(
        f.app.close_project(f.project.id),
        Err(OrchError::Conflict(_))
    ));
    f.app
        .finish_run(run, "Completed", 1, &UsageSummary::default())
        .unwrap();
    f.app.close_project(f.project.id).unwrap();
}
