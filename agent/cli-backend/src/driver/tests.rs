use super::*;
use serde_json::json;

#[test]
fn paths_inside_a_root() {
    let root = if cfg!(windows) {
        Path::new(r"C:\wt")
    } else {
        Path::new("/wt")
    };
    assert!(inside(Path::new("a/b.txt"), root, root));
    assert!(inside(&root.join("x"), root, root));
    assert!(!inside(Path::new("../wt2/x"), root, root));
    assert!(!inside(Path::new("../../etc/passwd"), root, root));
    if cfg!(windows) {
        assert!(inside(Path::new(r"c:\WT\sub\f"), root, root));
        assert!(!inside(Path::new(r"C:\wt2\f"), root, root));
    }
}

/// macOS runs of 2026-10-25/26 (stream_run): the worktree was /var/folders/…/wt and the CLI wrote under its
/// getcwd(), /private/var/folders/…/wt; the hook refused it. On Windows the canonical name is `\\?\D:\…`.
#[test]
fn the_canonical_name_of_the_worktree_is_inside_it() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("wt");
    std::fs::create_dir(&root).unwrap();
    let canon = std::fs::canonicalize(&root).unwrap();
    assert!(inside(&canon.join("a.txt"), &root, &root));
    assert!(!inside(&canon.join("..").join("escape.txt"), &root, &root));
}

/// Path review of 2026-10-29: a path under a name no driver listed, in a list, as `FILE:///` or with `%2F..` in a
/// URI, or as the base of a search pattern, is read and checked; the text a call carries is not.
#[test]
fn every_argument_that_names_a_file_is_read() {
    let got = |v: Value| named_paths(&v);
    assert_eq!(
        got(json!({"TargetDirectories": ["D:/elsewhere"], "Query": "a/b"})).unwrap(),
        ["D:/elsewhere"]
    );
    assert_eq!(got(json!({"Uri": "FILE:///C:/x/y"})).unwrap(), ["C:/x/y"]);
    assert_eq!(
        got(json!({"uri": "file:///home/u/..%2F..%2Fsecret"})).unwrap(),
        ["/home/u/../../secret"]
    );
    assert_eq!(
        got(json!({"target": "C:\\outside", "nested": {"more": ["/etc/x"]}})).unwrap(),
        ["/etc/x", "C:\\outside"]
    );
    assert_eq!(
        got(json!({"file_path": "notes.md", "content": "see a/b and C:\\d", "old_string": "x/y"}))
            .unwrap(),
        ["notes.md"]
    );
    assert!(got(json!({"Uri": "agenium://schemas/submit_report"}))
        .unwrap()
        .is_empty());
    for escaping in [
        "C:/Users/me/.ssh/*",
        "/etc/*",
        "../x/*",
        "~/.ssh/*",
        "\\\\server\\share\\*",
    ] {
        assert!(
            got(json!({"pattern": escaping, "path": "notes"})).is_err(),
            "{escaping}"
        );
    }
    assert!(got(json!({"pattern": "fn main", "glob": "**/*.rs", "path": "notes"})).is_ok());
}

/// A CLI's own tools keep to its scratch folder, but not to the files there that other programs read as the
/// user's (the folder is their home): a planted `.gitconfig` ran a command through git (path review of 2026-10-29).
#[test]
fn the_scratch_folder_keeps_no_dotfiles_but_the_cli_state() {
    let tmp = tempfile::tempdir().unwrap();
    let (wt, home) = (tmp.path().join("wt"), tmp.path().join("home"));
    let cx = CheckCtx {
        worktree: &wt,
        home: &home,
        mcp_name: "agenium",
        subagents: false,
        read_only: false,
    };
    assert!(file_access(&home.join("notes/plan.md"), &cx, &[]).is_ok());
    assert!(file_access(&home.join(".gemini/antigravity/brain/c1/task.md"), &cx, &[]).is_ok());
    for planted in [
        ".gitconfig",
        ".npmrc",
        ".ssh/config",
        ".config/git/config",
        ".GITCONFIG",
    ] {
        let e = file_access(&home.join(planted), &cx, &[]).unwrap_err();
        assert!(e.contains("other programs read"), "{planted}: {e}");
    }
    let git = crate::run::isolated_home(&home, Default::default());
    assert_eq!(git["GIT_CONFIG_NOSYSTEM"], "1");
    assert!(matches!(
        git["GIT_CONFIG_GLOBAL"].as_str(),
        "NUL" | "/dev/null"
    ));
}

/// The copy path (no hard link, another volume): a refresh made in the run reaches the user's file; what is not the
/// same login (`{}`, another account) does not, and the run's copy is put back (audit M-10).
#[test]
fn a_copied_login_syncs_only_the_same_login() {
    let tmp = tempfile::tempdir().unwrap();
    let (user, home) = (tmp.path().join("auth.json"), tmp.path().join("home"));
    std::fs::create_dir_all(&home).unwrap();
    let login = r#"{"tokens":{"account_id":"a1","refresh":"r1"}}"#;
    std::fs::write(&user, login).unwrap();
    let run = home.join("auth.json");
    copy_login(&user, &run).unwrap();
    for bad in ["{}", r#"{"tokens":{"account_id":"other","refresh":"x"}}"#] {
        std::fs::write(&run, bad).unwrap();
        sync_login(&user, &run).unwrap();
        assert_eq!(std::fs::read_to_string(&user).unwrap(), login, "{bad}");
        assert_eq!(std::fs::read_to_string(&run).unwrap(), login, "{bad}");
    }
    let refreshed = r#"{"tokens":{"account_id":"a1","refresh":"r2"}}"#;
    std::fs::write(&run, refreshed).unwrap();
    sync_login(&user, &run).unwrap();
    assert_eq!(std::fs::read_to_string(&user).unwrap(), refreshed);
}

/// ADR-136: the run's login is the user's own file (a hard link): what the CLI writes there (a refreshed token) is
/// in the user's file at once, and the copy-and-sync fallback is not used.
#[test]
fn a_runs_login_is_the_users_own_file() {
    let tmp = tempfile::tempdir().unwrap();
    let (user, home) = (tmp.path().join("auth.json"), tmp.path().join("home"));
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(&user, r#"{"refresh":"r1"}"#).unwrap();
    let run = home.join("auth.json");
    std::fs::write(&run, "stale copy of an earlier run").unwrap();
    assert!(link_login(&user, &run).unwrap(), "linked");
    assert_eq!(
        std::fs::read_to_string(&run).unwrap(),
        r#"{"refresh":"r1"}"#
    );
    // Codex writes its login in place: the refresh reaches the user's file, and no original is kept to sync.
    std::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(&run)
        .and_then(|mut f| std::io::Write::write_all(&mut f, br#"{"refresh":"r2"}"#))
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(&user).unwrap(),
        r#"{"refresh":"r2"}"#
    );
    assert!(!original(&run).exists());
    sync_login(&user, &run).unwrap();
    // Codex stopped while writing a refresh: the user's file is put back whole.
    crate::login::guard_linked(&user, &run).unwrap();
    std::fs::write(&user, "").unwrap();
    crate::login::guard_linked(&user, &run).unwrap();
    assert_eq!(
        std::fs::read_to_string(&user).unwrap(),
        r#"{"refresh":"r2"}"#
    );
    assert_eq!(
        std::fs::read_to_string(&run).unwrap(),
        r#"{"refresh":"r2"}"#,
        "still one file"
    );
    // Audit M13 (2026-10-31): the model writes the login (the run's home is writable) with something that parses
    // but is not this login: put back too. A refresh of the same shape stays.
    let write = |text: &str| {
        std::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&run)
            .and_then(|mut f| std::io::Write::write_all(&mut f, text.as_bytes()))
            .unwrap()
    };
    write("{}");
    crate::login::guard_linked(&user, &run).unwrap();
    assert_eq!(
        std::fs::read_to_string(&user).unwrap(),
        r#"{"refresh":"r2"}"#
    );
    write(r#"{"refresh":"r3"}"#);
    crate::login::guard_linked(&user, &run).unwrap();
    assert_eq!(
        std::fs::read_to_string(&user).unwrap(),
        r#"{"refresh":"r3"}"#,
        "a refresh stays"
    );
    // The run's home goes: the user's login stays.
    std::fs::remove_dir_all(&home).unwrap();
    assert_eq!(
        std::fs::read_to_string(&user).unwrap(),
        r#"{"refresh":"r3"}"#
    );
}

/// One CLI account in several runs at once: a run's refreshed login reaches the user's file at once, a login
/// refreshed elsewhere reaches the other runs' copies, the user's wins a clash, and half a login moves nowhere.
#[test]
fn logins_stay_the_same_across_runs_and_the_users_cli() {
    let user = tempfile::tempdir().unwrap();
    let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let u = user.path().join("auth.json");
    let (ha, hb) = (a.path().join("auth.json"), b.path().join("auth.json"));
    let read = |p: &Path| std::fs::read_to_string(p).unwrap();
    std::fs::write(&u, r#"{"refresh": "r1"}"#).unwrap();
    copy_login(&u, &ha).unwrap();
    copy_login(&u, &hb).unwrap();
    sync_login(&u, &ha).unwrap();
    assert_eq!(read(&u), r#"{"refresh": "r1"}"#, "nothing refreshed");
    // Run A refreshes (r1 is used up): the user's file has r2 at once, and run B's copy gets it at its next sync.
    std::fs::write(&ha, r#"{"refresh": "r2"}"#).unwrap();
    sync_login(&u, &ha).unwrap();
    assert_eq!(read(&u), r#"{"refresh": "r2"}"#);
    sync_login(&u, &hb).unwrap();
    assert_eq!(
        read(&hb),
        r#"{"refresh": "r2"}"#,
        "run B no longer holds r1"
    );
    sync_login(&u, &ha).unwrap();
    assert_eq!(read(&ha), r#"{"refresh": "r2"}"#, "run A is settled");
    // The user logged in again while run B refreshed: theirs wins, and B's copy takes it.
    std::fs::write(&u, r#"{"refresh": "user"}"#).unwrap();
    std::fs::write(&hb, r#"{"refresh": "r3"}"#).unwrap();
    sync_login(&u, &hb).unwrap();
    assert_eq!(read(&u), r#"{"refresh": "user"}"#);
    sync_login(&u, &hb).unwrap();
    assert_eq!(read(&hb), r#"{"refresh": "user"}"#);
    // Run A takes the user's login, then half a login (the CLI stopped while writing it) is handed nowhere; whole,
    // it goes.
    sync_login(&u, &ha).unwrap();
    assert_eq!(read(&ha), r#"{"refresh": "user"}"#);
    std::fs::write(&ha, r#"{"refresh": "#).unwrap();
    sync_login(&u, &ha).unwrap();
    assert_eq!(read(&u), r#"{"refresh": "user"}"#);
    std::fs::write(&ha, r#"{"refresh": "r4"}"#).unwrap();
    sync_login(&u, &ha).unwrap();
    assert_eq!(read(&u), r#"{"refresh": "r4"}"#);
    let left: Vec<_> = std::fs::read_dir(user.path())
        .unwrap()
        .flatten()
        .map(|e| e.file_name())
        .collect();
    assert_eq!(left.len(), 1, "no temporary file left: {left:?}");
}
