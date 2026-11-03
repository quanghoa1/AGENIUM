//! What a ticket may touch and when: aggregates, cancellations, Solo requests, files, pending merges.

use super::*;

/// The user approves an item's scope with the plan: a ticket may not reach past it. A Lead on 2026-11-03 created a
/// README.md ticket for a notes-only item (QS-4) and changed a file the user had excluded; the plan item's scope now
/// refuses it, and a ticket that names no scope takes the item's rather than the whole tree.
#[test]
fn a_ticket_stays_inside_its_plan_items_scope() {
    let f = fixture();
    let a = solo_with_scoped_plan(&f, &[("PI-1", &[], &["notes/**"])]);
    let lead = Actor::Lead(a.id);
    let create = |deps: &[&str], scope: &[&str]| {
        f.app.apply(
            &ctx(&f, lead),
            Intent::CreateTicket(new_ticket("PI-1", "Document it", deps, scope)),
        )
    };
    let outside = create(&[], &["README.md"]);
    assert!(
        matches!(outside, Err(OrchError::Invalid(_))),
        "README.md is outside notes/**"
    );
    assert_eq!(
        f.app.tickets(f.project.id).unwrap().len(),
        0,
        "nothing was created"
    );

    let ok = create(&[], &["notes/HOWTO.md"]).unwrap();
    assert_eq!(ok.result["ticket_id"], "AGN-1");

    // No scope named: the item's applies (and the open ticket it overlaps is waited for).
    let inherited = create(&["AGN-1"], &[]).unwrap();
    assert_eq!(inherited.result["ticket_id"], "AGN-2");
    assert_eq!(
        ticket(&f, "AGN-2").scope_paths,
        vec!["notes/**".to_string()]
    );
}

/// An aggregate links two or more existing tickets, and its ticket commits with its links: an empty list panicked
/// inside a store read, and a child key that does not exist left an orphan aggregate ticket behind.
#[test]
fn an_aggregate_commits_with_its_links_or_not_at_all() {
    let f = fixture();
    let a = solo_with_plan(&f, &[("PI-1", &[])]);
    let lead = Actor::Lead(a.id);
    for title in ["One", "Two"] {
        f.app
            .apply(
                &ctx(&f, lead),
                Intent::CreateTicket(new_ticket("PI-1", title, &[], &[])),
            )
            .unwrap();
    }
    let aggregate = |children: &[&str]| {
        f.app.apply(
            &ctx(&f, lead),
            Intent::CreateAggregate(agenium_core::protocol::CreateAggregate {
                children: children.iter().map(|k| key(k)).collect(),
                kind: "attack chain".into(),
                title: "Chain".into(),
                summary: String::new(),
            }),
        )
    };
    assert!(matches!(aggregate(&[]), Err(OrchError::Invalid(_))));
    assert!(matches!(
        aggregate(&["AGN-1", "AGN-9"]),
        Err(OrchError::NotFound(_))
    ));
    assert_eq!(
        f.app.tickets(f.project.id).unwrap().len(),
        2,
        "no orphan aggregate"
    );
    let out = aggregate(&["AGN-1", "AGN-2"]).unwrap();
    assert_eq!(out.result["ticket_id"], "AGN-3");
    let links = f
        .app
        .store()
        .read(|c| agenium_store::core::tickets::links_of(c, ticket(&f, "AGN-3").id))
        .unwrap();
    assert_eq!(
        links.iter().filter(|l| l.kind == "aggregates").count(),
        2,
        "{links:?}"
    );
}

/// More work on a ticket is a new ticket that names it (`follows`): the two are linked `follow_up_of` as the new one
/// is created, and a follow-up of a ticket that does not exist creates nothing.
#[test]
fn a_follow_up_ticket_is_linked_to_the_ticket_it_goes_on_from() {
    let f = fixture();
    let a = solo_with_plan(&f, &[("PI-1", &[])]);
    let lead = Actor::Lead(a.id);
    let create = |title: &str, follows: Option<&str>| {
        f.app.apply(
            &ctx(&f, lead),
            Intent::CreateTicket(NewTicket {
                follows: follows.map(key),
                ..new_ticket("PI-1", title, &[], &[])
            }),
        )
    };
    create("First", None).unwrap();
    assert!(matches!(
        create("Lost", Some("AGN-9")),
        Err(OrchError::NotFound(_))
    ));
    assert_eq!(f.app.tickets(f.project.id).unwrap().len(), 1);
    let out = create("More", Some("AGN-1")).unwrap();
    assert_eq!(out.result["ticket_id"], "AGN-2");
    let links = f
        .app
        .store()
        .read(|c| agenium_store::core::tickets::links_of(c, ticket(&f, "AGN-1").id))
        .unwrap();
    assert!(
        links.iter().any(|l| l.kind == "follow_up_of"
            && l.src == ticket(&f, "AGN-2").id
            && l.dst == ticket(&f, "AGN-1").id),
        "{links:?}"
    );
}

/// A cancelled ticket is never done: a new ticket cannot wait for it, and the tickets already waiting for it go to
/// the Lead (they could never start, and nothing said so).
#[test]
fn tickets_waiting_for_a_cancelled_one_go_to_the_lead() {
    use agenium_orchestrator::inbox::InboxKind;
    let f = fixture();
    let a = solo_with_plan(&f, &[("PI-1", &[]), ("PI-2", &["PI-1"])]);
    let lead = Actor::Lead(a.id);
    let create = |item: &str, deps: &[&str]| {
        f.app.apply(
            &ctx(&f, lead),
            Intent::CreateTicket(new_ticket(item, "Work", deps, &[])),
        )
    };
    create("PI-1", &[]).unwrap();
    create("PI-2", &["AGN-1"]).unwrap();
    let blocked = || -> Vec<Option<TicketKey>> {
        f.app
            .lead_inbox(f.project.id)
            .unwrap()
            .into_iter()
            .filter(|i| i.kind == InboxKind::Blocked)
            .map(|i| i.ticket)
            .collect()
    };
    assert!(blocked().is_empty());
    let out = f.app.apply(&ctx(&f, lead), cancel("AGN-1")).unwrap();
    assert_eq!(
        out.result["blocked_dependents"],
        serde_json::json!(["AGN-2"])
    );
    assert_eq!(blocked(), [Some(key("AGN-2"))]);
    let e = create("PI-2", &["AGN-1"]).unwrap_err();
    assert!(e.to_string().contains("AGN-1 is cancelled"), "{e}");
    // Dealt with (cancelled; a new ticket would replace it): the item leaves the inbox.
    f.app.apply(&ctx(&f, lead), cancel("AGN-2")).unwrap();
    assert!(blocked().is_empty());
}

/// A Solo agent's questions and permission requests go to the user: the agent, its own Lead, cannot decide them.
#[test]
fn a_solo_agent_does_not_decide_its_own_requests() {
    let f = fixture();
    let mut m = f.app.manifest(&f.project).unwrap();
    m.network.allowlist = vec!["crates.io".into()];
    m.save(std::path::Path::new(&f.project.path)).unwrap();
    let a = solo_with_plan(&f, &[("PI-1", &[])]);
    let (lead, worker) = (Actor::Lead(a.id), Actor::Worker(a.id));
    let k = started(&f, a.id, "PI-1", "Ask first", &[], &[]);
    f.app.start_run(f.project.id, &k, "mock", "m").unwrap();
    let q = f
        .app
        .apply(
            &ctx(&f, worker),
            Intent::AskQuestion(Question {
                ticket_id: k.clone(),
                category: QuestionCategory::Requirement,
                question: "Which greeting language should be used?".into(),
                context: String::new(),
                options: vec![],
                recommendation: None,
                blocking: true,
                urgency: Urgency::Normal,
            }),
        )
        .unwrap();
    let qid: CommentId = serde_json::from_value(q.result["comment_id"].clone()).unwrap();
    let e = f
        .app
        .apply(
            &ctx(&f, lead),
            Intent::AnswerQuestion(AnswerQuestion {
                ticket_id: k.clone(),
                reply_to: qid,
                answer: "English".into(),
                chosen_option: None,
                escalate: false,
            }),
        )
        .unwrap_err();
    assert!(matches!(e, OrchError::Denied(_)), "{e}");
    let p = f
        .app
        .apply(
            &ctx(&f, worker),
            Intent::RequestPermission(PermissionRequest {
                ticket_id: k.clone(),
                kind: PermissionKind::Network,
                command: Some("cargo add serde".into()),
                domains: vec!["crates.io".into()],
                paths: vec![],
                reason: "cargo add serde".into(),
            }),
        )
        .unwrap();
    assert_eq!(p.result["decided_by"], "user");
    let rid: CommentId = serde_json::from_value(p.result["request_id"].clone()).unwrap();
    let e = f
        .app
        .apply(
            &ctx(&f, lead),
            Intent::ApprovePermission(ApprovePermission {
                ticket_id: k.clone(),
                request_id: rid,
                decision: PermissionDecision::Approve,
                note: None,
            }),
        )
        .unwrap_err();
    assert!(matches!(e, OrchError::Denied(_)), "{e}");
    assert_eq!(ticket(&f, "AGN-1").status, TicketStatus::Pending);
    // The user decides both.
    f.app
        .user_answer(f.project.id, &k, qid, "English", None)
        .unwrap();
    f.app
        .decide_permission(
            f.project.id,
            &k,
            rid,
            PermissionDecision::Approve,
            false,
            None,
        )
        .unwrap();
    assert_eq!(ticket(&f, "AGN-1").status, TicketStatus::InProgress);
}

/// A done item keeps its work across revisions, also when one revision dropped it and a later one brings its id
/// back with new work (that work would count as done by the old closed ticket, and nobody would do it).
#[test]
fn a_dropped_done_item_cannot_come_back_with_new_work() {
    let f = fixture();
    let a = solo_with_plan(&f, &[("PI-1", &[]), ("PI-2", &[])]);
    let lead = Actor::Lead(a.id);
    // PI-1 is done: its ticket (no code) is reviewed and closed.
    let k = started(&f, a.id, "PI-1", "Look something up", &[], &[]);
    f.app.start_run(f.project.id, &k, "mock", "m").unwrap();
    f.app
        .apply(
            &ctx(&f, Actor::Worker(a.id)),
            Intent::SubmitReport(report("AGN-1")),
        )
        .unwrap();
    f.app
        .apply(&ctx(&f, lead), Intent::ReviewTicket(approve(&k)))
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
    let v1 = plan(&[("PI-1", &[]), ("PI-2", &[])]);
    let revise = |base: u32, items: Vec<agenium_core::plan::PlanItem>| {
        f.app.apply(
            &ctx(&f, lead),
            Intent::RevisePlan(agenium_core::plan::PlanRevision {
                base_version: base,
                reason: "re-plan".into(),
                milestones: v1.milestones.clone(),
                items,
            }),
        )
    };
    revise(1, vec![v1.items[1].clone()]).unwrap();
    let mut new_work = v1.items[0].clone();
    new_work.title = "Something else entirely".into();
    let e = revise(2, vec![new_work, v1.items[1].clone()]).unwrap_err();
    assert!(e.to_string().contains("plan item PI-1 is done"), "{e}");
    // Back as it was in v1: the done item returns with its own work.
    revise(2, v1.items.clone()).unwrap();
}

/// ADR-120: two tickets without declared scopes that name the same file do not run in parallel: the later one is
/// refused until it depends on the earlier (it then starts from the earlier one's merged work); naming different
/// files, or declaring different scopes, keeps them parallel.
#[test]
fn tickets_naming_the_same_file_run_one_after_the_other() {
    let f = fixture();
    let a = solo_with_plan(&f, &[("PI-1", &[]), ("PI-2", &[])]);
    let create = |t: NewTicket| {
        f.app
            .apply(&ctx(&f, Actor::Lead(a.id)), Intent::CreateTicket(t))
    };
    create(new_ticket(
        "PI-1",
        "Create deliverables/hello.txt and deliverables/check.py",
        &[],
        &[],
    ))
    .unwrap();
    let e = create(new_ticket(
        "PI-2",
        "Create deliverables/check.py that prints hello.txt",
        &[],
        &[],
    ))
    .unwrap_err()
    .to_string();
    assert!(
        e.contains("open ticket AGN-1 both change deliverables/check.py")
            && e.contains("depends_on"),
        "{e}"
    );
    create(new_ticket(
        "PI-2",
        "Create deliverables/check.py that prints hello.txt",
        &["AGN-1"],
        &[],
    ))
    .unwrap();
    // Another file, or a declared scope apart from the named ones: parallel.
    create(new_ticket("PI-2", "Write deliverables/notes.md", &[], &[])).unwrap();
    create(new_ticket(
        "PI-2",
        "Polish deliverables/check.py",
        &[],
        &["deliverables/style.css"],
    ))
    .unwrap();
}

/// Moving a file out of the write scope deletes it there: a rename into the scope is outside it too (only the new
/// path was checked, and the commit deleted the old one).
#[test]
fn a_rename_out_of_the_scope_is_outside_it() {
    let f = fixture();
    let a = solo_with_plan(&f, &[("PI-1", &[])]);
    let k = started(&f, a.id, "PI-1", "Write hello", &[], &["deliverables/**"]);
    f.app.start_run(f.project.id, &k, "mock", "m").unwrap();
    let wt = worktree(&f, "AGN-1");
    std::fs::create_dir_all(wt.join("deliverables")).unwrap();
    agenium_vcs::git::Git::default()
        .check(&wt, ["mv", "agenium.toml", "deliverables/agenium.toml"])
        .unwrap();
    let e = f
        .app
        .apply(
            &ctx(&f, Actor::Worker(a.id)),
            Intent::SubmitReport(report("AGN-1")),
        )
        .unwrap_err()
        .to_string();
    assert!(e.contains("your role: agenium.toml."), "{e}");
    assert_eq!(ticket(&f, "AGN-1").status, TicketStatus::InProgress);
}

/// A branch rebased on main (by its worker after a conflict, or by the merge queue before a failed check) holds
/// the files other tickets merged meanwhile; they are not this ticket's work. The scope check and the review diff
/// start from the merge-base with main, not from the base the worktree was made on.
#[test]
fn a_rebased_branch_is_measured_from_main() {
    let f = fixture();
    let a = solo_with_plan(&f, &[("PI-1", &[]), ("PI-2", &[])]);
    let (lead, worker) = (Actor::Lead(a.id), Actor::Worker(a.id));
    let k1 = started(&f, a.id, "PI-1", "First", &[], &["src/a/**"]);
    let k2 = started(&f, a.id, "PI-2", "Second", &[], &["src/b/**"]);
    let r1 = f.app.start_run(f.project.id, &k1, "mock", "m").unwrap();
    write(&worktree(&f, "AGN-1"), "src/a/x.txt", "x");
    f.app
        .apply(&ctx(&f, worker), Intent::SubmitReport(report("AGN-1")))
        .unwrap();
    f.app
        .finish_run(r1.run, "Completed", 1, &UsageSummary::default())
        .unwrap();
    f.app
        .apply(&ctx(&f, lead), Intent::ReviewTicket(approve(&k1)))
        .unwrap();
    assert!(ticket(&f, "AGN-1").merged);

    // The second ticket's worker commits its work and rebases it on the new main.
    f.app.start_run(f.project.id, &k2, "mock", "m").unwrap();
    let wt = worktree(&f, "AGN-2");
    write(&wt, "src/b/y.txt", "y");
    let vcs = agenium_vcs::git::Git::default();
    let as_worker = |args: &[&str]| {
        let mut v = vec![
            "-c",
            "user.name=worker",
            "-c",
            "user.email=worker@example.com",
        ];
        v.extend_from_slice(args);
        vcs.check(&wt, v).unwrap();
    };
    as_worker(&["add", "-A"]);
    as_worker(&["commit", "-q", "-m", "wip"]);
    as_worker(&["rebase", "-q", "main"]);
    assert!(wt.join("src/a/x.txt").is_file(), "rebased on main");
    let out = f
        .app
        .apply(&ctx(&f, worker), Intent::SubmitReport(report("AGN-2")))
        .unwrap();
    assert_eq!(out.result["has_code"], true);
    // The recorded base follows: the review diff shows this ticket's work only.
    let ws = f
        .app
        .store()
        .read(|c| workspaces::get(c, ticket(&f, "AGN-2").id))
        .unwrap()
        .unwrap();
    let diff = f.app.repo(&f.project).diff(&wt, &ws.base_sha).unwrap();
    assert!(
        diff.contains("src/b/y.txt") && !diff.contains("src/a/x.txt"),
        "{diff}"
    );
}

/// A merge left pending by a failure runs later, when the ticket may have changed: the Lead's reopen or cancel in
/// the meantime drops it (it merged the work sent back, marking the ticket merged so its rework was never merged, or
/// a cancelled ticket's branch).
#[test]
fn a_pending_merge_never_lands_reopened_or_cancelled_work() {
    let f = fixture();
    let root = std::path::Path::new(&f.project.path);
    let a = solo_with_plan(&f, &[("PI-1", &[]), ("PI-2", &[])]);
    let (lead, worker) = (Actor::Lead(a.id), Actor::Worker(a.id));
    for (item, file) in [
        ("PI-1", "deliverables/one.txt"),
        ("PI-2", "deliverables/two.txt"),
    ] {
        let k = started(&f, a.id, item, "Work", &[], &[]);
        let run = f.app.start_run(f.project.id, &k, "mock", "m").unwrap();
        write(&worktree(&f, k.as_str()), file, "x");
        f.app
            .apply(&ctx(&f, worker), Intent::SubmitReport(report(k.as_str())))
            .unwrap();
        f.app
            .finish_run(run.run, "Completed", 1, &UsageSummary::default())
            .unwrap();
        // A transient failure leaves the merge pending.
        f.app.faults().lock().unwrap().push("before:merge".into());
        f.app
            .apply(&ctx(&f, lead), Intent::ReviewTicket(approve(&k)))
            .unwrap();
        assert!(!ticket(&f, k.as_str()).merged);
    }
    f.app
        .apply(
            &ctx(&f, lead),
            Intent::ReviewTicket(ReviewDecision {
                ticket_id: key("AGN-1"),
                decision: ReviewVerdict::Reopen,
                ac_feedback: vec![AcFeedback {
                    ac_id: "AC-1".into(),
                    feedback: "Greet in English too".into(),
                }],
                required_changes: vec![],
            }),
        )
        .unwrap();
    f.app.apply(&ctx(&f, lead), cancel("AGN-2")).unwrap();
    f.app.replay_pending_ops(f.project.id).unwrap();
    assert!(f.app.store().read(ops::pending).unwrap().is_empty());
    for (k, file) in [
        ("AGN-1", "deliverables/one.txt"),
        ("AGN-2", "deliverables/two.txt"),
    ] {
        assert!(!ticket(&f, k).merged, "{k}");
        assert!(!root.join(file).exists(), "{file} stays out of main");
    }
}

/// Cancelling a ticket while a run works in it keeps the worktree until the run ends (it was removed under the
/// running agent); the run's end removes it and keeps the unmerged branch.
#[test]
fn a_cancelled_tickets_worktree_goes_when_its_run_ends() {
    let f = fixture();
    let a = solo_with_plan(&f, &[("PI-1", &[])]);
    let k = started(&f, a.id, "PI-1", "Work", &[], &[]);
    let run = f.app.start_run(f.project.id, &k, "mock", "m").unwrap();
    let wt = worktree(&f, "AGN-1");
    f.app
        .apply(&ctx(&f, Actor::Lead(a.id)), cancel("AGN-1"))
        .unwrap();
    assert_eq!(ticket(&f, "AGN-1").status, TicketStatus::Cancelled);
    assert!(wt.is_dir(), "the live run keeps its worktree");
    f.app
        .finish_run(run.run, "Cancelled", 1, &UsageSummary::default())
        .unwrap();
    assert!(!wt.exists(), "removed when the run ended");
    assert!(f.app.store().read(ops::pending).unwrap().is_empty());
    assert!(
        f.app
            .repo(&f.project)
            .branch_exists("agenium/AGN-1")
            .unwrap(),
        "the unmerged branch stays"
    );
}
