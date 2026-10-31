//! M0-10: crash points of the git side effects (spike S4's injection points, re-implemented): a fault at each
//! point leaves a journaled-but-unfinished operation; reopening the app (reconcile) must heal it without losing
//! committed work and without manual git surgery.

mod common;

use std::path::Path;

use agenium_core::forms::*;
use agenium_core::*;
use agenium_store::core::{ops, runs, workspaces};
use common::*;

fn inject(f: &Fixture, point: &str) {
    f.app.faults().lock().unwrap().push(point.into());
}

/// Drops the app and opens it again on the same root (process restart).
fn restart(f: Fixture) -> (Fixture, Vec<String>) {
    let Fixture {
        _tmp,
        root,
        keyring,
        app,
        project,
    } = f;
    drop(app);
    let (app, report) = agenium_orchestrator::App::open(
        &root,
        agenium_orchestrator::AppOptions {
            keyring: keyring.clone(),
            checks: std::sync::Arc::new(Checks(None)),
        },
    )
    .unwrap();
    (
        Fixture {
            _tmp,
            root,
            keyring,
            app,
            project,
        },
        report.actions,
    )
}

fn pending(f: &Fixture) -> usize {
    f.app.store().read(ops::pending).unwrap().len()
}

#[test]
fn crash_points_heal_on_restart() {
    for point in ["before:create_worktree", "after:create_worktree"] {
        let f = fixture();
        let a = solo_with_plan(&f, &[("PI-1", &[])]);
        inject(&f, point);
        let _k = started(&f, a.id, "PI-1", "t", &[], &[]);
        assert_eq!(pending(&f), 1, "{point}: op left pending");
        let (f, actions) = restart(f);
        assert!(
            actions
                .iter()
                .any(|x| x.contains("replayed create_worktree")),
            "{point}: {actions:?}"
        );
        assert_eq!(pending(&f), 0);
        assert!(
            worktree(&f, "AGN-1").join("agenium.toml").is_file(),
            "{point}: worktree exists"
        );
        let ws = f
            .app
            .store()
            .read(|c| workspaces::get(c, ticket(&f, "AGN-1").id))
            .unwrap()
            .unwrap();
        assert!(!ws.removed);
    }

    for point in ["before:commit", "after:commit"] {
        let f = fixture();
        let a = solo_with_plan(&f, &[("PI-1", &[])]);
        let k = started(&f, a.id, "PI-1", "t", &[], &[]);
        f.app.start_run(f.project.id, &k, "mock", "m").unwrap();
        write(&worktree(&f, "AGN-1"), "deliverables/a.txt", "work");
        inject(&f, point);
        assert!(f
            .app
            .apply(
                &ctx(&f, Actor::Worker(a.id)),
                Intent::SubmitReport(report("AGN-1"))
            )
            .is_err());
        let (f, actions) = restart(f);
        assert_eq!(pending(&f), 0, "{point}: {actions:?}");
        let t = ticket(&f, "AGN-1");
        assert_eq!(
            t.status,
            TicketStatus::InProgress,
            "{point}: the report was not accepted, the agent resubmits"
        );
        // The interrupted run was closed; the agent is free for a continuation run.
        assert!(f.app.store().read(runs::unfinished).unwrap().is_empty());
        assert_eq!(f.app.agent(a.id).unwrap().status, AgentStatus::Idle);
        // Work is never lost: either committed (after) or still in the tree (before).
        let wt = worktree(&f, "AGN-1");
        assert_eq!(
            std::fs::read_to_string(wt.join("deliverables/a.txt")).unwrap(),
            "work"
        );
        let run = f.app.start_run(f.project.id, &k, "mock", "m").unwrap();
        assert!(run.continuation);
        f.app
            .apply(
                &ctx(&f, Actor::Worker(a.id)),
                Intent::SubmitReport(report("AGN-1")),
            )
            .unwrap();
        assert!(ticket(&f, "AGN-1").has_code, "{point}");
    }

    for point in ["before:merge", "after:merge"] {
        let f = fixture();
        let a = solo_with_plan(&f, &[("PI-1", &[])]);
        let k = started(&f, a.id, "PI-1", "t", &[], &[]);
        f.app.start_run(f.project.id, &k, "mock", "m").unwrap();
        write(&worktree(&f, "AGN-1"), "deliverables/a.txt", "work");
        f.app
            .apply(
                &ctx(&f, Actor::Worker(a.id)),
                Intent::SubmitReport(report("AGN-1")),
            )
            .unwrap();
        inject(&f, point);
        // The approval commits; the merge op fails and stays journaled.
        f.app
            .apply(
                &ctx(&f, Actor::Lead(a.id)),
                Intent::ReviewTicket(ReviewDecision {
                    ticket_id: k.clone(),
                    decision: ReviewVerdict::Approve,
                    ac_feedback: vec![],
                    required_changes: vec![],
                }),
            )
            .unwrap();
        assert!(!ticket(&f, "AGN-1").merged);
        assert_eq!(pending(&f), 1);
        let (f, actions) = restart(f);
        assert!(
            actions.iter().any(|x| x.contains("replayed merge")),
            "{actions:?}"
        );
        let t = ticket(&f, "AGN-1");
        assert!(
            t.merged && t.approved,
            "{point}: merge completed by reconcile"
        );
        assert!(Path::new(&f.project.path)
            .join("deliverables/a.txt")
            .is_file());
        let repo = f.app.repo(&f.project);
        let n: usize = repo
            .git()
            .check(Path::new(&f.project.path), ["rev-list", "--count", "main"])
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert_eq!(
            n, 3,
            "{point}: merged exactly once (init + layout + ticket)"
        );
    }

    for point in ["before:remove_worktree", "after:remove_worktree"] {
        let f = fixture();
        let a = solo_with_plan(&f, &[("PI-1", &[])]);
        let k = started(&f, a.id, "PI-1", "t", &[], &[]);
        f.app.start_run(f.project.id, &k, "mock", "m").unwrap();
        write(&worktree(&f, "AGN-1"), "deliverables/a.txt", "work");
        f.app
            .apply(
                &ctx(&f, Actor::Worker(a.id)),
                Intent::SubmitReport(report("AGN-1")),
            )
            .unwrap();
        f.app
            .apply(
                &ctx(&f, Actor::Lead(a.id)),
                Intent::ReviewTicket(ReviewDecision {
                    ticket_id: k.clone(),
                    decision: ReviewVerdict::Approve,
                    ac_feedback: vec![],
                    required_changes: vec![],
                }),
            )
            .unwrap();
        inject(&f, point);
        f.app
            .apply(
                &ctx(&f, Actor::Lead(a.id)),
                Intent::CloseTicket(CloseTicket {
                    ticket_id: k.clone(),
                    reason: "done and merged".into(),
                }),
            )
            .unwrap();
        assert_eq!(ticket(&f, "AGN-1").status, TicketStatus::Closed);
        let (f, actions) = restart(f);
        assert_eq!(pending(&f), 0, "{actions:?}");
        assert!(!worktree(&f, "AGN-1").exists(), "{point}: worktree removed");
        assert!(!f
            .app
            .repo(&f.project)
            .branch_exists("agenium/AGN-1")
            .unwrap());
    }
}

#[test]
fn missing_and_orphan_worktrees_are_repaired() {
    let f = fixture();
    let a = solo_with_plan(&f, &[("PI-1", &[])]);
    started(&f, a.id, "PI-1", "t", &[], &[]);
    // The user deletes the ticket worktree by hand and leaves a stray directory behind.
    let repo = f.app.repo(&f.project);
    repo.remove_worktree("AGN-1").unwrap();
    std::fs::create_dir_all(worktree(&f, "AGN-99")).unwrap();
    let (f, actions) = restart(f);
    assert!(
        actions
            .iter()
            .any(|x| x.contains("restored missing worktree of AGN-1")),
        "{actions:?}"
    );
    assert!(
        actions.iter().any(|x| x.contains("orphan worktree AGN-99")),
        "{actions:?}"
    );
    assert!(worktree(&f, "AGN-1").is_dir() && !worktree(&f, "AGN-99").exists());
    // A clean restart has nothing to do.
    let (_f, actions) = restart(f);
    assert!(actions.is_empty(), "{actions:?}");
}

/// Audit M10 (2026-10-31): a worktree that cannot be removed at start (a file still held, on Windows) kept
/// Agenium from opening. It is reported and tried again at the next start.
#[test]
fn a_worktree_that_cannot_be_removed_does_not_keep_the_app_from_opening() {
    let f = fixture();
    let stray = worktree(&f, "AGN-98");
    std::fs::create_dir_all(stray.join("locked")).unwrap();
    std::fs::write(stray.join("locked/held.txt"), "x").unwrap();
    // Held open on Windows; a folder without write permission on Unix: either way the removal fails.
    // (Rust opens files sharing delete access by default: shared with nothing, as a program holding it would.)
    #[cfg(windows)]
    let held = {
        use std::os::windows::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(stray.join("locked/held.txt"))
            .unwrap()
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(stray.join("locked"), std::fs::Permissions::from_mode(0o500))
            .unwrap();
    }
    let (f, actions) = restart(f);
    assert!(
        actions
            .iter()
            .any(|x| x.contains("orphan worktree AGN-98 not removed yet")),
        "{actions:?}"
    );
    #[cfg(windows)]
    drop(held);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(stray.join("locked"), std::fs::Permissions::from_mode(0o700))
            .unwrap();
    }
    let (f, actions) = restart(f);
    assert!(
        actions
            .iter()
            .any(|x| x.contains("removed orphan worktree AGN-98")),
        "{actions:?}"
    );
    assert!(!worktree(&f, "AGN-98").exists());
}

#[test]
fn failed_merge_is_retried_without_a_restart() {
    let f = fixture();
    let a = solo_with_plan(&f, &[("PI-1", &[])]);
    let k = started(&f, a.id, "PI-1", "t", &[], &[]);
    f.app.start_run(f.project.id, &k, "mock", "m").unwrap();
    write(&worktree(&f, "AGN-1"), "deliverables/a.txt", "work");
    f.app
        .apply(
            &ctx(&f, Actor::Worker(a.id)),
            Intent::SubmitReport(report("AGN-1")),
        )
        .unwrap();
    // A transient failure (e.g. a Windows file lock held by a CLI process that is still running).
    inject(&f, "before:merge");
    let out = f
        .app
        .apply(
            &ctx(&f, Actor::Lead(a.id)),
            Intent::ReviewTicket(ReviewDecision {
                ticket_id: k.clone(),
                decision: ReviewVerdict::Approve,
                ac_feedback: vec![],
                required_changes: vec![],
            }),
        )
        .unwrap();
    assert_eq!(out.result["merged"], false);
    assert!(out.result["next"].as_str().unwrap().contains("retries it"));
    assert_eq!(pending(&f), 1);
    // The scheduler replays it at its next round.
    assert_eq!(f.app.replay_pending_ops(f.project.id).unwrap(), 1);
    assert_eq!(pending(&f), 0);
    let t = ticket(&f, "AGN-1");
    assert!(t.merged && t.approved && t.status == TicketStatus::Resolved);
}

/// A merge that keeps failing (git itself, not a conflict or a failed check) is not re-run at every tick forever in
/// silence: after a few failed tries in a row the ticket says so and the user is asked to look (once), and the tries
/// back off. It still merges once git works again.
#[test]
fn a_merge_that_keeps_failing_tells_the_user_once_and_backs_off() {
    let f = fixture();
    let a = solo_with_plan(&f, &[("PI-1", &[])]);
    let k = started(&f, a.id, "PI-1", "t", &[], &[]);
    f.app.start_run(f.project.id, &k, "mock", "m").unwrap();
    write(&worktree(&f, "AGN-1"), "deliverables/a.txt", "work");
    f.app
        .apply(
            &ctx(&f, Actor::Worker(a.id)),
            Intent::SubmitReport(report("AGN-1")),
        )
        .unwrap();
    let n = agenium_orchestrator::effects::OP_FAILURES_BEFORE_USER;
    // The try at approval, then `n` replays, all fail.
    for _ in 0..=n {
        inject(&f, "before:merge");
    }
    f.app
        .apply(
            &ctx(&f, Actor::Lead(a.id)),
            Intent::ReviewTicket(ReviewDecision {
                ticket_id: k.clone(),
                decision: ReviewVerdict::Approve,
                ac_feedback: vec![],
                required_changes: vec![],
            }),
        )
        .unwrap();
    let notes = || {
        f.app
            .store()
            .read(|c| agenium_store::core::comments::for_ticket(c, ticket(&f, "AGN-1").id))
            .unwrap()
            .iter()
            .filter(|c| matches!(&c.body, CommentBody::System { text } if text.contains("keeps failing")))
            .count()
    };
    for i in 1..=n {
        assert_eq!(f.app.replay_pending_ops(f.project.id).unwrap(), 0);
        assert_eq!(notes(), usize::from(i == n), "after {i} failed replays");
    }
    let asked = f
        .app
        .store()
        .read(|c| agenium_store::core::events::recent_of_kind(c, "NeedsUser", 50))
        .unwrap()
        .into_iter()
        .filter(|e| {
            e.payload["reason"]
                .as_str()
                .is_some_and(|r| r.contains("keeps failing"))
        })
        .count();
    assert_eq!(asked, 1);
    // Backing off: the next two replays do not try it.
    for _ in 0..2 {
        assert_eq!(f.app.replay_pending_ops(f.project.id).unwrap(), 0);
        assert_eq!(pending(&f), 1);
    }
    assert_eq!(f.app.replay_pending_ops(f.project.id).unwrap(), 1);
    assert!(ticket(&f, "AGN-1").merged);
    assert_eq!(notes(), 1, "told once");
}

/// ADR-147: a run that dies with the process leaves its edits in the worktree; they are the agent's work, not a change
/// made outside Agenium. The run after the restart goes on from them; the guard undoes nothing.
#[test]
fn an_interrupted_runs_edits_survive_the_restart() {
    let f = fixture();
    let a = solo_with_plan(&f, &[("PI-1", &[])]);
    let k = started(&f, a.id, "PI-1", "t", &[], &[]);
    let wt = worktree(&f, k.as_str());
    // Run A ends normally: its state is the guard's reference.
    let ra = f.app.start_run(f.project.id, &k, "mock", "m").unwrap();
    write(&wt, "notes.md", "from run A\n");
    f.app
        .finish_run(ra.run, "Stopped:max_steps", 1, &UsageSummary::default())
        .unwrap();
    // Run B edits more, then the process dies with B open.
    let rb = f.app.start_run(f.project.id, &k, "mock", "m").unwrap();
    write(&wt, "notes.md", "from run A\nand run B\n");
    write(&wt, "b.txt", "B's new file\n");
    let (f, actions) = restart(f);
    assert!(
        actions.iter().any(|x| x.contains("interrupted run")),
        "{actions:?}"
    );
    let b = f.app.store().read(|c| runs::get(c, rb.run)).unwrap();
    assert_eq!(b.outcome.as_deref(), Some("Interrupted"));
    assert!(
        f.app.guard_worktree(f.project.id, &k).unwrap().is_none(),
        "B's work is not undone"
    );
    let wt = worktree(&f, k.as_str());
    assert_eq!(
        std::fs::read_to_string(wt.join("notes.md")).unwrap(),
        "from run A\nand run B\n"
    );
    assert!(wt.join("b.txt").is_file());
    // A change made by hand after the restart is still undone.
    write(&wt, "notes.md", "by hand\n");
    assert!(f.app.guard_worktree(f.project.id, &k).unwrap().is_some());
    assert_eq!(
        std::fs::read_to_string(wt.join("notes.md")).unwrap(),
        "from run A\nand run B\n"
    );
}

/// Audit M9 (2026-10-31): a message to the Lead was marked delivered when a chat turn took it, so a process that died
/// during the turn lost it (neither queued nor in the conversation). It is delivered only at the turn's end; one the
/// process died with goes back in the queue at the next start.
#[test]
fn a_message_taken_by_a_turn_the_process_died_in_is_queued_again() {
    let f = fixture();
    f.app.post_lead_message(f.project.id, "deploy it").unwrap();
    let taken = f.app.take_lead_messages(f.project.id).unwrap();
    assert_eq!(taken.messages.len(), 1);
    assert!(f
        .app
        .undelivered_lead_messages(f.project.id)
        .unwrap()
        .is_empty());
    // The process dies during the turn: nothing puts it back, nothing confirms it.
    std::mem::forget(taken);
    let (f, actions) = restart(f);
    assert!(
        actions.iter().any(|x| x.contains("back in the queue")),
        "{actions:?}"
    );
    assert_eq!(
        f.app.undelivered_lead_messages(f.project.id).unwrap().len(),
        1
    );
    // A turn that reaches its end delivers it for good.
    f.app.take_lead_messages(f.project.id).unwrap().keep();
    let (f, actions) = restart(f);
    assert!(
        !actions.iter().any(|x| x.contains("back in the queue")),
        "{actions:?}"
    );
    assert!(f
        .app
        .undelivered_lead_messages(f.project.id)
        .unwrap()
        .is_empty());
}
