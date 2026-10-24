use std::path::Path;

use agenium_vcs::*;

const AUTHOR: (&str, &str) = ("worker-1", "agent_x@agenium.local");

fn repo(dir: &Path) -> Repo {
    let r = Repo::open(&dir.join("proj"), "main");
    r.init_local().unwrap();
    r
}

fn write(p: &Path, rel: &str, content: &str) {
    let f = p.join(rel);
    std::fs::create_dir_all(f.parent().unwrap()).unwrap();
    std::fs::write(f, content).unwrap();
}

struct FailCheck;
impl CheckRunner for FailCheck {
    fn run(&self, _: &Path) -> CheckOutcome {
        CheckOutcome {
            ok: false,
            log: "test failed: 1 of 3".into(),
        }
    }
}

#[test]
fn init_is_idempotent_and_excludes_agenium_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let r = repo(tmp.path());
    let first = r.head_sha("main").unwrap().unwrap();
    r.init_local().unwrap();
    assert_eq!(r.head_sha("main").unwrap().unwrap(), first);
    let excl = std::fs::read_to_string(r.root.join(".git/info/exclude")).unwrap();
    assert_eq!(excl.matches(".agenium/").count(), 1);
    r.add_worktree("AGN-1", "main").unwrap();
    assert!(
        r.status(&r.root).unwrap().is_empty(),
        "worktrees under .agenium/ do not dirty main"
    );
}

#[test]
fn worktree_commit_and_secret_blocking() {
    let tmp = tempfile::tempdir().unwrap();
    let r = repo(tmp.path());
    let main_sha = r.head_sha("main").unwrap().unwrap();
    let w = r.add_worktree("AGN-1", "main").unwrap();
    assert_eq!(w.branch, "agenium/AGN-1");
    assert_eq!(w.base_sha, main_sha);
    assert_eq!(r.add_worktree("AGN-1", "main").unwrap(), w, "idempotent");

    assert_eq!(r.commit_all(&w.path, "nothing", AUTHOR, &[]).unwrap(), None);
    write(&w.path, "deliverables/hello.txt", "hello\n");
    let sha = r
        .commit_all(
            &w.path,
            "Add hello",
            AUTHOR,
            &[("Ticket", "AGN-1"), ("Agenium-Intent", "01ABC")],
        )
        .unwrap()
        .unwrap();
    assert!(
        !r.root.join("deliverables/hello.txt").exists(),
        "main is untouched"
    );
    assert_eq!(
        r.commit_with_intent("agenium/AGN-1", "01ABC")
            .unwrap()
            .as_deref(),
        Some(sha.as_str())
    );
    let msg = r
        .git()
        .check(&w.path, ["log", "-1", "--format=%an|%cn|%B"])
        .unwrap();
    assert!(msg.starts_with("worker-1|Agenium|Add hello"), "{msg}");
    assert!(msg.contains("Ticket: AGN-1"));
    assert!(r.diff(&w.path, &w.base_sha).unwrap().contains("+hello"));

    write(
        &w.path,
        "config.env",
        "GITHUB_TOKEN=ghp_abcdefghijklmnopqrstuvwxyz0123456789\n",
    );
    match r.commit_all(&w.path, "oops", AUTHOR, &[]) {
        Err(VcsError::SecretsFound(f)) => assert_eq!(
            (f[0].file.as_str(), f[0].rule.as_str()),
            ("config.env", "github-token")
        ),
        other => panic!("expected SecretsFound, got {other:?}"),
    }
    assert_eq!(
        r.head_sha("agenium/AGN-1").unwrap().unwrap(),
        sha,
        "nothing committed"
    );
    assert!(
        !r.status(&w.path).unwrap().is_empty(),
        "work is kept in the tree"
    );
}

#[test]
fn merge_queue_rebases_detects_conflicts_and_failed_checks() {
    let tmp = tempfile::tempdir().unwrap();
    let r = repo(tmp.path());
    // Seed a shared file on main.
    write(&r.root, "shared.txt", "line1\nline2\nline3\n");
    r.commit_all(&r.root, "seed", AUTHOR, &[]).unwrap();

    let a = r.add_worktree("AGN-1", "main").unwrap();
    let b = r.add_worktree("AGN-2", "main").unwrap();
    let c = r.add_worktree("AGN-3", "main").unwrap();
    let d = r.add_worktree("AGN-4", "main").unwrap();
    write(&a.path, "a.txt", "a\n");
    write(&a.path, "shared.txt", "line1 by A\nline2\nline3\n");
    r.commit_all(&a.path, "A", AUTHOR, &[]).unwrap();
    write(&b.path, "b.txt", "b\n");
    r.commit_all(&b.path, "B", AUTHOR, &[]).unwrap();
    write(&c.path, "shared.txt", "line1 by C\nline2\nline3\n");
    r.commit_all(&c.path, "C", AUTHOR, &[]).unwrap();
    write(&d.path, "d.txt", "d\n");
    r.commit_all(&d.path, "D", AUTHOR, &[]).unwrap();

    let m1 = r.merge_ticket("AGN-1", &NoCheck).unwrap();
    assert!(
        matches!(&m1, MergeOutcome::Merged { changed_files, .. } if changed_files == &vec!["a.txt".to_string(), "shared.txt".to_string()]),
        "{m1:?}"
    );
    // B branched before A merged: rebased onto the new main, then fast-forwarded.
    let m2 = r.merge_ticket("AGN-2", &NoCheck).unwrap();
    assert!(matches!(m2, MergeOutcome::Merged { .. }), "{m2:?}");
    assert!(r.root.join("a.txt").exists() && r.root.join("b.txt").exists());
    // C edits the same line as A: conflict, rebase aborted, branch unchanged.
    let c_head = r.head_sha("agenium/AGN-3").unwrap();
    assert_eq!(
        r.merge_ticket("AGN-3", &NoCheck).unwrap(),
        MergeOutcome::Conflict {
            files: vec!["shared.txt".into()]
        }
    );
    assert_eq!(r.head_sha("agenium/AGN-3").unwrap(), c_head);
    assert!(
        r.status(&c.path).unwrap().is_empty(),
        "no rebase left in progress"
    );
    // D fails its pre-merge check: main does not move.
    let main_before = r.head_sha("main").unwrap();
    assert!(
        matches!(r.merge_ticket("AGN-4", &FailCheck).unwrap(), MergeOutcome::CheckFailed { log } if log.contains("1 of 3"))
    );
    assert_eq!(r.head_sha("main").unwrap(), main_before);
    // Replays are idempotent.
    assert!(matches!(
        r.merge_ticket("AGN-1", &NoCheck).unwrap(),
        MergeOutcome::AlreadyMerged { .. }
    ));
    // Cleanup is idempotent; abandoned branches need -D.
    r.remove_worktree("AGN-3").unwrap();
    r.remove_worktree("AGN-3").unwrap();
    r.delete_branch("agenium/AGN-3").unwrap();
    assert!(!r.branch_exists("agenium/AGN-3").unwrap());
    assert!(!c.path.exists());
}

#[test]
fn sequential_queue_of_twenty_tickets_has_no_lost_work() {
    let tmp = tempfile::tempdir().unwrap();
    let r = repo(tmp.path());
    let keys: Vec<String> = (1..=20).map(|i| format!("AGN-{i}")).collect();
    for k in &keys {
        let w = r.add_worktree(k, "main").unwrap();
        write(&w.path, &format!("deliverables/{k}.txt"), k);
        r.commit_all(&w.path, k, AUTHOR, &[("Ticket", k)]).unwrap();
    }
    for k in &keys {
        assert!(
            matches!(
                r.merge_ticket(k, &NoCheck).unwrap(),
                MergeOutcome::Merged { .. }
            ),
            "{k}"
        );
    }
    for k in &keys {
        assert!(
            r.root.join(format!("deliverables/{k}.txt")).exists(),
            "{k} lost"
        );
        assert!(r
            .is_ancestor(&r.head_sha(&ticket_branch(k)).unwrap().unwrap(), "main")
            .unwrap());
    }
    let n: usize = r
        .git()
        .check(&r.root, ["rev-list", "--count", "main"])
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert_eq!(
        n, 21,
        "one commit per ticket + the initial commit, linear history"
    );
}

#[test]
fn remote_test_against_local_bare_repository() {
    let tmp = tempfile::tempdir().unwrap();
    let r = repo(tmp.path());
    let bare = tmp.path().join("remote.git");
    Git::default()
        .check(tmp.path(), ["init", "-q", "--bare", bare.to_str().unwrap()])
        .unwrap();
    let url = bare.to_string_lossy().replace('\\', "/");
    let t = r.test_remote(&url, &[]).unwrap();
    assert!(t.read_ok && t.write_ok, "{t:?}");
    let bad = r
        .test_remote(&tmp.path().join("missing.git").to_string_lossy(), &[])
        .unwrap();
    assert!(!bad.read_ok && !bad.write_ok);
    r.push(&url, "main:refs/heads/main", &[]).unwrap();
    let heads = Git::default()
        .check(tmp.path(), ["ls-remote", "--heads", &url])
        .unwrap();
    assert!(heads.contains("refs/heads/main"));
}
