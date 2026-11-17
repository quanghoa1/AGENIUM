//! Real check that a private plugin repository's token is kept for its updates (ADR-157 amendment, 2026-11-17):
//! 1. without a token the private repository is refused;
//! 2. installed with the token, the token is kept (the install record does not hold it);
//! 3. an update preview with no token pasted reads the repository with the kept one;
//! 4. removing the plugin forgets the token: an update preview of a new install without one is refused again.
//!
//! AGENIUM_TEST_GH_TOKEN=<token> cargo run -p agenium-eval --example private_plugin_token -- owner/repo
//! (the token comes from the environment only; it is never printed)

use std::sync::Arc;

fn check(fails: &mut u32, name: &str, ok: bool, seen: &str) {
    if !ok {
        *fails += 1;
    }
    println!(
        "{} {name}{}",
        if ok { "PASS" } else { "FAIL" },
        if seen.is_empty() {
            String::new()
        } else {
            format!(": {seen}")
        }
    );
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let repo = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "quanghoa1/agenium-pentest".into());
    let token = std::env::var("AGENIUM_TEST_GH_TOKEN").map_err(|_| "set AGENIUM_TEST_GH_TOKEN")?;
    let tmp = tempfile::tempdir()?;
    let (app, _) = agenium_orchestrator::App::open(
        &tmp.path().join("root"),
        agenium_orchestrator::AppOptions {
            keyring: Arc::new(agenium_secrets::MemoryKeyring::default()),
            checks: Arc::new(agenium_orchestrator::NoCheckFactory),
        },
    )?;
    // Every message below goes through this: the token must never show.
    let clean = |s: String| s.replace(token.as_str(), "<TOKEN>");
    let mut fails = 0;

    // 1. Refused without a token.
    let r = app.preview_plugin(&repo, None, None).await;
    check(
        &mut fails,
        "a private repository is refused without a token",
        r.is_err(),
        &clean(r.err().map(|e| e.to_string()).unwrap_or_default()),
    );

    // 2. Installed with the pasted token.
    let p = app.preview_plugin(&repo, None, Some(&token)).await?;
    let plugin = app
        .install_plugin(
            &p.plugin.source,
            &p.plugin.reference,
            p.plugin.entry.as_deref(),
            &p.plugin.commit,
            Some(&token),
        )
        .await?;
    let listed = app.plugin(&plugin.name).ok_or("not listed")?;
    check(
        &mut fails,
        "installed with the token; it is kept",
        listed.private && listed.token_kept,
        &format!(
            "{} @ {}, {} skills",
            plugin.name,
            &plugin.commit[..7.min(plugin.commit.len())],
            plugin.skills.len()
        ),
    );
    let mut on_disk = String::new();
    for e in walk(&tmp.path().join("root")) {
        if let Ok(t) = std::fs::read(&e) {
            if String::from_utf8_lossy(&t).contains(token.as_str()) {
                on_disk.push_str(&e.display().to_string());
            }
        }
    }
    check(
        &mut fails,
        "the token is in no file under the root (the test keyring is in memory)",
        on_disk.is_empty(),
        &on_disk,
    );

    // 3. An update with nothing pasted.
    let u = app.preview_plugin_update(&plugin.name, None).await;
    check(
        &mut fails,
        "an update reads the repository with the kept token",
        u.as_ref().is_ok_and(|u| u.plugin.private),
        &clean(match &u {
            Ok(u) => format!(
                "vendor at {}",
                &u.plugin.commit[..7.min(u.plugin.commit.len())]
            ),
            Err(e) => e.to_string(),
        }),
    );

    // 4. Removed: the token goes.
    app.remove_plugin(&plugin.name)?;
    let again = app.preview_plugin(&repo, None, None).await;
    check(
        &mut fails,
        "removing the last plugin forgets the token",
        again.is_err(),
        &clean(again.err().map(|e| e.to_string()).unwrap_or_default()),
    );

    println!(
        "{}",
        if fails == 0 {
            "ALL PASS"
        } else {
            "SOME FAILED"
        }
    );
    std::process::exit(i32::from(fails > 0));
}

fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(walk(&p));
        } else {
            out.push(p);
        }
    }
    out
}
