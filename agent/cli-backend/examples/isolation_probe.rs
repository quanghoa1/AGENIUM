//! Checks that a vendor CLI run by Agenium sees none of the user's own configuration: for each installed CLI it builds
//! the per-run home exactly as a run does (`Driver::prepare`), then asks the CLI, without a model turn, which MCP
//! servers (and, where the CLI can say, plugins, extensions or agents) it would load in Agenium's environment. The
//! user's own environment is not probed: listing MCP servers starts them. Anything listed that is not Agenium's or the
//! CLI's built-in is a leak. The per-run home (with its login copy) is handed back and removed.
//!
//! cargo run -p agenium-cli-backend --example isolation_probe -- <scratch dir>

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use agenium_cli_backend::{detect_all, driver, CliKind, Setup};

/// The listing subcommands per CLI (no model turn).
fn listings(kind: CliKind) -> &'static [&'static [&'static str]] {
    match kind {
        CliKind::Claude => &[&["mcp", "list"], &["plugin", "list"]],
        CliKind::Codex => &[&["mcp", "list"]],
        CliKind::Grok => &[&["mcp", "list"]],
        CliKind::Opencode => &[&["mcp", "list"], &["agent", "list"]],
        CliKind::Agy => &[&["mcp", "list"], &["extensions", "list"]],
        _ => &[],
    }
}

fn run(
    program: &Path,
    args: &[&str],
    cwd: &Path,
    env: Option<&BTreeMap<String, String>>,
) -> String {
    let mut cmd = Command::new(program);
    cmd.args(args).current_dir(cwd);
    if let Some(env) = env {
        cmd.env_clear().envs(env);
    }
    let start = Instant::now();
    let child = cmd
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn();
    let Ok(child) = child else {
        return "<cannot start>".into();
    };
    let out = child.wait_with_output();
    if start.elapsed() > Duration::from_secs(120) {
        return "<slow>".into();
    }
    match out {
        Ok(o) => format!(
            "{}{}",
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        ),
        Err(e) => format!("<{e}>"),
    }
}

/// Lines that name something (not blank, not a header or a "no servers" note).
fn items(out: &str) -> Vec<String> {
    out.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .filter(|l| {
            let l = l.to_lowercase();
            !(l.starts_with("no ")
                || l.contains("not found")
                || l.starts_with("checking")
                || l.starts_with("name")
                || l.starts_with("usage")
                || l.starts_with("error")
                || l.starts_with('-')
                || l.starts_with('┌')
                || l.starts_with('└')
                || l.starts_with('│') && l.len() < 4)
        })
        .map(str::to_owned)
        .collect()
}

fn main() {
    let scratch = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| ".isolation-probe".into()),
    );
    let worktree = scratch.join("worktree");
    std::fs::create_dir_all(&worktree).expect("scratch dir");
    for info in detect_all() {
        let (Some(path), Some(d)) = (info.path.clone(), driver(info.kind)) else {
            continue;
        };
        let program = d.program(&path);
        let home = scratch.join(format!("home-{}", info.kind.program()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).expect("home");
        let setup = Setup {
            home: &home,
            worktree: &worktree,
            mcp_name: "agenium",
            mcp_url: "http://127.0.0.1:9/mcp",
            mcp_authorization: "Bearer probe",
            hook_url: "http://127.0.0.1:9/hook",
            subagents: false,
            model: "",
            run_for: std::time::Duration::from_secs(2700),
        };
        let env = match d.prepare(&setup) {
            Ok(e) => agenium_sandbox::clean_env(&agenium_cli_backend::run::isolated_home(&home, e)),
            Err(e) => {
                println!("{}: prepare failed: {e}", info.kind.program());
                continue;
            }
        };
        for args in listings(info.kind) {
            let ours = items(&run(&program, args, &worktree, Some(&env)));
            println!(
                "{:<9} {:<18} {} line(s)",
                info.kind.program(),
                args.join(" "),
                ours.len()
            );
            for l in &ours {
                println!("    {}", l.chars().take(140).collect::<String>());
            }
        }
        // `--turn`: one short Claude turn with the run's real arguments; its `init` event lists what it loaded.
        if info.kind == CliKind::Claude && std::env::args().any(|a| a == "--turn") {
            let args = d.args("claude-haiku-5-5", None, &home, None, false);
            let mut child = Command::new(&program)
                .args(&args)
                .current_dir(&worktree)
                .env_clear()
                .envs(&env)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .spawn()
                .expect("claude");
            use std::io::{BufRead, Write};
            let mut stdin = child.stdin.take().unwrap();
            let _ = stdin.write_all(
                format!(
                    "{}
",
                    d.user_input("Reply with the single word OK.")
                )
                .as_bytes(),
            );
            let _ = stdin.flush();
            let out = std::io::BufReader::new(child.stdout.take().unwrap());
            for line in out.lines().map_while(Result::ok) {
                let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
                    continue;
                };
                if v["type"] == "system" && v["subtype"] == "init" {
                    let names = |k: &str| -> Vec<String> {
                        v[k].as_array()
                            .into_iter()
                            .flatten()
                            .map(|x| {
                                x.get("name")
                                    .and_then(|n| n.as_str())
                                    .or(x.as_str())
                                    .unwrap_or("?")
                                    .to_owned()
                            })
                            .collect()
                    };
                    let tools = names("tools");
                    println!("claude init: mcp_servers {:?}", names("mcp_servers"));
                    println!(
                        "claude init: {} tools ({} from MCP: {:?})",
                        tools.len(),
                        tools.iter().filter(|t| t.starts_with("mcp__")).count(),
                        tools
                            .iter()
                            .filter(|t| t.starts_with("mcp__") && !t.starts_with("mcp__agenium__"))
                            .collect::<Vec<_>>()
                    );
                    println!("claude init: plugins {}", v["plugins"]);
                    println!("claude init: skills {:?}", names("skills"));
                    println!("claude init: agents {:?}", names("agents"));
                    println!("claude init: slash_commands {:?}", names("slash_commands"));
                    let _ = child.kill();
                    break;
                }
            }
            drop(stdin);
            let _ = child.wait();
        }
        if let Err(e) = d.finish(&setup) {
            println!("{}: finish failed: {e}", info.kind.program());
        }
        let _ = std::fs::remove_dir_all(&home);
    }
    let _ = std::fs::remove_dir_all(&scratch);
}
