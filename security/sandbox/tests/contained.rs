//! `spawn_contained`: a long-running process inside the sandbox with live stdio (CliBackend, ADR-56).

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};

use agenium_sandbox::service::spawn_contained;
use agenium_sandbox::{detect, SandboxPolicy};

/// Removes a worktree's command TEMP (outside the worktree, under the system TEMP) when the test ends.
struct RunTemp(std::path::PathBuf);

impl Drop for RunTemp {
    fn drop(&mut self) {
        agenium_sandbox::remove_run_temp_dir(&self.0);
    }
}

fn echo_argv() -> Vec<String> {
    if cfg!(windows) {
        // Reads one line from stdin and writes it back, then exits.
        vec![
            "powershell.exe".into(),
            "-NoProfile".into(),
            "-Command".into(),
            "$l = [Console]::In.ReadLine(); [Console]::Out.WriteLine('got ' + $l); exit 0".into(),
        ]
    } else {
        vec!["sh".into(), "-c".into(), "read l; echo \"got $l\"".into()]
    }
}

#[test]
fn live_stdio_round_trip() {
    if detect().backend.is_none() {
        eprintln!("skipped: no sandbox backend here");
        return;
    }
    let wt = tempfile::tempdir().unwrap();
    let _temp = RunTemp(wt.path().to_path_buf());
    let policy = SandboxPolicy::workspace(wt.path());
    let (mut p, mut io) =
        spawn_contained(&echo_argv(), wt.path(), &BTreeMap::new(), &policy).unwrap();
    let mut stdin = io.stdin.take().unwrap();
    writeln!(stdin, "ping").unwrap();
    stdin.flush().unwrap();
    drop(stdin);
    let mut line = String::new();
    BufReader::new(io.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    assert_eq!(line.trim(), "got ping");
    for _ in 0..100 {
        if p.try_wait().unwrap().is_some() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    panic!("the contained process did not exit");
}

/// A `cmd /c` script keeps its quotes in the container (upstream `cmd_script_command_line`): the C runtime's quoting
/// would have put a backslash before each one, which cmd keeps.
#[cfg(windows)]
#[test]
fn a_cmd_script_runs_with_its_quotes() {
    if detect().backend.is_none() {
        eprintln!("skipped: no sandbox backend here");
        return;
    }
    let wt = tempfile::tempdir().unwrap();
    let _temp = RunTemp(wt.path().to_path_buf());
    std::fs::write(wt.path().join("a b.txt"), "inside\r\n").unwrap();
    let policy = SandboxPolicy::workspace(wt.path());
    let argv: Vec<String> = ["cmd", "/D", "/S", "/C", r#"type "a b.txt" & echo "q""#]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let (mut p, mut io) = spawn_contained(&argv, wt.path(), &BTreeMap::new(), &policy).unwrap();
    drop(io.stdin.take());
    let mut out = String::new();
    std::io::Read::read_to_string(&mut io.stdout.take().unwrap(), &mut out).unwrap();
    let _ = p.try_wait();
    assert!(out.contains("inside") && out.contains("\"q\""), "{out:?}");
}

/// Opt-in: `AGENIUM_CONTAINED_AGY=1` runs agy (logged in) inside the sandbox with network allowed and a per-run home.
#[test]
fn agy_inside_the_sandbox() {
    if std::env::var("AGENIUM_CONTAINED_AGY").is_err() {
        eprintln!("skipped: set AGENIUM_CONTAINED_AGY=1");
        return;
    }
    let agy = agenium_sandbox::find_program("agy").expect("agy on PATH");
    let wt = tempfile::tempdir().unwrap();
    let _temp = RunTemp(wt.path().to_path_buf());
    let home = tempfile::tempdir().unwrap();
    let mut policy = SandboxPolicy::workspace(wt.path());
    policy.network = true;
    policy.writable_roots.push(home.path().to_path_buf());
    let env: BTreeMap<String, String> = [
        ("USERPROFILE".to_owned(), home.path().display().to_string()),
        ("HOME".to_owned(), home.path().display().to_string()),
    ]
    .into();
    let argv = vec![
        agy.display().to_string(),
        "--output-format".into(),
        "stream-json".into(),
        "--model".into(),
        "gemini-3.8-flash-low".into(),
        "-p=Reply with the single word PONG.".into(),
    ];
    let (mut p, mut io) = spawn_contained(&argv, wt.path(), &env, &policy).unwrap();
    drop(io.stdin.take());
    let err = io.stderr.take().unwrap();
    let errs = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = std::io::Read::read_to_string(&mut BufReader::new(err), &mut s);
        s
    });
    let mut out = String::new();
    std::io::Read::read_to_string(&mut BufReader::new(io.stdout.take().unwrap()), &mut out)
        .unwrap();
    let code = loop {
        if let Some(c) = p.try_wait().unwrap() {
            break c;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    };
    let errs = errs.join().unwrap();
    eprintln!(
        "exit {code}\nSTDOUT tail: {}\nSTDERR tail: {}",
        tail(&out),
        tail(&errs)
    );
    assert!(
        out.contains("PONG"),
        "agy did not answer inside the sandbox"
    );
}

fn tail(s: &str) -> String {
    let n = s.chars().count();
    s.chars().skip(n.saturating_sub(800)).collect()
}

/// Opt-in: `AGENIUM_CONTAINED_CODEX=<model>` runs codex (logged in) inside the sandbox in its YOLO mode with a per-run
/// CODEX_HOME holding a copy of the login.
#[test]
fn codex_inside_the_sandbox() {
    let Ok(model) = std::env::var("AGENIUM_CONTAINED_CODEX") else {
        eprintln!("skipped: set AGENIUM_CONTAINED_CODEX=<model>");
        return;
    };
    let codex = agenium_sandbox::find_program("codex").expect("codex on PATH");
    let user_home = std::path::PathBuf::from(std::env::var("USERPROFILE").unwrap());
    let wt = tempfile::tempdir().unwrap();
    let _temp = RunTemp(wt.path().to_path_buf());
    let home = tempfile::tempdir().unwrap();
    std::fs::copy(
        user_home.join(".codex/auth.json"),
        home.path().join("auth.json"),
    )
    .unwrap();
    let mut policy = SandboxPolicy::workspace(wt.path());
    policy.network = true;
    policy.writable_roots.push(home.path().to_path_buf());
    let env: BTreeMap<String, String> =
        [("CODEX_HOME".to_owned(), home.path().display().to_string())].into();
    let argv = vec![
        codex.display().to_string(),
        "exec".into(),
        "--json".into(),
        "--ephemeral".into(),
        "--skip-git-repo-check".into(),
        "--dangerously-bypass-approvals-and-sandbox".into(),
        "-m".into(),
        model,
        "Create the file hello.txt containing the word hello, then reply DONE.".into(),
    ];
    let (mut p, mut io) = spawn_contained(&argv, wt.path(), &env, &policy).unwrap();
    drop(io.stdin.take());
    let err = io.stderr.take().unwrap();
    let errs = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = std::io::Read::read_to_string(&mut BufReader::new(err), &mut s);
        s
    });
    let mut out = String::new();
    std::io::Read::read_to_string(&mut BufReader::new(io.stdout.take().unwrap()), &mut out)
        .unwrap();
    let code = loop {
        if let Some(c) = p.try_wait().unwrap() {
            break c;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    };
    let errs = errs.join().unwrap();
    eprintln!(
        "exit {code}\nSTDOUT tail: {}\nSTDERR tail: {}",
        tail(&out),
        tail(&errs)
    );
    assert_eq!(
        std::fs::read_to_string(wt.path().join("hello.txt"))
            .unwrap_or_default()
            .trim(),
        "hello"
    );
}

/// A CLI's login shared with the user's (ADR-136): the per-run home, writable inside the sandbox, holds a hard link to a
/// file outside every writable root; the contained process reads and rewrites the file through the link (a Codex token
/// refresh), while the file's own path stays out of its reach.
#[test]
fn a_hard_link_in_a_writable_home_reaches_its_file() {
    if detect().backend.is_none() {
        eprintln!("skipped: no sandbox backend here");
        return;
    }
    let wt = tempfile::tempdir().unwrap();
    let _temp = RunTemp(wt.path().to_path_buf());
    let (home, outside) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let login = outside.path().join("auth.json");
    std::fs::write(&login, "old").unwrap();
    if std::fs::hard_link(&login, home.path().join("auth.json")).is_err() {
        eprintln!("skipped: no hard link between the temp folders");
        return;
    }
    let mut policy = SandboxPolicy::workspace(wt.path());
    policy.writable_roots.push(home.path().to_path_buf());
    let (link, direct) = (
        home.path().join("auth.json"),
        outside.path().join("direct.txt"),
    );
    let argv: Vec<String> = if cfg!(windows) {
        vec![
            "powershell.exe".into(),
            "-NoProfile".into(),
            "-Command".into(),
            format!(
                "$o = [IO.File]::ReadAllText('{l}'); [IO.File]::WriteAllText('{l}', $o + ' new'); \
                 try {{ [IO.File]::WriteAllText('{d}', 'x') }} catch {{ }}; exit 0",
                l = link.display(),
                d = direct.display()
            ),
        ]
    } else {
        vec![
            "sh".into(),
            "-c".into(),
            format!(
                "o=$(cat '{l}'); printf '%s new' \"$o\" > '{l}'; echo x > '{d}' 2>/dev/null; exit 0",
                l = link.display(),
                d = direct.display()
            ),
        ]
    };
    let (mut p, mut io) = spawn_contained(&argv, wt.path(), &BTreeMap::new(), &policy).unwrap();
    drop(io.stdin.take());
    let mut out = String::new();
    std::io::Read::read_to_string(&mut BufReader::new(io.stdout.take().unwrap()), &mut out)
        .unwrap();
    while p.try_wait().unwrap().is_none() {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert_eq!(std::fs::read_to_string(&login).unwrap(), "old new", "{out}");
    assert!(!direct.exists(), "the file's folder stays out of reach");
}
