use super::*;

fn policy(dir: &Path) -> SandboxPolicy {
    SandboxPolicy::workspace(dir)
}

/// Removes the worktree's command TEMP when the test ends.
struct RunTemp<'a>(&'a Path);

impl Drop for RunTemp<'_> {
    fn drop(&mut self) {
        remove_run_temp_dir(self.0);
    }
}

fn git(dir: &Path, args: &[&str]) {
    let ok = std::process::Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@agenium.local"])
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    assert!(ok, "git {args:?}");
}

/// The worktree's `.git` file is the agent's to rewrite: only the git directory git made for this worktree gives a
/// read root (the main repository's `.git`), never a folder the file names (`gitdir: <profile>` made the profile
/// readable).
#[test]
fn git_read_roots_trust_only_the_worktrees_own_git_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let project = tmp.path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    git(&project, &["init", "-q", "-b", "main"]);
    git(&project, &["commit", "-q", "--allow-empty", "-m", "init"]);
    let worktrees = project.join(".agenium").join("worktrees");
    let (wt, other) = (worktrees.join("T1"), worktrees.join("T2"));
    for (w, b) in [(&wt, "agenium/T1"), (&other, "agenium/T2")] {
        git(
            &project,
            &["worktree", "add", "-q", "-b", b, w.to_str().unwrap()],
        );
    }
    let main_git = std::fs::canonicalize(project.join(".git")).unwrap();
    let roots = |w: &Path| -> Vec<PathBuf> {
        git_read_roots(w)
            .iter()
            .map(|r| std::fs::canonicalize(r).unwrap())
            .collect()
    };
    assert_eq!(roots(&wt), std::slice::from_ref(&main_git));

    let genuine = std::fs::read_to_string(wt.join(".git")).unwrap();
    let point = |target: &Path| {
        std::fs::write(wt.join(".git"), format!("gitdir: {}\n", target.display())).unwrap()
    };
    // A folder outside the worktree: nothing in it points back.
    let profile = tmp.path().join("profile");
    std::fs::create_dir_all(&profile).unwrap();
    point(&profile);
    assert!(roots(&wt).is_empty());
    // The worktree itself, with the files git would write: its folder `.agenium/worktrees` made `.agenium` (every
    // other worktree) the common dir.
    std::fs::write(
        wt.join("gitdir"),
        format!("{}\n", wt.join(".git").display()),
    )
    .unwrap();
    std::fs::write(wt.join("commondir"), "../..\n").unwrap();
    point(&wt);
    assert!(roots(&wt).is_empty());
    // A made-up admin folder elsewhere (the agent's temp dir) that names the profile as its repository.
    let fake = tmp.path().join("tmp").join("T1");
    std::fs::create_dir_all(&fake).unwrap();
    std::fs::write(
        fake.join("gitdir"),
        format!("{}\n", wt.join(".git").display()),
    )
    .unwrap();
    std::fs::write(fake.join("commondir"), format!("{}\n", profile.display())).unwrap();
    point(&fake);
    assert!(roots(&wt).is_empty());
    // Another worktree's admin folder: it points back to that worktree.
    let other_admin = std::fs::read_to_string(other.join(".git")).unwrap();
    std::fs::write(wt.join(".git"), other_admin).unwrap();
    assert!(roots(&wt).is_empty());

    std::fs::write(wt.join(".git"), genuine).unwrap();
    assert_eq!(roots(&wt), [main_git]);
}

#[test]
fn fail_closed_detection() {
    let a = detect();
    // Windows: MXC when the native API works (ADR-53); Linux: bubblewrap (ADR-54); macOS is M2.
    if cfg!(windows) {
        assert!(matches!(a.backend, None | Some(Backend::WindowsMxc)));
    } else if cfg!(target_os = "linux") {
        assert!(matches!(a.backend, None | Some(Backend::Bubblewrap)));
    } else if cfg!(target_os = "macos") {
        assert!(matches!(a.backend, None | Some(Backend::Seatbelt)));
    }
    assert!(!a.reason.is_empty());
}

#[tokio::test]
async fn runs_with_clean_env_and_captures_output() {
    let tmp = tempfile::tempdir().unwrap();
    let _temp = RunTemp(tmp.path());
    std::env::set_var("AGENIUM_TEST_SECRET_CANARY", "sk-ant-CANARY");
    let ex = Executor::new(Shell::detect());
    let cmd = match ex.shell() {
        Shell::Bash(_) => "echo hello; echo oops >&2; env; exit 3",
        Shell::Cmd => "echo hello& echo oops 1>&2& set& exit /b 3",
        Shell::PowerShell(_) => "Write-Output hello; [Console]::Error.WriteLine('oops'); Get-ChildItem env: | ForEach-Object { \"$($_.Name)=$($_.Value)\" }; exit 3",
    };
    let mut extra = BTreeMap::new();
    extra.insert("AGENIUM_RUN_ID".to_string(), "run-1".to_string());
    let out = ex
        .run(
            &CommandSpec {
                command: cmd.into(),
                cwd: tmp.path().into(),
                timeout: Duration::from_secs(30),
                env: extra,
                ..Default::default()
            },
            &policy(tmp.path()),
        )
        .await
        .unwrap();
    assert_eq!(out.exit_code, Some(3));
    assert!(out.stdout.contains("hello"));
    assert!(out.stderr.contains("oops"));
    assert!(out.stdout.contains("AGENIUM_RUN_ID=run-1"));
    assert!(
        !out.stdout.contains("CANARY"),
        "secrets never reach agent commands"
    );
}

/// Vietnamese text survives a command: a UTF-8 file read with the shell's own reader comes back as written
/// (Windows PowerShell 5.1 read it as ANSI: "xin chÃ o").
#[tokio::test]
async fn utf8_text_survives_the_shell() {
    let tmp = tempfile::tempdir().unwrap();
    let _temp = RunTemp(tmp.path());
    std::fs::write(tmp.path().join("greeting.txt"), "xin chào\n").unwrap();
    let ex = Executor::new(Shell::detect());
    let cmd = match ex.shell() {
        Shell::Bash(_) => "cat greeting.txt",
        Shell::Cmd => "type greeting.txt",
        Shell::PowerShell(_) => "Get-Content greeting.txt",
    };
    let out = ex
        .run(
            &CommandSpec {
                command: cmd.into(),
                cwd: tmp.path().into(),
                timeout: Duration::from_secs(30),
                env: BTreeMap::new(),
                ..Default::default()
            },
            &policy(tmp.path()),
        )
        .await
        .unwrap();
    assert!(out.stdout.contains("xin chào"), "{:?}", out.stdout);
}

/// Output that is not UTF-8 (a Windows program writing in the ANSI code page) is decoded with that code page;
/// the UTF-8 lines around it stay as they are.
#[test]
fn output_in_the_ansi_code_page_is_still_text() {
    let s = decode_output(b"ok \xc3\xa0\n\xe0\n");
    assert!(s.starts_with("ok \u{e0}\n") && s.ends_with('\n'), "{s:?}");
    if cfg!(windows) {
        assert!(!s.contains('\u{fffd}'), "{s:?}");
    }
}

/// A double-byte character is decoded whole: its lead byte alone is invalid UTF-8 and its trail byte came as the
/// next invalid run, so decoding run by run turned "中" (GBK D6 D0) into two wrong characters.
#[test]
fn a_double_byte_character_is_not_split() {
    let gbk = |b: &[u8]| (b == b"\xd6\xd0\n").then(|| "\u{4e2d}\n".to_owned());
    assert_eq!(decode_lines(b"ok\n\xd6\xd0\n", gbk), "ok\n\u{4e2d}\n");
    #[cfg(windows)]
    assert_eq!(
        decode_lines(b"\xd6\xd0\xce\xc4\n", |b| code_page(b, 936)),
        "\u{4e2d}\u{6587}\n"
    );
}

/// `[sandbox] network = true` is the default (ADR-84): a command must start with it. On Ubuntu 25.10 every such
/// command failed, because the policy also asked bubblewrap to accept inbound connections, and where the host
/// cannot give the sandbox a private network the command now runs without one and says so.
#[tokio::test]
async fn a_command_with_the_network_on_starts() {
    if detect().backend.is_none() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let _temp = RunTemp(tmp.path());
    let ex = Executor::new(Shell::detect());
    let mut p = policy(tmp.path());
    p.network = true;
    let out = ex
        .run(
            &CommandSpec {
                command: "echo up".into(),
                cwd: tmp.path().into(),
                timeout: Duration::from_secs(30),
                env: BTreeMap::new(),
                ..Default::default()
            },
            &p,
        )
        .await
        .unwrap();
    assert_eq!(out.exit_code, Some(0), "{out:?}");
    assert!(out.stdout.contains("up"), "{out:?}");
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    if unix::private_network().is_err() {
        assert!(out.stderr.contains("ran without network"), "{out:?}");
    }
}

/// Node resolves every `require` with `fs.realpathSync`, which reads the attributes of each folder from the
/// drive root down to the file: inside the sandbox those parents must be listable (found by a team run whose
/// `node --test` died with `EPERM: lstat 'C:\'`).
#[tokio::test]
async fn node_resolves_paths_inside_the_sandbox() {
    if detect().backend.is_none() || find_program("node").is_none() {
        eprintln!("skipped: no sandbox backend or no node here");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let _temp = RunTemp(tmp.path());
    std::fs::write(tmp.path().join("m.js"), "module.exports = 41 + 1;").unwrap();
    // A file next to the worktree: its folder may be listed, the file must stay unreadable.
    let beside = tempfile::NamedTempFile::new_in(tmp.path().parent().unwrap()).unwrap();
    std::fs::write(beside.path(), "secret").unwrap();
    std::fs::write(
        tmp.path().join("t.js"),
        format!(
            "console.log('got', require('./m.js'));
try {{ require('fs').readFileSync({:?}); console.log('LEAK') }} catch (e) {{ console.log('denied', e.code) }}",
            beside.path().to_string_lossy()
        ),
    )
    .unwrap();
    let ex = Executor::new(Shell::detect());
    let out = ex
        .run(
            &CommandSpec {
                command: "node t.js".into(),
                cwd: tmp.path().into(),
                timeout: Duration::from_secs(60),
                env: BTreeMap::new(),
                ..Default::default()
            },
            &policy(tmp.path()),
        )
        .await
        .unwrap();
    assert!(out.stdout.contains("got 42"), "{out:?}");
    assert!(
        out.stdout.contains("denied") && !out.stdout.contains("LEAK"),
        "{out:?}"
    );
}

#[tokio::test]
async fn timeout_kills_the_process_tree() {
    let tmp = tempfile::tempdir().unwrap();
    let _temp = RunTemp(tmp.path());
    let ex = Executor::new(Shell::detect());
    let cmd = match ex.shell() {
        Shell::Bash(_) => "sleep 30 & sleep 30; echo never",
        Shell::Cmd => "ping -n 30 127.0.0.1 > nul & echo never",
        Shell::PowerShell(_) => "Start-Sleep -Seconds 30; Write-Output never",
    };
    let out = ex
        .run(
            &CommandSpec {
                command: cmd.into(),
                cwd: tmp.path().into(),
                timeout: Duration::from_millis(800),
                env: BTreeMap::new(),
                ..Default::default()
            },
            &policy(tmp.path()),
        )
        .await
        .unwrap();
    assert!(out.timed_out);
    assert!(out.elapsed_ms < 10_000, "{}", out.elapsed_ms);
    assert!(!out.stdout.contains("never"));
}

/// Audit L1 (2026-10-31): unsandboxed on Unix (approval mode), what a command started in the background ran on
/// after it ended; on Windows the job object ends it. It ends with the command.
#[cfg(unix)]
#[tokio::test]
async fn a_background_child_ends_with_its_unsandboxed_command() {
    let tmp = tempfile::tempdir().unwrap();
    let ex = Executor::new(Shell::detect());
    let mut full = policy(tmp.path());
    full.mode = SandboxMode::FullAccess;
    let out = ex
        .run(
            &CommandSpec {
                command: "sleep 100 & echo $!".into(),
                cwd: tmp.path().into(),
                timeout: Duration::from_secs(30),
                env: BTreeMap::new(),
                ..Default::default()
            },
            &full,
        )
        .await
        .unwrap();
    let child: i32 = out.stdout.trim().parse().unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    // SAFETY: signal 0 only asks whether the process exists.
    let alive = unsafe { libc::kill(child, 0) } == 0;
    assert!(!alive, "the background sleep was ended");
}

/// The cmd fallback keeps a script's quotes as written (the C runtime's quoting put a backslash before each, which cmd
/// kept: audit M-22).
#[cfg(windows)]
#[tokio::test]
async fn cmd_keeps_the_scripts_quotes() {
    let tmp = tempfile::tempdir().unwrap();
    let _temp = RunTemp(tmp.path());
    let out = Executor::new(Shell::Cmd)
        .run(
            &CommandSpec {
                command: r#"echo "a b"> "out file.txt"& type "out file.txt""#.into(),
                cwd: tmp.path().into(),
                timeout: Duration::from_secs(30),
                ..Default::default()
            },
            &SandboxPolicy {
                mode: SandboxMode::FullAccess,
                ..policy(tmp.path())
            },
        )
        .await
        .unwrap();
    assert_eq!(out.exit_code, Some(0), "{out:?}");
    assert_eq!(out.stdout.trim(), r#""a b""#);
    assert!(tmp.path().join("out file.txt").is_file());
}
