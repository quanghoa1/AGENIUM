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

/// ADR-134: a ticket's kept CLI sessions end with its worktree when it closes (they held its work in progress); another
/// ticket's stay.
#[test]
fn a_closed_tickets_kept_cli_sessions_go_with_its_worktree() {
    let f = fixture();
    let a = solo_with_plan(&f, &[("PI-1", &[]), ("PI-2", &[])]);
    let (lead, worker) = (Actor::Lead(a.id), Actor::Worker(a.id));
    let k = started(&f, a.id, "PI-1", "Writes x", &[], &[]);
    let kept = f.app.cli_sessions(a.id);
    for key in ["AGN-1", "AGN-2"] {
        std::fs::create_dir_all(kept.join(key).join("home")).unwrap();
        std::fs::write(kept.join(key).join("agenium-session.json"), "{}").unwrap();
    }
    let run = f.app.start_run(f.project.id, &k, "mock", "m").unwrap();
    write(&worktree(&f, "AGN-1"), "deliverables/x.txt", "x");
    f.app
        .apply(&ctx(&f, worker), Intent::SubmitReport(report("AGN-1")))
        .unwrap();
    f.app
        .finish_run(run.run, "Completed", 1, &UsageSummary::default())
        .unwrap();
    f.app
        .apply(&ctx(&f, lead), Intent::ReviewTicket(approve(&k)))
        .unwrap();
    assert!(ticket(&f, "AGN-1").merged);
    assert!(kept.join("AGN-1").exists(), "kept while the ticket is open");
    f.app
        .apply(
            &ctx(&f, lead),
            Intent::CloseTicket(CloseTicket {
                ticket_id: k.clone(),
                reason: "Merged".into(),
            }),
        )
        .unwrap();
    assert!(
        !kept.join("AGN-1").exists(),
        "the closed ticket's session is gone"
    );
    assert!(kept.join("AGN-2").exists(), "another ticket's stays");
}

/// A pre-merge check that does something while it runs (the user acting during a test suite), then passes.
struct DuringCheck(std::sync::Mutex<Option<Box<dyn FnOnce() + Send>>>);

impl agenium_vcs::CheckRunner for DuringCheck {
    fn run(&self, _: &std::path::Path) -> agenium_vcs::CheckOutcome {
        if let Some(act) = self.0.lock().unwrap().take() {
            act();
        }
        agenium_vcs::CheckOutcome {
            ok: true,
            log: String::new(),
        }
    }
}

struct DuringChecks(std::sync::Arc<DuringCheck>);

impl CheckFactory for DuringChecks {
    fn for_project(
        &self,
        _: &agenium_store::core::projects::Project,
        _: &agenium_orchestrator::manifest::ProjectManifest,
    ) -> std::sync::Arc<dyn agenium_vcs::CheckRunner> {
        self.0.clone()
    }
}

/// The app, handed to the check: it runs on the merging thread, inside the approval's call, while the fixture lives.
struct AppRef(*const App);
// SAFETY: the check runs synchronously on the test's thread while the fixture (and so the app) is alive.
unsafe impl Send for AppRef {}

/// Audit H1 (2026-10-31): the user reopened a ticket while its merge ran the pre-merge check; the merge still went
/// into main and marked the reopened ticket merged, so the rework approved later was never merged and its branch was
/// deleted. Now the merge is dropped, and the rework merges.
#[test]
fn a_ticket_reopened_while_its_merge_checks_is_not_merged() {
    let during = std::sync::Arc::new(DuringCheck(std::sync::Mutex::new(None)));
    let f = fixture_checked(std::sync::Arc::new(DuringChecks(during.clone())));
    let a = solo_with_plan(&f, &[("PI-1", &[])]);
    let (lead, worker) = (Actor::Lead(a.id), Actor::Worker(a.id));
    let k = started(&f, a.id, "PI-1", "Reopened during its check", &[], &[]);
    let run = f.app.start_run(f.project.id, &k, "mock", "m").unwrap();
    write(
        &worktree(&f, "AGN-1"),
        "deliverables/first.txt",
        "first try",
    );
    f.app
        .apply(&ctx(&f, worker), Intent::SubmitReport(report("AGN-1")))
        .unwrap();
    f.app
        .finish_run(run.run, "Completed", 1, &UsageSummary::default())
        .unwrap();
    let app = AppRef(&f.app);
    let (project, key) = (f.project.id, k.clone());
    *during.0.lock().unwrap() = Some(Box::new(move || {
        let app = app;
        // SAFETY: see `AppRef`.
        let app = unsafe { &*app.0 };
        app.apply(
            &CallCtx {
                project,
                actor: Actor::User,
                intent_id: Some(ulid::Ulid::new().to_string()),
                run: None,
            },
            Intent::ReviewTicket(ReviewDecision {
                ticket_id: key,
                decision: ReviewVerdict::Reopen,
                ac_feedback: vec![AcFeedback {
                    ac_id: "AC-1".into(),
                    feedback: "not this way".into(),
                }],
                required_changes: vec![],
            }),
        )
        .unwrap();
    }));
    f.app
        .apply(&ctx(&f, lead), Intent::ReviewTicket(approve(&k)))
        .unwrap();
    let t = ticket(&f, "AGN-1");
    assert_eq!((t.status, t.merged), (TicketStatus::Reopened, false));
    let main = std::path::Path::new(&f.project.path);
    assert!(
        !main.join("deliverables/first.txt").exists(),
        "the reopened work stayed out of main"
    );

    // The rework is approved and merged.
    let run = f.app.start_run(f.project.id, &k, "mock", "m").unwrap();
    write(&worktree(&f, "AGN-1"), "deliverables/second.txt", "rework");
    f.app
        .apply(&ctx(&f, worker), Intent::SubmitReport(report("AGN-1")))
        .unwrap();
    f.app
        .finish_run(run.run, "Completed", 1, &UsageSummary::default())
        .unwrap();
    f.app
        .apply(&ctx(&f, lead), Intent::ReviewTicket(approve(&k)))
        .unwrap();
    let t = ticket(&f, "AGN-1");
    assert!(t.merged, "the rework is merged");
    assert!(main.join("deliverables/second.txt").exists());
}

/// Audit L8 (2026-10-31): the Lead closes a ticket from its review run, whose root is the ticket's worktree; the
/// worktree was removed under that run. It goes when the run ends, as for a cancel.
#[test]
fn a_ticket_closed_from_its_review_run_keeps_its_worktree_until_the_run_ends() {
    let f = fixture();
    let a = solo_with_plan(&f, &[("PI-1", &[])]);
    let (lead, worker) = (Actor::Lead(a.id), Actor::Worker(a.id));
    let k = started(&f, a.id, "PI-1", "Closed from its review", &[], &[]);
    let run = f.app.start_run(f.project.id, &k, "mock", "m").unwrap();
    write(&worktree(&f, "AGN-1"), "deliverables/x.txt", "x");
    f.app
        .apply(&ctx(&f, worker), Intent::SubmitReport(report("AGN-1")))
        .unwrap();
    f.app
        .finish_run(run.run, "Completed", 1, &UsageSummary::default())
        .unwrap();
    f.app
        .apply(&ctx(&f, lead), Intent::ReviewTicket(approve(&k)))
        .unwrap();
    assert!(ticket(&f, "AGN-1").merged);
    // The Lead's review run on the ticket, live.
    let t = ticket(&f, "AGN-1");
    let review = RunId::new();
    f.app
        .store()
        .commit(None, "system", |w| {
            agenium_store::core::runs::insert(
                w.tx,
                &agenium_store::core::runs::Run {
                    id: review,
                    ticket: Some(t.id),
                    agent: a.id,
                    provider: "mock".into(),
                    model: "m".into(),
                    started_at: now_iso(),
                    ended_at: None,
                    outcome: None,
                    steps: 0,
                    usage: UsageSummary::default(),
                    heartbeat: Some(now_iso()),
                },
            )
        })
        .unwrap();
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
        worktree(&f, "AGN-1").is_dir(),
        "kept while the review run works in it"
    );
    f.app
        .finish_run(review, "Completed", 1, &UsageSummary::default())
        .unwrap();
    assert!(
        !worktree(&f, "AGN-1").exists(),
        "removed when the run ended"
    );
}

/// Audit L22 (2026-10-31): a report sent again with the same intent id (a retry after its answer was lost) failed its
/// handler's checks ("not in progress") instead of returning the stored result.
#[test]
fn a_report_sent_again_returns_what_it_returned() {
    let f = fixture();
    let a = solo_with_plan(&f, &[("PI-1", &[])]);
    let k = started(&f, a.id, "PI-1", "Sent twice", &[], &[]);
    f.app.start_run(f.project.id, &k, "mock", "m").unwrap();
    write(&worktree(&f, "AGN-1"), "deliverables/x.txt", "x");
    let call = CallCtx {
        project: f.project.id,
        actor: Actor::Worker(a.id),
        intent_id: Some("report-once".into()),
        run: None,
    };
    let first = f
        .app
        .apply(&call, Intent::SubmitReport(report("AGN-1")))
        .unwrap();
    let again = f
        .app
        .apply(&call, Intent::SubmitReport(report("AGN-1")))
        .unwrap();
    assert_eq!(again.result, first.result);
    assert_eq!((again.run, first.run), (RunEffect::End, RunEffect::End));
    assert!(again.ops.is_empty());
}
