//! Merging: conflicts, failed checks, rebase nudges, plan revisions, pushes.

use super::*;

#[test]
fn merge_conflict_and_failed_check_reopen_to_the_same_worker() {
    let f = fixture_with(Some((false, "1 test failed".into())));
    let a = solo_with_plan(&f, &[("PI-1", &[]), ("PI-2", &[])]);
    let (lead, worker) = (Actor::Lead(a.id), Actor::Worker(a.id));
    let k = started(&f, a.id, "PI-1", "Fails its check", &[], &[]);
    let run = f.app.start_run(f.project.id, &k, "mock", "m").unwrap();
    write(&worktree(&f, "AGN-1"), "deliverables/x.txt", "x");
    f.app
        .apply(&ctx(&f, worker), Intent::SubmitReport(report("AGN-1")))
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
    let t = ticket(&f, "AGN-1");
    assert_eq!(
        (t.status, t.assignee, t.approved, t.merged, t.reopen_count),
        (TicketStatus::Reopened, Some(a.id), false, false, 1)
    );
    let cs = f
        .app
        .store()
        .read(|c| comments::for_ticket(c, t.id))
        .unwrap();
    assert!(cs.iter().any(
        |c| matches!(&c.body, CommentBody::System { text } if text.contains("1 test failed"))
    ));
}

#[test]
fn merge_conflict_reopens_with_file_list() {
    let f = fixture();
    let a = solo_with_plan(&f, &[("PI-1", &[]), ("PI-2", &[])]);
    let (lead, worker) = (Actor::Lead(a.id), Actor::Worker(a.id));
    // Two tickets edit the same file; the scope check is bypassed by leaving scope_paths empty.
    let k1 = started(&f, a.id, "PI-1", "First", &[], &[]);
    let r1 = f.app.start_run(f.project.id, &k1, "mock", "m").unwrap();
    write(&worktree(&f, "AGN-1"), "notes/shared.md", "from one\n");
    f.app
        .apply(&ctx(&f, worker), Intent::SubmitReport(report("AGN-1")))
        .unwrap();
    f.app
        .finish_run(r1.run, "Completed", 1, &UsageSummary::default())
        .unwrap();
    let k2 = started(&f, a.id, "PI-2", "Second", &[], &[]);
    let r2 = f.app.start_run(f.project.id, &k2, "mock", "m").unwrap();
    write(&worktree(&f, "AGN-2"), "notes/shared.md", "from two\n");
    f.app
        .apply(&ctx(&f, worker), Intent::SubmitReport(report("AGN-2")))
        .unwrap();
    f.app
        .finish_run(r2.run, "Completed", 1, &UsageSummary::default())
        .unwrap();
    let approve = |k: &TicketKey| {
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
            .unwrap()
    };
    approve(&k1);
    approve(&k2);
    assert!(ticket(&f, "AGN-1").merged);
    let t2 = ticket(&f, "AGN-2");
    assert_eq!(t2.status, TicketStatus::Reopened);
    let cs = f
        .app
        .store()
        .read(|c| comments::for_ticket(c, t2.id))
        .unwrap();
    assert!(cs.iter().any(
        |c| matches!(&c.body, CommentBody::System { text } if text.contains("notes/shared.md"))
    ));
}

/// Decision 18: when a merge changes a file that a ticket still in progress changes too, that ticket is told at once
/// to rebase, not at its own merge.
#[test]
fn a_merge_nudges_work_in_progress_on_the_same_file_to_rebase() {
    let f = fixture();
    let a = solo_with_plan(&f, &[("PI-1", &[]), ("PI-2", &[])]);
    let (lead, worker) = (Actor::Lead(a.id), Actor::Worker(a.id));
    let k1 = started(&f, a.id, "PI-1", "First", &[], &[]);
    let r1 = f.app.start_run(f.project.id, &k1, "mock", "m").unwrap();
    write(&worktree(&f, "AGN-1"), "notes/shared.md", "from one\n");
    f.app
        .apply(&ctx(&f, worker), Intent::SubmitReport(report("AGN-1")))
        .unwrap();
    f.app
        .finish_run(r1.run, "Completed", 1, &UsageSummary::default())
        .unwrap();
    // AGN-2 is still at work on the same file (and on another one) when AGN-1 merges.
    let k2 = started(&f, a.id, "PI-2", "Second", &[], &[]);
    f.app.start_run(f.project.id, &k2, "mock", "m").unwrap();
    write(&worktree(&f, "AGN-2"), "notes/shared.md", "from two\n");
    write(&worktree(&f, "AGN-2"), "notes/other.md", "mine\n");
    f.app
        .apply(
            &ctx(&f, lead),
            Intent::ReviewTicket(ReviewDecision {
                ticket_id: k1.clone(),
                decision: ReviewVerdict::Approve,
                ac_feedback: vec![],
                required_changes: vec![],
            }),
        )
        .unwrap();
    assert!(ticket(&f, "AGN-1").merged);
    let t2 = ticket(&f, "AGN-2");
    assert_eq!(t2.status, TicketStatus::InProgress);
    let cs = f
        .app
        .store()
        .read(|c| comments::for_ticket(c, t2.id))
        .unwrap();
    let nudge = cs.iter().find_map(|c| match &c.body {
        CommentBody::System { text } if text.contains("AGN-1 was merged") => Some(text.clone()),
        _ => None,
    });
    let nudge = nudge.expect("AGN-2 is told to rebase");
    assert!(
        nudge.contains("notes/shared.md") && !nudge.contains("notes/other.md"),
        "{nudge}"
    );
}

/// Found with a real model: after a new spec version adds a requirement, a plan revision must trace to the newest
/// approved spec, not to the spec of the base plan.
#[test]
fn plan_revision_follows_the_newest_approved_spec() {
    let f = fixture();
    let a = solo_with_plan(&f, &[("PI-1", &[])]);
    let lead = Actor::Lead(a.id);
    let mut s2 = spec();
    s2.requirements.push(agenium_core::plan::Requirement {
        id: "REQ-2".into(),
        text: "A second greeting file exists".into(),
        acceptance: vec![agenium_core::ticket::AcceptanceCriterion {
            id: "AC-1".into(),
            text: "hello2.txt says hello".into(),
        }],
    });
    f.app
        .apply(&ctx(&f, lead), Intent::ProposeSpec(s2))
        .unwrap();
    let mut p = plan(&[("PI-1", &[]), ("PI-2", &[])]);
    p.items[1].trace_to = vec!["REQ-2".into()];
    let out = f
        .app
        .apply(
            &ctx(&f, lead),
            Intent::RevisePlan(agenium_core::plan::PlanRevision {
                base_version: 1,
                reason: "add the second greeting".into(),
                milestones: p.milestones,
                items: p.items,
            }),
        )
        .unwrap();
    assert_eq!(out.result["plan_version"], 2);
    let latest = f
        .app
        .store()
        .read(|c| agenium_store::core::plans::latest_plan(c, f.project.id))
        .unwrap()
        .unwrap();
    assert_eq!(latest.body.spec_version, 2);
}

/// Push on close only when the project has a remote (the user added a repo): the merged main lands on origin;
/// without a remote the work stays local. A push failure does not block the close.
#[test]
fn a_closed_ticket_pushes_when_the_project_has_a_remote() {
    let f = fixture();
    let root = std::path::Path::new(&f.project.path);
    let a = solo_with_plan(&f, &[("PI-1", &[])]);
    let (lead, worker) = (Actor::Lead(a.id), Actor::Worker(a.id));
    let k = started(&f, a.id, "PI-1", "Write hello", &[], &["deliverables/**"]);
    let run = f
        .app
        .start_run(f.project.id, &k, "mock", "scripted")
        .unwrap();
    let wt = worktree(&f, "AGN-1");
    write(&wt, "deliverables/hello.txt", "hello\n");
    f.app
        .apply(&ctx(&f, worker), Intent::SubmitReport(report("AGN-1")))
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
    assert!(ticket(&f, "AGN-1").merged);

    // The user adds a repo: a bare origin.
    let bare = f.root.parent().unwrap().join("origin.git");
    let git = agenium_vcs::git::Git::default();
    git.check(root, ["init", "-q", "--bare", bare.to_str().unwrap()])
        .unwrap();
    let url = bare.to_string_lossy().replace('\\', "/");
    git.check(root, ["remote", "add", "origin", &url]).unwrap();

    f.app
        .apply(
            &ctx(&f, lead),
            Intent::CloseTicket(CloseTicket {
                ticket_id: k.clone(),
                reason: "merged".into(),
            }),
        )
        .unwrap();
    assert_eq!(ticket(&f, "AGN-1").status, TicketStatus::Closed);
    assert!(
        f.app.store().read(ops::pending).unwrap().is_empty(),
        "push op ran"
    );
    let heads = git.check(root, ["ls-remote", "--heads", &url]).unwrap();
    assert!(
        heads.contains("refs/heads/main"),
        "main pushed to origin: {heads}"
    );
}
