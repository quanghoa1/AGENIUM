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
fn generated_files_stay_out_of_commits() {
    let tmp = tempfile::tempdir().unwrap();
    let r = repo(tmp.path());
    // A project made before the list grew: the worktree adds what is missing, once.
    std::fs::write(r.root.join(".git/info/exclude"), ".agenium/\n").unwrap();
    let w = r.add_worktree("AGN-1", "main").unwrap();
    r.add_worktree("AGN-1", "main").unwrap();
    let excl = std::fs::read_to_string(r.root.join(".git/info/exclude")).unwrap();
    assert_eq!(excl.matches("__pycache__/").count(), 1, "{excl}");
    write(&w.path, "app.py", "print(1)\n");
    write(&w.path, "__pycache__/app.cpython-313.pyc", "x");
    write(&w.path, "tests/__pycache__/t.pyc", "x");
    write(&w.path, "web/node_modules/left-pad/index.js", "x");
    r.commit_all(&w.path, "Add app", AUTHOR, &[])
        .unwrap()
        .unwrap();
    let files = r
        .git()
        .check(&w.path, ["show", "--name-only", "--format=", "HEAD"])
        .unwrap();
    assert_eq!(files.trim(), "app.py");
    assert!(
        r.status(&w.path).unwrap().is_empty(),
        "caches do not dirty the tree"
    );
    // A tool output committed before it was excluded leaves the branch with the next commit, and stays on disk.
    write(
        &w.path,
        "artifacts/tool-output/run-1/0001.log",
        "long output",
    );
    r.git()
        .check(
            &w.path,
            ["add", "-f", "artifacts/tool-output/run-1/0001.log"],
        )
        .unwrap();
    r.git()
        // An identity of its own: a machine without a global git identity runs this too.
        .check(
            &w.path,
            [
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "commit",
                "-q",
                "-m",
                "stray log",
            ],
        )
        .unwrap();
    write(&w.path, "app.py", "print(2)\n");
    r.commit_all(&w.path, "Fix app", AUTHOR, &[])
        .unwrap()
        .unwrap();
    let tracked = r.git().check(&w.path, ["ls-files"]).unwrap();
    assert_eq!(tracked.trim(), "app.py", "{tracked}");
    assert!(w
        .path
        .join("artifacts/tool-output/run-1/0001.log")
        .is_file());
    // Merging after main moved replays the commit that held the log: the copy on disk does not block the rebase.
    write(
        &r.root,
        "other.txt",
        "x
",
    );
    r.commit_all(&r.root, "other", AUTHOR, &[])
        .unwrap()
        .unwrap();
    let m = r.merge_ticket("AGN-1", &NoCheck).unwrap();
    assert!(matches!(m, MergeOutcome::Merged { .. }), "{m:?}");
    assert!(
        !r.root.join("artifacts/tool-output").exists(),
        "no log in main"
    );
    assert!(
        w.path
            .join("artifacts/tool-output/run-1/0001.log")
            .is_file(),
        "put back"
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
    // B branched before A merged: rebased onto the new main, then fast-forwarded. The full output a review
    // run's tool left in B's worktree is not part of the work and does not block the merge.
    write(
        &b.path,
        "artifacts/tool-output/run9/0001.log",
        "long output\n",
    );
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

#[test]
fn a_dependent_worktree_catches_up_with_main_before_its_work() {
    let tmp = tempfile::tempdir().unwrap();
    let r = repo(tmp.path());
    let a = r.add_worktree("AGN-1", "main").unwrap();
    let b = r.add_worktree("AGN-2", "main").unwrap();
    assert_eq!(r.catch_up("AGN-2").unwrap(), None, "nothing new on main");
    write(&a.path, "api/greeting.json", "{}");
    r.commit_all(&a.path, "AGN-1: api", AUTHOR, &[]).unwrap();
    assert!(matches!(
        r.merge_ticket("AGN-1", &NoCheck).unwrap(),
        MergeOutcome::Merged { .. }
    ));
    // AGN-2 was created before AGN-1 merged: it starts from the new main.
    let base = r.catch_up("AGN-2").unwrap().expect("fast-forwarded");
    assert_eq!(Some(base), r.head_sha("main").unwrap());
    assert!(b.path.join("api/greeting.json").is_file());
    // Work of its own (committed or not) is never moved here.
    write(&b.path, "web/index.html", "x");
    assert_eq!(r.catch_up("AGN-2").unwrap(), None);
}

/// Decision 19: a worktree changed outside Agenium (an edited file, a new file, a commit without Agenium's trailer)
/// is detected against the state Agenium left, and put back; the undone change comes back as a patch.
#[test]
fn outside_changes_to_a_worktree_are_detected_and_undone() {
    let tmp = tempfile::tempdir().unwrap();
    let r = repo(tmp.path());
    let w = r.add_worktree("AGN-1", "main").unwrap();
    write(&w.path, "deliverables/a.txt", "agent\n");
    r.commit_all(&w.path, "Add a", AUTHOR, &[("Agenium-Intent", "01ABC")])
        .unwrap();
    write(&w.path, "deliverables/wip.txt", "agent work in progress\n");
    let left = r.worktree_state(&w.path).unwrap();
    assert_eq!(r.worktree_state(&w.path).unwrap(), left, "stable");
    assert!(
        r.status(&w.path)
            .unwrap()
            .iter()
            .any(|l| l.contains("wip.txt")),
        "real index untouched"
    );
    assert!(r.foreign_commits(&w.path, &left.head).unwrap().is_empty());

    // Outside: edit, add a file, and commit on the branch.
    write(&w.path, "deliverables/a.txt", "user\n");
    write(&w.path, "notes/user.md", "mine\n");
    r.git().check(&w.path, ["add", "-A"]).unwrap();
    r.git()
        .check(
            &w.path,
            [
                "-c",
                "user.name=u",
                "-c",
                "user.email=u@x",
                "commit",
                "-q",
                "-m",
                "by hand",
            ],
        )
        .unwrap();
    assert_ne!(r.worktree_state(&w.path).unwrap(), left);
    assert_eq!(r.foreign_commits(&w.path, &left.head).unwrap().len(), 1);

    let patch = r.restore_worktree(&w.path, &left).unwrap();
    assert!(
        patch.contains("notes/user.md") && patch.contains("+user"),
        "{patch}"
    );
    assert_eq!(
        r.worktree_state(&w.path).unwrap(),
        left,
        "back to what Agenium left"
    );
    assert!(!w.path.join("notes/user.md").exists());
    assert_eq!(
        std::fs::read_to_string(w.path.join("deliverables/a.txt")).unwrap(),
        "agent\n"
    );
    assert_eq!(
        std::fs::read_to_string(w.path.join("deliverables/wip.txt")).unwrap(),
        "agent work in progress\n",
        "the agent's uncommitted work stays"
    );
}

/// `remote_url` reports the repo's `origin` (push-on-close uses it to tell a project with a remote from a local-only
/// one) and `push origin` lands on it.
#[test]
fn remote_url_detects_origin_and_push_lands() {
    let tmp = tempfile::tempdir().unwrap();
    let r = repo(tmp.path());
    assert!(
        r.remote_url("origin").is_none(),
        "no remote yet: stays local"
    );
    let bare = tmp.path().join("remote.git");
    Git::default()
        .check(tmp.path(), ["init", "-q", "--bare", bare.to_str().unwrap()])
        .unwrap();
    let url = bare.to_string_lossy().replace('\\', "/");
    Git::default()
        .check(&r.root, ["remote", "add", "origin", &url])
        .unwrap();
    assert_eq!(r.remote_url("origin").as_deref(), Some(url.as_str()));
    r.push("origin", "main:main", &[]).unwrap();
    let heads = Git::default()
        .check(tmp.path(), ["ls-remote", "--heads", &url])
        .unwrap();
    assert!(heads.contains("refs/heads/main"), "{heads}");
}

/// An executable hook script.
fn hook(path: &Path, body: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

/// A worktree's `.git` file is inside the sandbox's writable root: the agent can point it at a repository whose config
/// runs commands (`core.fsmonitor`, hooks). Agenium's git calls on the worktree run on the host, so they take the git
/// directory from the main repository and run none of it, and the work still lands on the ticket branch. Hooks that
/// the main repository's own config finds inside the worktree (a relative `core.hooksPath`, as husky sets) do not run
/// either.
#[test]
fn a_rewritten_dot_git_file_runs_nothing_on_the_host() {
    let tmp = tempfile::tempdir().unwrap();
    let r = repo(tmp.path());
    let w = r.add_worktree("AGN-1", "main").unwrap();
    let marker = tmp.path().join("ran-on-host");
    let m = marker.to_string_lossy().replace('\\', "/");
    let evil = tmp.path().join("evil");
    Git::default()
        .check(tmp.path(), ["init", "-q", evil.to_str().unwrap()])
        .unwrap();
    Git::default()
        .check(
            &evil,
            [
                "config",
                "core.fsmonitor",
                &format!("echo fsmonitor >> '{m}'"),
            ],
        )
        .unwrap();
    hook(
        &evil.join(".git/hooks/post-commit"),
        &format!("echo evil-hook >> '{m}'"),
    );
    std::fs::write(
        w.path.join(".git"),
        format!(
            "gitdir: {}\n",
            evil.join(".git").to_string_lossy().replace('\\', "/")
        ),
    )
    .unwrap();
    Git::default()
        .check(&r.root, ["config", "core.hooksPath", ".hooks"])
        .unwrap();
    hook(
        &w.path.join(".hooks/post-commit"),
        &format!("echo worktree-hook >> '{m}'"),
    );

    write(&w.path, "src/a.txt", "a\n");
    assert!(!r.status(&w.path).unwrap().is_empty());
    let sha = r
        .commit_all(&w.path, "Add a", AUTHOR, &[])
        .unwrap()
        .unwrap();
    r.worktree_state(&w.path).unwrap();
    assert!(
        !marker.exists(),
        "ran on the host: {}",
        std::fs::read_to_string(&marker).unwrap_or_default()
    );
    assert!(r.diff(&w.path, &w.base_sha).unwrap().contains("+a"));
    assert_eq!(r.head_sha("agenium/AGN-1").unwrap(), Some(sha));
    assert!(
        r.git_for(&evil).is_err(),
        "a directory that is not one of the repository's worktrees is refused"
    );
}

/// The merge into the project's own checkout runs no hook either: with a relative `core.hooksPath` (husky's
/// `.husky`), a `post-merge` the ticket's branch brings was found in the checkout right after the fast-forward and ran
/// on the host.
#[test]
fn the_merge_into_the_checkout_runs_no_hook_the_branch_brings() {
    let tmp = tempfile::tempdir().unwrap();
    let r = repo(tmp.path());
    write(&r.root, "README.md", "seed\n");
    r.commit_all(&r.root, "seed", AUTHOR, &[]).unwrap();
    Git::default()
        .check(&r.root, ["config", "core.hooksPath", ".husky"])
        .unwrap();
    let marker = tmp.path().join("ran-on-host");
    let m = marker.to_string_lossy().replace('\\', "/");
    let w = r.add_worktree("AGN-1", "main").unwrap();
    hook(
        &w.path.join(".husky/post-merge"),
        &format!("echo post-merge >> '{m}'"),
    );
    write(&w.path, "src/a.txt", "a\n");
    r.commit_all(&w.path, "Add a", AUTHOR, &[]).unwrap();
    assert!(matches!(
        r.merge_ticket("AGN-1", &NoCheck).unwrap(),
        MergeOutcome::Merged { .. }
    ));
    assert!(
        r.root.join(".husky/post-merge").exists(),
        "the branch's files are merged"
    );
    assert!(
        !marker.exists(),
        "ran on the host: {}",
        std::fs::read_to_string(&marker).unwrap_or_default()
    );
}

/// A crash in the middle of the merge queue's rebase leaves the worktree on a detached HEAD with the rebase in
/// progress. Opening the worktree again aborts the rebase and keeps the directory and its work: it was deleted, and
/// every later attempt failed ("missing but already registered worktree"). A registered worktree whose directory is
/// gone is added again.
#[test]
fn a_worktree_left_mid_rebase_is_recovered_not_deleted() {
    let tmp = tempfile::tempdir().unwrap();
    let r = repo(tmp.path());
    write(&r.root, "f.txt", "base\n");
    r.commit_all(&r.root, "base", AUTHOR, &[]).unwrap();
    let w = r.add_worktree("AGN-1", "main").unwrap();
    write(&w.path, "f.txt", "ticket\n");
    let ticket = r
        .commit_all(&w.path, "ticket", AUTHOR, &[])
        .unwrap()
        .unwrap();
    write(&r.root, "f.txt", "main\n");
    r.commit_all(&r.root, "main moves", AUTHOR, &[]).unwrap();
    // The crash: a rebase stopped on its conflict and never aborted.
    let stopped = Git::default()
        .run(&w.path, ["rebase", "-q", "main"])
        .unwrap();
    assert!(!stopped.ok(), "{stopped:?}");

    let again = r.add_worktree("AGN-1", "main").unwrap();
    assert_eq!(again.path, w.path);
    assert_eq!(r.head_sha("agenium/AGN-1").unwrap(), Some(ticket));
    assert_eq!(
        std::fs::read_to_string(w.path.join("f.txt")).unwrap(),
        "ticket\n"
    );
    assert!(
        r.status(&w.path).unwrap().is_empty(),
        "no rebase left in progress"
    );
    assert_eq!(r.add_worktree("AGN-1", "main").unwrap(), again);

    // The directory is gone but still registered: added again from the branch.
    std::fs::remove_dir_all(&w.path).unwrap();
    let back = r.add_worktree("AGN-1", "main").unwrap();
    assert_eq!(
        std::fs::read_to_string(back.path.join("f.txt")).unwrap(),
        "ticket\n"
    );
}
