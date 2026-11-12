//! Which everyday developer commands the sandbox lets through, compared with the sandbox turned off (ADR-160).
//! Each command runs as Agenium's `shell` tool runs it (`Executor::run`, the worktree policy, network on) in a fresh
//! git worktree, then again with `set_off(true)`. A command that works off but not on is one the sandbox blocks.
//! `cargo run -p agenium-sandbox --example tool_probe [-- --out FILE]`; one line a command, then a summary.

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use agenium_sandbox::{set_off, CommandSpec, Executor, SandboxPolicy, Shell};

/// (name, command) — Unix shell lines; on Windows the probe only reports that it is meant for macOS and Linux.
const PROBES: &[(&str, &str)] = &[
    ("uname", "uname -a"),
    ("sw_vers", "sw_vers"),
    ("whoami", "whoami"),
    ("ps", "ps -o pid,comm -p $$"),
    ("tmp write", "f=\"${TMPDIR:-/tmp}/agenium-probe-$$\" && echo hi > \"$f\" && cat \"$f\" && rm \"$f\""),
    ("worktree write", "echo hi > probe.txt && cat probe.txt && rm probe.txt"),
    ("git status", "git status --short --branch"),
    ("git commit", "git add -A && git -c user.name=p -c user.email=p@p commit -q --allow-empty -m probe && git log --oneline -1"),
    ("git log", "git log --oneline -3"),
    ("curl https", "curl -sS -o /dev/null -w '%{http_code}' https://example.com"),
    ("dns", "python3 -c \"import socket; print(socket.gethostbyname('example.com'))\""),
    ("python3", "python3 -c 'import sys, ssl, sqlite3, json; print(sys.version)'"),
    ("python venv", "python3 -m venv .venv && .venv/bin/python -c 'print(1)' && rm -rf .venv"),
    ("pip install (venv)", "python3 -m venv .venv && .venv/bin/pip install -q six && .venv/bin/python -c 'import six; print(six.__version__)'; r=$?; rm -rf .venv; exit $r"),
    ("node", "node -e 'console.log(process.version)'"),
    ("npm", "npm --version"),
    ("npm install", "npm init -y >/dev/null && npm install --no-audit --no-fund left-pad >/dev/null && node -e \"console.log(require('left-pad')('x',3))\"; r=$?; rm -rf node_modules package.json package-lock.json; exit $r"),
    ("npx", "npx --yes cowsay@1.6.0 hi | head -3"),
    ("cargo", "cargo --version"),
    ("rustc", "rustc --version"),
    ("cargo new+build", "cargo new -q probe_crate && cd probe_crate && cargo build -q && ./target/debug/probe_crate; r=$?; cd .. && rm -rf probe_crate; exit $r"),
    ("cargo add (registry)", "cargo new -q probe_dep && cd probe_dep && cargo add -q itoa && cargo build -q; r=$?; cd .. && rm -rf probe_dep; exit $r"),
    ("go", "go version"),
    ("java", "java -version"),
    ("make", "make --version | head -1"),
    ("clang", "clang --version | head -1"),
    ("cc build", "printf 'int main(){return 0;}' > p.c && cc p.c -o p && ./p; r=$?; rm -f p p.c; exit $r"),
    ("xcrun sdk", "xcrun --show-sdk-path"),
    ("xcodebuild", "xcodebuild -version"),
    ("swift", "swift --version"),
    ("swift build", "mkdir sp && cd sp && swift package init -q --type executable >/dev/null 2>&1; swift build -q; r=$?; cd .. && rm -rf sp; exit $r"),
    ("swift build --disable-sandbox", "mkdir sp && cd sp && swift package init -q --type executable >/dev/null 2>&1; swift build -q --disable-sandbox; r=$?; cd .. && rm -rf sp; exit $r"),
    ("brew", "brew --version | head -1"),
    ("ruby", "ruby -v"),
    ("sqlite3", "sqlite3 :memory: 'select 1'"),
    ("docker", "docker version --format '{{.Client.Version}}'"),
    ("gh", "gh --version | head -1"),
    ("ssh", "ssh -V"),
    ("open -R (GUI)", "open -g -R ."),
    ("pbcopy", "echo x | pbcopy && pbpaste"),
    ("security (keychain)", "security list-keychains"),
    ("home read", "ls ~ | head -3"),
    ("home cache write", "mkdir -p ~/.cache && echo x > ~/.cache/agenium-probe && rm ~/.cache/agenium-probe"),
    ("localhost server", "python3 -m http.server 18765 >/dev/null 2>&1 & s=$!; sleep 1; curl -sS -o /dev/null -w '%{http_code}' http://127.0.0.1:18765/; r=$?; kill $s; exit $r"),
];

struct Run {
    code: Option<i32>,
    timed_out: bool,
    /// The output says a file or folder was refused.
    denied: bool,
    tail: String,
}

/// Exit 0 within the time, and no refusal in the output: some commands go on after a denied write (npm's logs) or
/// print a denied read and exit 0 (`ls ~ | head`), which the macOS run 2026-11-12 scored as fine.
fn ok(r: &Run) -> bool {
    r.code == Some(0) && !r.timed_out && !r.denied
}

fn tail(s: &str) -> String {
    let one: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let n = one.chars().count();
    one.chars().skip(n.saturating_sub(200)).collect()
}

async fn run(ex: &Executor, wt: &Path, command: &str) -> Run {
    let mut policy = SandboxPolicy::workspace(wt);
    policy.network = true;
    let spec = CommandSpec {
        command: command.into(),
        cwd: wt.to_path_buf(),
        timeout: Duration::from_secs(240),
        ..Default::default()
    };
    match ex.run(&spec, &policy).await {
        Ok(o) => Run {
            code: o.exit_code,
            timed_out: o.timed_out,
            denied: o.stdout.contains("Operation not permitted")
                || o.stderr.contains("Operation not permitted"),
            tail: tail(&format!("{} {}", o.stdout, o.stderr)),
        },
        Err(e) => Run {
            code: None,
            timed_out: false,
            denied: false,
            tail: tail(&e.to_string()),
        },
    }
}

fn worktree(root: &Path) -> std::path::PathBuf {
    let repo = root.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let git = |dir: &Path, args: &[&str]| {
        let s = Command::new("git")
            .args(args)
            .current_dir(dir)
            .status()
            .unwrap();
        assert!(s.success(), "git {args:?}");
    };
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("README.md"), "probe\n").unwrap();
    git(&repo, &["add", "-A"]);
    git(
        &repo,
        &[
            "-c",
            "user.name=p",
            "-c",
            "user.email=p@p",
            "commit",
            "-q",
            "-m",
            "init",
        ],
    );
    let wt = root.join("wt");
    git(
        &repo,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "agenium/PROBE-1",
            wt.to_str().unwrap(),
        ],
    );
    wt
}

#[tokio::main]
async fn main() {
    let out = std::env::args()
        .skip_while(|a| a != "--out")
        .nth(1)
        .map(std::path::PathBuf::from);
    if cfg!(windows) {
        println!("tool_probe runs Unix shell lines: meant for macOS and Linux");
        return;
    }
    let os = agenium_sandbox::detect_os();
    let mut report = format!("sandbox: {:?} - {}\n", os.backend, os.reason);
    let ex = Executor::new(Shell::detect());
    let (mut blocked, mut both_fail, mut fine) = (Vec::new(), Vec::new(), 0);
    for (name, command) in PROBES {
        let tmp = tempfile::tempdir().unwrap();
        let wt = worktree(tmp.path());
        set_off(false);
        let on = run(&ex, &wt, command).await;
        set_off(true);
        let off = run(&ex, &wt, command).await;
        set_off(false);
        agenium_sandbox::remove_run_temp_dir(&wt);
        let verdict = match (ok(&on), ok(&off)) {
            (true, true) => {
                fine += 1;
                "ok"
            }
            (false, true) => {
                blocked.push(*name);
                "BLOCKED BY SANDBOX"
            }
            (true, false) => "ok on, fails off (?)",
            (false, false) => {
                both_fail.push(*name);
                "fails both (missing tool?)"
            }
        };
        let line = format!(
            "{name:<30} {verdict:<26} on: exit={:?}{} | off: exit={:?}{}\n    on:  {}\n    off: {}\n",
            on.code,
            if on.timed_out { " TIMEOUT" } else { "" },
            off.code,
            if off.timed_out { " TIMEOUT" } else { "" },
            on.tail,
            off.tail,
        );
        print!("{line}");
        report.push_str(&line);
    }
    let summary = format!(
        "\nsummary: {fine} ok both ways; {} blocked by the sandbox: {}; {} fail both ways: {}\n",
        blocked.len(),
        blocked.join(", "),
        both_fail.len(),
        both_fail.join(", ")
    );
    print!("{summary}");
    report.push_str(&summary);
    if let Some(p) = out {
        std::fs::write(p, report).unwrap();
    }
}
