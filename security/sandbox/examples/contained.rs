//! Diagnostics: runs a command inside the sandbox with the policy a CLI backend gets (worktree and extra roots
//! writable, network on, extra read roots), printing its output.
//! `cargo run -p agenium-sandbox --example contained -- <cwd> <write-root,...> <read-root,...> -- <argv...>`
//! Environment variables `CONTAINED_<NAME>=value` are passed in as `<NAME>=value`; stdin is forwarded.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let split = args
        .iter()
        .position(|a| a == "--")
        .expect("-- before the command");
    let (opts, argv) = (&args[..split], &args[split + 1..]);
    let cwd = PathBuf::from(&opts[0]);
    let list = |i: usize| -> Vec<PathBuf> {
        opts.get(i)
            .map(|s| {
                s.split(',')
                    .filter(|x| !x.is_empty())
                    .map(PathBuf::from)
                    .collect()
            })
            .unwrap_or_default()
    };
    let mut policy = agenium_sandbox::SandboxPolicy::workspace(&cwd);
    policy.network = true;
    policy.writable_roots.extend(list(1));
    policy.read_roots.extend(list(2));
    let mut env: BTreeMap<String, String> = std::env::vars()
        .filter_map(|(k, v)| k.strip_prefix("CONTAINED_").map(|k| (k.to_owned(), v)))
        .collect();
    for k in [
        "PATH",
        "SystemRoot",
        "TEMP",
        "TMP",
        "USERPROFILE",
        "APPDATA",
        "LOCALAPPDATA",
    ] {
        if let Ok(v) = std::env::var(k) {
            env.entry(k.to_owned()).or_insert(v);
        }
    }
    let (mut child, io) = agenium_sandbox::service::spawn_contained(argv, &cwd, &env, &policy)
        .expect("start contained");
    if let Some(mut stdin) = io.stdin {
        let mut input = String::new();
        let _ = std::io::stdin().read_to_string(&mut input);
        let _ = stdin.write_all(input.as_bytes());
        drop(stdin);
    }
    let err = io.stderr.map(|mut e| {
        std::thread::spawn(move || {
            let mut s = String::new();
            let _ = e.read_to_string(&mut s);
            s
        })
    });
    if let Some(mut out) = io.stdout {
        let mut s = String::new();
        let _ = out.read_to_string(&mut s);
        println!("{s}");
    }
    if let Some(e) = err {
        eprintln!("--- stderr ---\n{}", e.join().unwrap_or_default());
    }
    child.kill();
}
