//! The file-size ratchet (owner, 2026-10-28: "chia nhỏ code ra từng module nhỏ cho dễ maintain … thay vì gộp mọi thứ
//! vào 1 file lớn"; the guard learned from jevgrep). A source file may hold at most `LIMIT_BYTES` (≈ 8 000 tokens at four
//! bytes a token). The files over it today are frozen at their size in `devtools/ratchet.json`: they may shrink, never
//! grow. `cargo xtask ratchet` checks (and `ci` runs it); `cargo xtask ratchet --update` records the sizes after a file
//! was split (it refuses to record growth). A file may opt out with `ratchet: allow — <reason>` in its first 10 lines,
//! when splitting it would cost more than it saves.
//!
//! Sizes are counted without `\r`, so a Windows checkout (CRLF) and a Linux one agree.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};

/// The most a source file may hold (bytes, `\r` not counted).
pub const LIMIT_BYTES: u64 = 32_000;
/// Sources the ratchet covers.
const EXTENSIONS: &[&str] = &["rs", "ts", "tsx", "js", "mjs", "py", "sh", "ps1"];
/// Paths it does not cover: code kept as upstream wrote it, generated files.
const EXCLUDED: &[&str] = &["vendor/", "apps/desktop/gen/", "apps/desktop/ui/dist/"];
const BASELINE: &str = "devtools/ratchet.json";

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The tracked source files, root-relative with `/`.
fn sources(root: &Path) -> Result<Vec<String>> {
    let out = Command::new("git")
        .args(["ls-files", "-z"])
        .current_dir(root)
        .output()
        .context("git ls-files")?;
    if !out.status.success() {
        bail!("git ls-files failed");
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .split('\0')
        .filter(|p| !p.is_empty())
        .filter(|p| {
            Path::new(p)
                .extension()
                .is_some_and(|e| EXTENSIONS.contains(&e.to_string_lossy().as_ref()))
        })
        .filter(|p| !EXCLUDED.iter().any(|x| p.starts_with(x)))
        .map(str::to_owned)
        .collect())
}

/// Size without `\r`, and whether the file opted out.
fn measure(path: &Path) -> Result<(u64, bool)> {
    let bytes = std::fs::read(path).with_context(|| path.display().to_string())?;
    let size = bytes.iter().filter(|b| **b != b'\r').count() as u64;
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]);
    let allowed = head.lines().take(10).any(|l| l.contains("ratchet: allow"));
    Ok((size, allowed))
}

fn load(root: &Path) -> Result<BTreeMap<String, u64>> {
    let p = root.join(BASELINE);
    if !p.exists() {
        return Ok(BTreeMap::new());
    }
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&p)?)?;
    Ok(v["frozen"]
        .as_object()
        .map(|o| {
            o.iter()
                .filter_map(|(k, v)| Some((k.clone(), v.as_u64()?)))
                .collect()
        })
        .unwrap_or_default())
}

fn save(root: &Path, frozen: &BTreeMap<String, u64>) -> Result<()> {
    let v = serde_json::json!({
        "_": "File-size ratchet (devtools/xtask/src/ratchet.rs): files over limit_bytes, frozen at their size. They may shrink, never grow; split them along their features.",
        "limit_bytes": LIMIT_BYTES,
        "frozen": frozen,
    });
    std::fs::write(
        root.join(BASELINE),
        serde_json::to_string_pretty(&v)? + "\n",
    )?;
    Ok(())
}

/// What the ratchet finds: (`path`, size, frozen size if any) for every file over the limit, and the problems.
struct Findings {
    over: Vec<(String, u64, Option<u64>)>,
    problems: Vec<String>,
    shrunk: Vec<String>,
}

fn find(root: &Path, frozen: &BTreeMap<String, u64>) -> Result<Findings> {
    let mut f = Findings {
        over: vec![],
        problems: vec![],
        shrunk: vec![],
    };
    for p in sources(root)? {
        let (size, allowed) = measure(&root.join(&p))?;
        if allowed {
            continue;
        }
        let was = frozen.get(&p).copied();
        if size > LIMIT_BYTES {
            match was {
                None => f.problems.push(format!(
                    "{p}: {size} bytes, over the {LIMIT_BYTES}-byte limit: split it along its features"
                )),
                Some(w) if size > w => f.problems.push(format!(
                    "{p}: grew from {w} to {size} bytes; it is over the limit and may only shrink: split it"
                )),
                Some(w) if size < w => f.shrunk.push(format!("{p}: {w} -> {size} bytes")),
                _ => {}
            }
            f.over.push((p, size, was));
        } else if was.is_some() {
            f.shrunk
                .push(format!("{p}: now within the limit ({size} bytes)"));
        }
    }
    Ok(f)
}

/// `cargo xtask ratchet [--update]`.
pub fn ratchet(update: bool) -> Result<()> {
    let root = root();
    let frozen = load(&root)?;
    let f = find(&root, &frozen)?;
    if update {
        let grew: Vec<&String> = f
            .problems
            .iter()
            .filter(|p| p.contains(" grew from "))
            .collect();
        if !grew.is_empty() {
            bail!(
                "refusing to record growth:\n  {}",
                grew.iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join("\n  ")
            );
        }
        let next: BTreeMap<String, u64> = f.over.iter().map(|(p, s, _)| (p.clone(), *s)).collect();
        save(&root, &next)?;
        println!(
            "ratchet: {} file(s) frozen over {LIMIT_BYTES} bytes",
            next.len()
        );
        for s in &f.shrunk {
            println!("  smaller: {s}");
        }
        return Ok(());
    }
    for s in &f.shrunk {
        println!("ratchet: smaller, record it with `cargo xtask ratchet --update`: {s}");
    }
    if !f.problems.is_empty() {
        bail!("ratchet:\n  {}", f.problems.join("\n  "));
    }
    println!(
        "ratchet: ok ({} file(s) over {LIMIT_BYTES} bytes, none grew)",
        f.over.len()
    );
    Ok(())
}
