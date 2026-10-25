//! Containment check of the shipping sandbox on Linux (bubblewrap) and macOS (Seatbelt) — the S3 attack list run against
//! the backend agents actually get, not a spike launcher (Windows: `docs/spike-reports/S2`).
//!
//! Each mode builds a fresh scenario in a temp dir: a git project with a ticket worktree, an `outside` folder with victim
//! files, a fake home with a secret, a TCP listener and a unix socket listening on the host, and a canary variable in
//! this process's environment. `containment_payload.py` then runs through `Executor::run` with the worktree policy, and
//! the verdict is taken **on the host** wherever possible (a write can succeed inside a private tmpfs and still not
//! escape). A double-forked daemon checks that nothing outlives the run, also after a timeout.
//!
//! `cargo run -p agenium-sandbox --example containment [-- --baseline]` (`--baseline` adds a run without the sandbox,
//! which shows that the payload can tell an escape when there is one). Exit code 1 on any escape or broken must-work.

#[cfg(not(unix))]
fn main() {
    eprintln!("containment: Linux and macOS only (Windows: docs/spike-reports/S2)");
}

#[cfg(unix)]
fn main() {
    std::process::exit(posix::main());
}

#[cfg(unix)]
mod posix {
    use std::collections::{BTreeMap, HashMap};
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::time::{Duration, Instant};

    use agenium_sandbox::{
        detect, network_report, toolchain_read_roots, CommandSpec, Executor, SandboxMode,
        SandboxPolicy, Shell,
    };
    use serde_json::{json, Value};

    const PAYLOAD: &str = include_str!("containment_payload.py");
    const CANARY: &str = "AGENIUM_CANARY_SECRET";
    const SECRET: &[u8] = b"-----BEGIN AGENIUM CANARY KEY-----";

    struct Scenario {
        _tmp: tempfile::TempDir,
        root: PathBuf,
        wt: PathBuf,
        outside: PathBuf,
        home: PathBuf,
        main_git: PathBuf,
        tag: String,
        tcp_port: u16,
        victim_mode: u32,
        victim_mtime: i64,
        git_config: String,
    }

    fn git(dir: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_AUTHOR_NAME", "containment")
            .env("GIT_AUTHOR_EMAIL", "containment@agenium.local")
            .env("GIT_COMMITTER_NAME", "containment")
            .env("GIT_COMMITTER_EMAIL", "containment@agenium.local")
            .status()
            .is_ok_and(|s| s.success());
        assert!(ok, "git {args:?}");
    }

    fn scenario(label: &str) -> Scenario {
        let tmp = tempfile::tempdir().unwrap();
        // Canonical paths: what the sandbox sees (macOS: /var -> /private/var).
        let root = std::fs::canonicalize(tmp.path()).unwrap();
        let tag = format!(
            "agenium-containment-{}-{label}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis()
        );
        let outside = root.join("outside");
        let home = root.join("home");
        let project = root.join("project");
        for d in [
            &outside,
            &home.join(".ssh"),
            &project,
            &root.join("markers"),
        ] {
            std::fs::create_dir_all(d).unwrap();
        }
        // One victim per attack, so one attack cannot hide the evidence of another.
        for v in [
            "victim_hardlink",
            "victim_truncate",
            "victim_meta",
            "victim2",
        ] {
            std::fs::write(outside.join(format!("{v}.txt")), "original victim").unwrap();
        }
        std::fs::write(home.join(".ssh").join("id_ed25519"), SECRET).unwrap();
        std::fs::write(project.join("README.md"), "# project\n").unwrap();
        git(&project, &["init", "-q", "-b", "main"]);
        git(&project, &["add", "-A"]);
        git(&project, &["commit", "-q", "-m", "init"]);
        let wt = project.join(".agenium").join("worktrees").join("T1");
        git(
            &project,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "agenium/T1",
                wt.to_str().unwrap(),
            ],
        );
        std::fs::write(wt.join("containment_payload.py"), PAYLOAD).unwrap();

        // Listeners on the host: a connection leaves a marker file.
        let markers = root.join("markers");
        let tcp = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let tcp_port = tcp.local_addr().unwrap().port();
        let m = markers.clone();
        std::thread::spawn(move || {
            for s in tcp.incoming().flatten() {
                let _ = std::fs::write(m.join("tcp"), "connected");
                drop(s);
            }
        });
        let unix = std::os::unix::net::UnixListener::bind(outside.join("daemon.sock")).unwrap();
        let m = markers.clone();
        std::thread::spawn(move || {
            for s in unix.incoming().flatten() {
                let _ = std::fs::write(m.join("unix"), "connected");
                drop(s);
            }
        });
        let meta = std::fs::metadata(outside.join("victim_meta.txt")).unwrap();
        let git_config = std::fs::read_to_string(project.join(".git").join("config")).unwrap();
        Scenario {
            _tmp: tmp,
            root,
            wt,
            outside,
            home,
            main_git: project.join(".git"),
            tag,
            tcp_port,
            victim_mode: meta.permissions().mode(),
            victim_mtime: meta.mtime(),
            git_config,
        }
    }

    fn targets(s: &Scenario) -> String {
        let real_home = std::env::var("HOME").unwrap_or_default();
        let real_ssh = Path::new(&real_home).join(".ssh");
        json!({
            "outside": s.outside, "home": s.home, "main_git": s.main_git, "tag": s.tag,
            "victim_hardlink": s.outside.join("victim_hardlink.txt"), "victim_truncate": s.outside.join("victim_truncate.txt"),
            "victim_meta": s.outside.join("victim_meta.txt"), "victim2": s.outside.join("victim2.txt"),
            "secret": s.home.join(".ssh").join("id_ed25519"),
            "real_home": real_home, "real_ssh": if real_ssh.is_dir() { Value::from(real_ssh.display().to_string()) } else { Value::Null },
            "tcp_port": s.tcp_port, "unix_sock": s.outside.join("daemon.sock"),
            "host_pid": std::process::id(), "canary_name": CANARY,
        })
        .to_string()
    }

    /// The daemon's heartbeat: `Some(pid, still beating)` once it has started.
    fn daemon_alive(wt: &Path) -> Option<(u32, bool)> {
        let read = || std::fs::read_to_string(wt.join("heartbeat")).ok();
        std::thread::sleep(Duration::from_millis(1500));
        let a = read()?;
        std::thread::sleep(Duration::from_millis(1000));
        let b = read().unwrap_or_default();
        let pid = a.split_whitespace().next()?.parse().ok()?;
        Some((pid, a != b))
    }

    fn kill(pid: u32) {
        let _ = Command::new("kill").arg("-9").arg(pid.to_string()).status();
    }

    struct Mode {
        name: &'static str,
        policy: fn(&Path) -> SandboxPolicy,
        network: bool,
    }

    fn sandbox_off_net(wt: &Path) -> SandboxPolicy {
        SandboxPolicy::workspace(wt)
    }

    fn sandbox_on_net(wt: &Path) -> SandboxPolicy {
        let mut p = SandboxPolicy::workspace(wt);
        p.network = true;
        p
    }

    fn no_sandbox(wt: &Path) -> SandboxPolicy {
        let mut p = SandboxPolicy::workspace(wt);
        p.mode = SandboxMode::FullAccess;
        p
    }

    async fn run_mode(m: &Mode, shell: &Shell) -> BTreeMap<String, String> {
        let s = scenario(m.name);
        let policy = (m.policy)(&s.wt);
        let spec = |cmd: &str, secs: u64| CommandSpec {
            command: cmd.into(),
            cwd: s.wt.clone(),
            timeout: Duration::from_secs(secs),
            env: [("AGENIUM_TARGETS".to_owned(), targets(&s))].into(),
        };
        let exec = Executor::new(shell.clone());
        let started = Instant::now();
        let out = exec
            .run(&spec("python3 containment_payload.py", 120), &policy)
            .await;
        let mut v: BTreeMap<String, String> = BTreeMap::new();
        let inside: HashMap<String, (bool, String)> = match &out {
            Ok(o) => {
                let line = o
                    .stdout
                    .lines()
                    .rev()
                    .find(|l| l.trim_start().starts_with('{'))
                    .unwrap_or("{}");
                let parsed: HashMap<String, (String, String)> =
                    serde_json::from_str(line).unwrap_or_default();
                if parsed.is_empty() {
                    eprintln!(
                        "[{}] payload printed no result (exit {:?}):\n{}\n{}",
                        m.name, o.exit_code, o.stdout, o.stderr
                    );
                }
                parsed
                    .into_iter()
                    .map(|(k, (r, d))| (k, (r == "did", d)))
                    .collect()
            }
            Err(e) => {
                eprintln!("[{}] run failed: {e}", m.name);
                HashMap::new()
            }
        };
        let did = |n: &str| inside.get(n).is_some_and(|x| x.0);
        let detail = |n: &str| inside.get(n).map(|x| x.1.clone()).unwrap_or_default();
        let verdict = |escaped: bool| if escaped { "ESCAPED" } else { "blocked" }.to_owned();
        // A verdict that only the payload can give: no result is not "blocked".
        let seen = |n: &str, escaped: bool| {
            if inside.contains_key(n) {
                verdict(escaped)
            } else {
                "no result".to_owned()
            }
        };
        let net = |escaped: bool| {
            if !escaped {
                "blocked".to_owned()
            } else if m.network {
                "allowed (network on)".to_owned()
            } else {
                "ESCAPED".to_owned()
            }
        };
        let o = &s.outside;
        let meta = std::fs::metadata(o.join("victim_meta.txt")).ok();
        let dotdot = {
            let mut p = s.wt.clone();
            for _ in 0..4 {
                p.pop();
            }
            p.join(format!("{}-dotdot.txt", s.tag))
        };
        let host_tmp = PathBuf::from("/tmp").join(format!("{}-tmp.txt", s.tag));
        v.insert(
            "write.outside_dir".into(),
            verdict(o.join("new.txt").exists()),
        );
        v.insert(
            "write.home".into(),
            verdict(s.home.join("new.txt").exists()),
        );
        v.insert("write.dotdot".into(), verdict(dotdot.exists()));
        v.insert("write.tmp".into(), verdict(host_tmp.exists()));
        let _ = std::fs::remove_file(&host_tmp);
        v.insert(
            "write.main_git_hook".into(),
            verdict(s.main_git.join("hooks").join("pre-commit").exists()),
        );
        v.insert(
            "write.main_git_config".into(),
            verdict(
                std::fs::read_to_string(s.main_git.join("config")).unwrap_or_default()
                    != s.git_config,
            ),
        );
        v.insert(
            "write.via_symlink".into(),
            verdict(o.join("via_symlink.txt").exists()),
        );
        v.insert(
            "write.via_hardlink".into(),
            verdict(
                std::fs::read_to_string(o.join("victim_hardlink.txt"))
                    .unwrap_or_default()
                    .contains("hardlinked"),
            ),
        );
        v.insert(
            "write.rename_out".into(),
            verdict(o.join("moved.txt").exists()),
        );
        v.insert(
            "write.delete_outside".into(),
            verdict(!o.join("victim2.txt").exists()),
        );
        v.insert(
            "write.truncate_outside".into(),
            verdict(
                std::fs::read_to_string(o.join("victim_truncate.txt")).is_ok_and(|t| t.is_empty()),
            ),
        );
        v.insert(
            "meta.chmod_outside".into(),
            verdict(
                meta.as_ref()
                    .is_some_and(|m| m.permissions().mode() != s.victim_mode),
            ),
        );
        v.insert(
            "meta.utime_outside".into(),
            verdict(
                meta.as_ref()
                    .is_some_and(|m| m.mtime() != s.victim_mtime && m.mtime() == 0),
            ),
        );
        let secret = format!(
            "{} {}",
            SECRET.len(),
            SECRET[..12]
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        );
        v.insert(
            "read.secret_outside".into(),
            seen(
                "read.secret_outside",
                detail("read.secret_outside") == secret,
            ),
        );
        // The real home is not visible; folders the toolchain needs (`~/.cargo`, `~/.rustup`) are mounted and show up as
        // their mount points. Anything else listed there is an escape.
        let home = PathBuf::from(std::env::var("HOME").unwrap_or_default());
        let mounted: Vec<String> = toolchain_read_roots()
            .iter()
            .filter_map(|r| r.strip_prefix(&home).ok())
            .filter_map(|r| r.components().next())
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        let listing = detail("read.real_home_listing");
        let only_mounts = listing
            .trim_matches(|c| c == '[' || c == ']')
            .split(", ")
            .map(|n| n.trim_matches('\''))
            .filter(|n| !n.is_empty())
            .all(|n| mounted.iter().any(|m| m == n));
        v.insert(
            "read.real_home_listing".into(),
            if did("read.real_home_listing") && only_mounts {
                format!("blocked (sees only the toolchain mounts {listing})")
            } else {
                seen("read.real_home_listing", did("read.real_home_listing"))
            },
        );
        for n in [
            "read.real_ssh_dir",
            "read.etc_shadow",
            "read.host_process_environ",
            "env.canary",
        ] {
            if inside.contains_key(n) || inside.is_empty() {
                v.insert(n.into(), seen(n, did(n)));
            }
        }
        v.insert(
            "net.tcp_host_listener".into(),
            net(s.root.join("markers").join("tcp").exists()),
        );
        for n in ["net.tcp_internet", "net.dns"] {
            v.insert(
                n.into(),
                if inside.contains_key(n) {
                    net(did(n))
                } else {
                    "no result".into()
                },
            );
        }
        v.insert(
            "net.unix_host_socket".into(),
            verdict(s.root.join("markers").join("unix").exists()),
        );
        for (n, (ok, d)) in inside.iter().filter(|(n, _)| n.starts_with("ok.")) {
            v.insert(
                n.clone(),
                if *ok {
                    "ok".into()
                } else {
                    format!("FAIL ({d})")
                },
            );
        }
        match daemon_alive(&s.wt) {
            Some((pid, true)) => {
                kill(pid);
                v.insert("daemon.outlives_the_run".into(), "ESCAPED".into());
            }
            Some((_, false)) => {
                v.insert("daemon.outlives_the_run".into(), "blocked".into());
            }
            None => {
                v.insert(
                    "daemon.outlives_the_run".into(),
                    format!("not started ({})", detail("daemon.spawned")),
                );
            }
        }
        // The same daemon, but the run is killed by its timeout while the payload still runs.
        let _ = std::fs::remove_file(s.wt.join("heartbeat"));
        let hung = exec
            .run(&spec("python3 containment_payload.py --hang", 4), &policy)
            .await;
        let timed_out = hung.as_ref().is_ok_and(|o| o.timed_out);
        let after = match daemon_alive(&s.wt) {
            Some((pid, true)) => {
                kill(pid);
                "ESCAPED".to_owned()
            }
            Some((_, false)) => "blocked".to_owned(),
            None => "not started".to_owned(),
        };
        v.insert(
            "daemon.outlives_a_timeout".into(),
            if timed_out {
                after
            } else {
                format!(
                    "{after} (run did not time out: {:?})",
                    hung.map(|o| o.exit_code)
                )
            },
        );
        eprintln!(
            "[{}] done in {:.1} s",
            m.name,
            started.elapsed().as_secs_f64()
        );
        // What the payload saw, for every verdict worth a look.
        for (n, r) in &v {
            if r.starts_with("ESCAPED") || r.starts_with("allowed") || r.starts_with("FAIL") {
                if let Some((_, d)) = inside.get(n) {
                    eprintln!("[{}] {n}: {r} — inside: {d}", m.name);
                }
            }
        }
        v
    }

    pub fn main() -> i32 {
        // A secret in Agenium's own environment that no sandboxed command may see (edition 2021: safe).
        std::env::set_var(CANARY, "agenium-canary-value");
        let backend = detect();
        println!(
            "platform: {} | backend: {:?} | shell: {}",
            std::env::consts::OS,
            backend.backend,
            Shell::detect().describe()
        );
        println!("network = true gives: {}", network_report());
        if backend.backend.is_none() {
            println!(
                "no sandbox backend here: nothing to check (Agenium is fail-closed without one)"
            );
            return 1;
        }
        let mut modes = vec![
            Mode {
                name: "sandbox, network off",
                policy: sandbox_off_net,
                network: false,
            },
            Mode {
                name: "sandbox, network on",
                policy: sandbox_on_net,
                network: true,
            },
        ];
        if std::env::args().any(|a| a == "--baseline") {
            modes.insert(
                0,
                Mode {
                    name: "no sandbox (baseline)",
                    policy: no_sandbox,
                    network: true,
                },
            );
        }
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        let shell = Shell::detect();
        let results: Vec<BTreeMap<String, String>> = modes
            .iter()
            .map(|m| rt.block_on(run_mode(m, &shell)))
            .collect();
        let mut names: Vec<&String> = results.iter().flat_map(|r| r.keys()).collect();
        names.sort();
        names.dedup();
        println!();
        println!(
            "| test | {} |",
            modes.iter().map(|m| m.name).collect::<Vec<_>>().join(" | ")
        );
        println!("|---|{}", "---|".repeat(modes.len()));
        for n in &names {
            let cells: Vec<String> = results
                .iter()
                .map(|r| r.get(*n).cloned().unwrap_or_else(|| "-".into()))
                .collect();
            println!("| {n} | {} |", cells.join(" | "));
        }
        let mut bad = 0;
        for (m, r) in modes.iter().zip(&results) {
            if m.name.starts_with("no sandbox") {
                continue;
            }
            bad += r
                .values()
                .filter(|x| x.starts_with("ESCAPED") || x.starts_with("FAIL"))
                .count();
        }
        println!();
        println!("sandboxed modes: {bad} escape(s) or broken must-work item(s)");
        i32::from(bad > 0)
    }
}
