mod common;

use agenium_core::forms::*;
use agenium_core::*;
use agenium_orchestrator::*;
use agenium_store::core::{comments, events, ops, workspaces};
use common::*;

#[test]
fn solo_ticket_full_lifecycle() {
    let f = fixture();
    // Project layout + git (decision 7/11).
    let root = std::path::Path::new(&f.project.path);
    for d in ["deliverables", "artifacts", "notes", "agenium.toml", ".git"] {
        assert!(root.join(d).exists(), "{d}");
    }
    let a = solo_with_plan(&f, &[("PI-1", &[])]);
    let (lead, worker) = (Actor::Lead(a.id), Actor::Worker(a.id));

    let k = started(&f, a.id, "PI-1", "Write hello", &[], &["deliverables/**"]);
    assert_eq!(k.as_str(), "AGN-1");
    let t = ticket(&f, "AGN-1");
    assert_eq!((t.status, t.assignee), (TicketStatus::Assigned, Some(a.id)));
    let wt = worktree(&f, "AGN-1");
    assert!(
        wt.join("agenium.toml").is_file(),
        "worktree created on assignment"
    );

    let ready = f.app.ready_tickets(f.project.id).unwrap();
    assert_eq!(ready.len(), 1);
    let run = f
        .app
        .start_run(f.project.id, &k, "mock", "scripted")
        .unwrap();
    assert!(!run.continuation);
    assert_eq!(ticket(&f, "AGN-1").status, TicketStatus::InProgress);
    assert_eq!(f.app.agent(a.id).unwrap().status, AgentStatus::Working);
    assert!(
        f.app.ready_tickets(f.project.id).unwrap().is_empty(),
        "a live run is not scheduled twice"
    );

    write(&wt, "deliverables/hello.txt", "hello\n");
    let sub = f
        .app
        .apply(&ctx(&f, worker), Intent::SubmitReport(report("AGN-1")))
        .unwrap();
    assert_eq!(sub.run, RunEffect::End);
    assert_eq!(sub.result["has_code"], true);
    f.app
        .finish_run(
            run.run,
            "Completed",
            3,
            &UsageSummary {
                tokens_in: 100,
                tokens_out: 20,
                ..Default::default()
            },
        )
        .unwrap();
    let t = ticket(&f, "AGN-1");
    assert_eq!(t.status, TicketStatus::Resolved);
    assert!(t.has_code && !t.approved && !t.merged);
    assert_eq!(t.usage.tokens_in, 100);
    assert!(
        !root.join("deliverables/hello.txt").exists(),
        "main untouched before merge"
    );

    // Self-review approve -> merge queue -> merged, still Resolved (merge never closes).
    let rev = f
        .app
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
    assert_eq!(rev.result["merged"], true);
    assert!(rev.result["next"]
        .as_str()
        .unwrap()
        .contains("close_ticket"));
    let t = ticket(&f, "AGN-1");
    assert!(t.approved && t.merged);
    assert_eq!(t.status, TicketStatus::Resolved);
    assert_eq!(
        std::fs::read_to_string(root.join("deliverables/hello.txt")).unwrap(),
        "hello\n"
    );

    let close_ctx = ctx(&f, lead);
    let closed = f
        .app
        .apply(
            &close_ctx,
            Intent::CloseTicket(CloseTicket {
                ticket_id: k.clone(),
                reason: "All AC met, merged".into(),
            }),
        )
        .unwrap();
    assert_eq!(ticket(&f, "AGN-1").status, TicketStatus::Closed);
    assert!(!wt.exists(), "worktree removed on close");
    let repo = f.app.repo(&f.project);
    assert!(
        !repo.branch_exists("agenium/AGN-1").unwrap(),
        "merged branch deleted"
    );
    let ws = f
        .app
        .store()
        .read(|c| workspaces::get(c, t.id))
        .unwrap()
        .unwrap();
    assert!(ws.removed && ws.merged_sha.is_some());
    // Idempotent replay of the same intent: same result, nothing new.
    let last = f.app.store().read(events::last_seq).unwrap();
    let again = f
        .app
        .apply(
            &close_ctx,
            Intent::CloseTicket(CloseTicket {
                ticket_id: k.clone(),
                reason: "All AC met, merged".into(),
            }),
        )
        .unwrap();
    assert_eq!(again.result, closed.result);
    assert_eq!(f.app.store().read(events::last_seq).unwrap(), last);

    let kinds: Vec<String> = f
        .app
        .store()
        .read(|c| events::since(c, 0, 10_000))
        .unwrap()
        .into_iter()
        .map(|e| e.kind)
        .collect();
    for k in [
        "ProjectCreated",
        "SpecApproved",
        "PlanApproved",
        "TicketCreated",
        "TicketAssigned",
        "RunStarted",
        "TicketMerged",
        "PlanItemDone",
        "VcsOpDone",
    ] {
        assert!(kinds.iter().any(|x| x == k), "missing event {k}");
    }
    assert!(f.app.store().read(ops::pending).unwrap().is_empty());
    // The commit carries the trailers that reconcile relies on.
    let log = repo
        .git()
        .check(root, ["log", "-1", "--format=%B", "main"])
        .unwrap();
    assert!(
        log.contains("Ticket: AGN-1") && log.contains("Agenium-Intent: "),
        "{log}"
    );
}

#[test]
fn research_ticket_without_code_closes_after_review() {
    let f = fixture();
    let a = solo_with_plan(&f, &[("PI-1", &[])]);
    let k = started(&f, a.id, "PI-1", "Look something up", &[], &[]);
    f.app.start_run(f.project.id, &k, "mock", "m").unwrap();
    f.app
        .apply(
            &ctx(&f, Actor::Worker(a.id)),
            Intent::SubmitReport(report(k.as_str())),
        )
        .unwrap();
    let t = ticket(&f, k.as_str());
    assert!(!t.has_code);
    let lead = Actor::Lead(a.id);
    let close = |f: &Fixture| {
        f.app.apply(
            &ctx(f, lead),
            Intent::CloseTicket(CloseTicket {
                ticket_id: k.clone(),
                reason: "Answer recorded".into(),
            }),
        )
    };
    assert!(
        matches!(close(&f), Err(OrchError::Invalid(_))),
        "must be reviewed first"
    );
    let r = f
        .app
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
    assert_eq!(r.result["merged"], false);
    assert!(r.result["next"]
        .as_str()
        .unwrap()
        .starts_with("no code to merge"));
    close(&f).unwrap();
    assert_eq!(ticket(&f, k.as_str()).status, TicketStatus::Closed);
}

#[test]
fn blocking_question_parks_and_answer_resumes() {
    let f = fixture();
    let a = solo_with_plan(&f, &[("PI-1", &[])]);
    let k = started(&f, a.id, "PI-1", "Ask first", &[], &[]);
    let run = f.app.start_run(f.project.id, &k, "mock", "m").unwrap();
    let q = Question {
        ticket_id: k.clone(),
        category: QuestionCategory::Requirement,
        question: "Which greeting language should be used?".into(),
        context: String::new(),
        options: vec![
            OptionProposal {
                label: "English".into(),
            },
            OptionProposal {
                label: "Vietnamese".into(),
            },
        ],
        recommendation: Some(0),
        blocking: true,
        urgency: Urgency::Normal,
    };
    let out = f
        .app
        .apply(&ctx(&f, Actor::Worker(a.id)), Intent::AskQuestion(q))
        .unwrap();
    assert_eq!(out.run, RunEffect::Park);
    assert_eq!(
        out.result["routed_to"], "user",
        "a Solo agent's questions go to the user"
    );
    f.app
        .finish_run(run.run, "Parked", 2, &UsageSummary::default())
        .unwrap();
    assert_eq!(ticket(&f, k.as_str()).status, TicketStatus::Pending);
    assert_eq!(f.app.agent(a.id).unwrap().status, AgentStatus::Waiting);
    assert!(f.app.ready_tickets(f.project.id).unwrap().is_empty());

    let qid: CommentId = serde_json::from_value(out.result["comment_id"].clone()).unwrap();
    // Answering with an out-of-range option is rejected.
    assert!(f
        .app
        .user_answer(f.project.id, &k, qid, "x", Some(5))
        .is_err());
    f.app
        .user_answer(f.project.id, &k, qid, "English", Some(0))
        .unwrap();
    assert_eq!(ticket(&f, k.as_str()).status, TicketStatus::InProgress);
    assert_eq!(f.app.agent(a.id).unwrap().status, AgentStatus::Idle);
    let ready = f.app.ready_tickets(f.project.id).unwrap();
    assert!(
        ready[0].continuation,
        "continuation run picks the conversation up"
    );
    let cs = f
        .app
        .store()
        .read(|c| comments::for_ticket(c, ticket(&f, k.as_str()).id))
        .unwrap();
    assert!(cs
        .iter()
        .any(|c| c.kind == CommentKind::Answer && c.reply_to == Some(qid)));
    assert!(
        f.app
            .user_answer(f.project.id, &k, qid, "again", None)
            .is_err(),
        "answered once"
    );
}

#[test]
fn dependencies_unlock_on_approve_and_merge_not_close() {
    let f = fixture();
    let a = solo_with_plan(&f, &[("PI-1", &[]), ("PI-2", &["PI-1"])]);
    let lead = Actor::Lead(a.id);
    let k1 = started(&f, a.id, "PI-1", "Base", &[], &["src/a/**"]);
    // Overlapping scope without a dependency is refused (decision 18).
    let e = f
        .app
        .apply(
            &ctx(&f, lead),
            Intent::CreateTicket(new_ticket("PI-2", "Overlap", &[], &["src/**"])),
        )
        .unwrap_err();
    assert!(e.to_string().contains("overlap"), "{e}");
    let k2 = started(&f, a.id, "PI-2", "Builds on base", &["AGN-1"], &["src/**"]);
    let ready: Vec<_> = f
        .app
        .ready_tickets(f.project.id)
        .unwrap()
        .into_iter()
        .map(|r| r.ticket.key)
        .collect();
    assert_eq!(ready, vec![k1.clone()], "AGN-2 waits for AGN-1");

    f.app.start_run(f.project.id, &k1, "mock", "m").unwrap();
    write(&worktree(&f, "AGN-1"), "src/a/x.txt", "x");
    f.app
        .apply(
            &ctx(&f, Actor::Worker(a.id)),
            Intent::SubmitReport(report("AGN-1")),
        )
        .unwrap();
    let r = f
        .app
        .store()
        .read(agenium_store::core::runs::unfinished)
        .unwrap();
    f.app
        .finish_run(r[0].id, "Completed", 1, &UsageSummary::default())
        .unwrap();
    assert!(
        f.app.ready_tickets(f.project.id).unwrap().is_empty(),
        "Resolved but not approved: still blocked"
    );
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
    let ready: Vec<_> = f
        .app
        .ready_tickets(f.project.id)
        .unwrap()
        .into_iter()
        .map(|r| r.ticket.key)
        .collect();
    assert_eq!(
        ready,
        vec![k2],
        "approved + merged unlocks the dependent before close_ticket"
    );
    assert_eq!(ticket(&f, "AGN-1").status, TicketStatus::Resolved);
}

#[test]
fn reopen_flow_and_escalation_after_three() {
    let f = fixture();
    let a = solo_with_plan(&f, &[("PI-1", &[])]);
    let (lead, worker) = (Actor::Lead(a.id), Actor::Worker(a.id));
    let k = started(&f, a.id, "PI-1", "Iterate", &[], &[]);
    for round in 1..=4u32 {
        let run = f.app.start_run(f.project.id, &k, "mock", "m").unwrap();
        write(
            &worktree(&f, "AGN-1"),
            "deliverables/n.txt",
            &round.to_string(),
        );
        f.app
            .apply(&ctx(&f, worker), Intent::SubmitReport(report("AGN-1")))
            .unwrap();
        f.app
            .finish_run(run.run, "Completed", 1, &UsageSummary::default())
            .unwrap();
        // Reopen without feedback is refused.
        let bad = ReviewDecision {
            ticket_id: k.clone(),
            decision: ReviewVerdict::Reopen,
            ac_feedback: vec![],
            required_changes: vec![],
        };
        assert!(Intent::parse(
            "review_ticket",
            serde_json::to_value(&bad).unwrap(),
            RoleKind::Lead,
            true
        )
        .is_err());
        let fb = ReviewDecision {
            ticket_id: k.clone(),
            decision: ReviewVerdict::Reopen,
            ac_feedback: vec![AcFeedback {
                ac_id: "AC-1".into(),
                feedback: "not yet".into(),
            }],
            required_changes: vec![],
        };
        f.app
            .apply(&ctx(&f, lead), Intent::ReviewTicket(fb))
            .unwrap();
        let t = ticket(&f, "AGN-1");
        assert_eq!(
            (t.status, t.reopen_count, t.escalated),
            (TicketStatus::Reopened, round, round > 3)
        );
    }
    let needs: Vec<_> = f
        .app
        .store()
        .read(|c| events::recent_of_kind(c, "TicketEscalated", 10))
        .unwrap();
    assert_eq!(needs.len(), 1, "escalated exactly once");
}

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
