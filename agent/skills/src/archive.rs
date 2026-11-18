//! A skill's folder on disk: walked within the limits (no link followed), copied, and taken out of a zip.

use std::io::Read;
use std::path::{Path, PathBuf};

use super::*;

pub(crate) fn walk(dir: &Path, base: &Path, out: &mut Vec<String>, total: &mut u64) -> Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)?.collect::<std::io::Result<_>>()?;
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let ft = e.file_type()?;
        let path = e.path();
        if ft.is_symlink() {
            return Err(invalid(format!(
                "symbolic links are not allowed in a skill: {}",
                path.display()
            )));
        }
        if ft.is_dir() {
            walk(&path, base, out, total)?;
        } else if ft.is_file() {
            *total += e.metadata()?.len();
            let rel = path.strip_prefix(base).unwrap_or(&path);
            out.push(rel.to_string_lossy().replace('\\', "/"));
            if out.len() > MAX_FILES {
                return Err(invalid(format!(
                    "a skill may hold at most {MAX_FILES} files"
                )));
            }
            if *total > MAX_TOTAL_BYTES {
                return Err(invalid(format!(
                    "a skill may hold at most {} MiB",
                    MAX_TOTAL_BYTES >> 20
                )));
            }
        }
    }
    Ok(())
}

/// SKILL.md at the staging root, or inside its single top folder.
pub(crate) fn skill_root(staging: &Path) -> Result<PathBuf> {
    if staging.join(SKILL_FILE).is_file() {
        // Move the content one level down so the folder can be renamed to the skill name.
        let inner = staging.join(".skill");
        std::fs::create_dir(&inner)?;
        for e in std::fs::read_dir(staging)? {
            let e = e?;
            if e.file_name() != ".skill" {
                std::fs::rename(e.path(), inner.join(e.file_name()))?;
            }
        }
        return Ok(inner);
    }
    let dirs: Vec<_> = std::fs::read_dir(staging)?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .collect();
    match dirs.as_slice() {
        [one] if one.path().join(SKILL_FILE).is_file() => Ok(one.path()),
        _ => Err(invalid(format!(
            "no {SKILL_FILE} at the root or in a single top folder"
        ))),
    }
}

/// Copies a folder under the skill limits (no symbolic links, at most 1000 files and 50 MiB).
pub fn copy_dir(src: &Path, dst: &Path) -> Result<()> {
    let mut files = Vec::new();
    let mut total = 0;
    walk(src, src, &mut files, &mut total)?; // rejects symlinks and enforces limits before copying
    for f in files {
        let from = src.join(&f);
        let to = dst.join(&f);
        if let Some(p) = to.parent() {
            std::fs::create_dir_all(p)?;
        }
        std::fs::copy(from, to)?;
    }
    Ok(())
}

/// An entry's name for messages (an undecodable name is shown as `?`).
fn entry_name<R: std::io::Read>(e: &zip::read::ZipFile<'_, R>) -> String {
    e.name()
        .map(|n| n.into_owned())
        .unwrap_or_else(|_| "?".into())
}

/// Unpacks a `.zip` into `dst` under the skill limits (no symbolic links, no entry leaving the folder, at most
/// 50 MiB written).
pub fn extract_zip(src: &Path, dst: &Path) -> Result<()> {
    let f = std::fs::File::open(src)?;
    let mut z = zip::ZipArchive::new(f).map_err(|e| SkillError::Zip(e.to_string()))?;
    if z.len() > MAX_FILES * 2 {
        return Err(invalid(format!(
            "the archive holds more than {} entries",
            MAX_FILES * 2
        )));
    }
    let mut total = 0u64;
    for i in 0..z.len() {
        let mut e = z.by_index(i).map_err(|e| SkillError::Zip(e.to_string()))?;
        if e.is_symlink() {
            return Err(invalid(format!(
                "symbolic links are not allowed: {}",
                entry_name(&e)
            )));
        }
        let Some(rel) = e.enclosed_name() else {
            return Err(invalid(format!(
                "archive entry {:?} leaves the folder",
                entry_name(&e)
            )));
        };
        let out = dst.join(rel);
        if e.is_dir() {
            std::fs::create_dir_all(&out)?;
            continue;
        }
        if let Some(p) = out.parent() {
            std::fs::create_dir_all(p)?;
        }
        let mut w = std::fs::File::create(&out)?;
        // Limit what is actually written, not what the header claims (zip bombs).
        let left = MAX_TOTAL_BYTES - total;
        let n = std::io::copy(&mut (&mut e).take(left + 1), &mut w)?;
        total += n;
        if total > MAX_TOTAL_BYTES {
            return Err(invalid(format!(
                "the archive unpacks to more than {} MiB",
                MAX_TOTAL_BYTES >> 20
            )));
        }
    }
    Ok(())
}
