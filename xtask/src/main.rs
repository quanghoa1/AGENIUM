//! Repository automation: `cargo xtask schemas [--check]`, `cargo xtask ci`.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf()
}

/// Files generated from Rust: tool argument schemas (model facing) and the canonical SQL migrations.
fn generated() -> Vec<(PathBuf, String)> {
    let mut out = Vec::new();
    let tools = root().join("schemas/tools");
    let mut index = serde_json::Map::new();
    for group in ["Lead", "Worker", "Shared"] {
        index.insert(group.into(), serde_json::Value::Array(vec![]));
    }
    for t in agenium_core::ProtocolTool::ALL {
        let mut v = t.parameters();
        if let Some(o) = v.as_object_mut() {
            o.insert(
                "$schema".into(),
                "https://json-schema.org/draft/2020-12/schema".into(),
            );
            o.insert(
                "$id".into(),
                format!(
                    "https://agenium.dev/schemas/v1/tools/{}.schema.json",
                    t.name()
                )
                .into(),
            );
            o.insert("title".into(), t.name().into());
            o.insert("description".into(), t.description().into());
            o.insert("x-agenium-role".into(), format!("{:?}", t.group()).into());
        }
        out.push((
            tools.join(format!("{}.schema.json", t.name())),
            serde_json::to_string_pretty(&v).unwrap() + "\n",
        ));
        index[&format!("{:?}", t.group())]
            .as_array_mut()
            .unwrap()
            .push(t.name().into());
    }
    index.insert("Solo".into(), "Lead + Worker + Shared".into());
    out.push((
        tools.join("index.json"),
        serde_json::to_string_pretty(&index).unwrap() + "\n",
    ));
    let sql = root().join("crates/agenium-store/src/schema");
    for f in [
        "core_001.sql",
        "core_002.sql",
        "agent_001.sql",
        "agent_002.sql",
    ] {
        let text = std::fs::read_to_string(sql.join(f)).unwrap();
        out.push((root().join("schemas/sql").join(f), text));
    }
    out
}

fn schemas(check: bool) -> Result<()> {
    let mut stale = Vec::new();
    for (path, content) in generated() {
        let cur = std::fs::read_to_string(&path)
            .unwrap_or_default()
            .replace("\r\n", "\n");
        if cur != content {
            if check {
                stale.push(path.display().to_string());
            } else {
                std::fs::create_dir_all(path.parent().unwrap())?;
                std::fs::write(&path, content).with_context(|| path.display().to_string())?;
                println!("wrote {}", path.display());
            }
        }
    }
    if !stale.is_empty() {
        bail!(
            "generated schemas are stale (run `cargo xtask schemas`):\n  {}",
            stale.join("\n  ")
        );
    }
    println!("schemas up to date");
    Ok(())
}

fn cargo(args: &[&str]) -> Result<()> {
    let st = Command::new(env!("CARGO"))
        .args(args)
        .current_dir(root())
        .status()?;
    if !st.success() {
        bail!("cargo {} failed", args.join(" "));
    }
    Ok(())
}

/// Vendored Codex sources must stay byte-for-byte upstream (`vendor/codex-sandbox/FILES.lock`).
fn vendor_check() -> Result<()> {
    use sha2::{Digest, Sha256};
    let dir = root().join("vendor/codex-sandbox");
    let lock = std::fs::read_to_string(dir.join("FILES.lock"))?;
    let mut bad = Vec::new();
    let mut n = 0;
    for line in lock
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
    {
        let (path, want) = line.trim().rsplit_once(' ').unwrap_or((line, ""));
        n += 1;
        match std::fs::read(dir.join("upstream").join(path)) {
            Ok(bytes) => {
                let got: String = Sha256::digest(&bytes)
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect();
                if got != want {
                    bad.push(format!("MODIFIED {path}"));
                }
            }
            Err(_) => bad.push(format!("MISSING {path}")),
        }
    }
    let listed = std::fs::read_to_string(dir.join("FILES.txt"))?;
    for path in listed
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
    {
        if !lock.lines().any(|l| l.starts_with(&format!("{path} "))) {
            bad.push(format!(
                "UNLOCKED {path} (run scripts/sync-codex-sandbox.sh <ref> --update)"
            ));
        }
    }
    if !bad.is_empty() {
        bail!(
            "vendor/codex-sandbox drifted from upstream:
{}",
            bad.join(
                "
"
            )
        );
    }
    println!("vendor/codex-sandbox: {n} files match FILES.lock");
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("schemas") => schemas(args.iter().any(|a| a == "--check")),
        Some("vendor-check") => vendor_check(),
        Some("ci") => {
            cargo(&["fmt", "--all", "--check"])?;
            cargo(&[
                "clippy",
                "--workspace",
                "--all-targets",
                "--",
                "-D",
                "warnings",
            ])?;
            cargo(&["test", "--workspace"])?;
            schemas(true)?;
            vendor_check()
        }
        _ => bail!("usage: cargo xtask <schemas [--check] | vendor-check | ci>"),
    }
}
