//! Runs the Jev `skill_quality` judge (ADR-88) over the built-in skill library and prints its verdict per skill:
//! one job, actionable steps, proof demands evidence, no stale code. Use to sanity-check skill scope (are any too
//! broad / several jobs in one). Usage:
//!   cargo run -p agenium-eval --example skill_quality -- --root <root-with-jev>
//! Reads the judge key from that root (costs Jev calls). Not a test; run by hand.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{anyhow, Result};

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let root = PathBuf::from(arg(&args, "--root").ok_or_else(|| anyhow!("--root required"))?);
    let keyring: Arc<dyn agenium_secrets::KeyringBackend> =
        Arc::new(agenium_secrets::OsKeyring::default());
    let (app, _) = agenium_orchestrator::App::open(
        &root,
        agenium_orchestrator::AppOptions {
            keyring,
            checks: Arc::new(agenium_orchestrator::NoCheckFactory),
        },
    )?;
    // --dir reads SKILL.md straight from a folder (e.g. the repo's templates), bypassing the root's seeded copies.
    let dir = match arg(&args, "--dir") {
        Some(d) => PathBuf::from(d),
        None => app.skill_registry().dir_of(&agenium_skills::Owner::Library),
    };
    let only: Vec<String> = arg(&args, "--only")
        .map(|s| s.split(',').map(str::to_owned).collect())
        .unwrap_or_default();
    let mut names: Vec<String> = std::fs::read_dir(&dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().join("SKILL.md").is_file())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| only.is_empty() || only.contains(n))
        .collect();
    names.sort();
    println!("{} skills in {}\n", names.len(), dir.display());
    for name in names {
        let body = agenium_orchestrator::skill_shape::strip_frontmatter(&std::fs::read_to_string(
            dir.join(&name).join("SKILL.md"),
        )?);
        match app.skill_quality(&body).await {
            Some(notes) => println!("{name:<24} {}", notes.join(" · ")),
            None => {
                println!("{name:<24} (judge off or unavailable)");
            }
        }
    }
    Ok(())
}
