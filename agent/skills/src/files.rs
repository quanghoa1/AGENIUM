//! A skill written with its files in one step: SKILL.md (tier 2) and its `references/`, `scripts/` and `assets/` (tier
//! 3). The app's editor adds, changes and removes them together; a SKILL.md that names `references/api.md` is valid only
//! once that file is there, so they are validated together, in a staging copy, before the real folder is touched.

use std::path::{Component, Path, PathBuf};

use super::*;

/// Folders a skill's own files live in.
const FILE_DIRS: [&str; 3] = ["references", "scripts", "assets"];

/// A skill's file path, relative to its folder: under `references/`, `scripts/` or `assets/`, with plain segments
/// (letters, digits, `.`, `_`, `-`; no `..`, at most four deep).
pub fn file_path(rel: &str) -> Result<PathBuf> {
    let rel = rel.trim().replace('\\', "/");
    let segments: Vec<&str> = rel.split('/').collect();
    let plain = |s: &&str| {
        !s.is_empty()
            && *s != "."
            && *s != ".."
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
    };
    let ok = segments.len() >= 2
        && segments.len() <= 4
        && FILE_DIRS.contains(&segments[0])
        && segments.iter().all(plain);
    if !ok {
        return Err(invalid(format!(
            "a skill's file goes under references/, scripts/ or assets/ with plain names (a-z, 0-9, . _ -); got {rel:?}"
        )));
    }
    let p = PathBuf::from(rel);
    debug_assert!(p.components().all(|c| matches!(c, Component::Normal(_))));
    Ok(p)
}

impl Registry {
    /// Writes a skill's SKILL.md and, with it, the files `files` (path, bytes; added or changed) and removes the files
    /// `removed`. The frontmatter `name` must equal `name`; `create` requires the folder not to exist yet. The folder as
    /// it would be after the write is validated in a staging copy first (its references found, its size within the
    /// limits); only then are the real files written (each to a temporary file renamed over it), SKILL.md last.
    pub fn write_with_files(
        &self,
        owner: &Owner,
        name: &str,
        content: &str,
        create: bool,
        files: &[(String, Vec<u8>)],
        removed: &[String],
    ) -> Result<SkillMeta> {
        if !valid_name(name) {
            return Err(invalid(format!("invalid skill name {name:?}")));
        }
        // The content must name this very skill, so the folder name and the frontmatter agree.
        let (fm, _) = parse(content)?;
        if fm.get("name") != Some(name) {
            return Err(invalid(format!(
                "the frontmatter name must be {name:?} (it is {:?})",
                fm.get("name").unwrap_or("")
            )));
        }
        let files: Vec<(PathBuf, &[u8])> = files
            .iter()
            .map(|(p, b)| Ok((file_path(p)?, b.as_slice())))
            .collect::<Result<_>>()?;
        let removed: Vec<PathBuf> = removed
            .iter()
            .map(|p| file_path(p))
            .collect::<Result<_>>()?;
        let origin = match owner {
            Owner::Agent(_) => Origin::Agent,
            Owner::Role(_) => Origin::Role,
            Owner::Library => Origin::Library,
        };
        let dir = self.dir_of(owner).join(name);
        let exists = dir.join(SKILL_FILE).exists();
        if create && exists {
            return Err(SkillError::Exists(name.to_owned()));
        }
        if !create && !exists {
            return Err(SkillError::NotFound(name.into()));
        }
        std::fs::create_dir_all(&dir)?;
        let staging = self
            .dir_of(owner)
            .join(format!(".edit-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&staging);
        // copy_dir only copies files; a new skill's folder is empty, so make the staging dir first.
        std::fs::create_dir_all(&staging)?;
        let result = (|| {
            copy_dir(&dir, &staging)?;
            apply(&staging, &files, &removed)?;
            std::fs::write(staging.join(SKILL_FILE), content)?;
            validate_named(&staging, origin, false)?;
            // Valid as a whole: the real folder gets the same changes, SKILL.md last.
            apply(&dir, &files, &removed)?;
            replace(&dir.join(SKILL_FILE), content.as_bytes())?;
            Ok(validate(&dir, origin)?.meta)
        })();
        let _ = std::fs::remove_dir_all(&staging);
        if result.is_err() && create && !exists {
            // A failed create leaves no folder behind.
            let _ = std::fs::remove_dir_all(&dir);
        }
        result
    }
}

/// Writes `files` into `dir` and removes `removed` from it (a file already gone is fine; a folder left empty goes).
fn apply(dir: &Path, files: &[(PathBuf, &[u8])], removed: &[PathBuf]) -> Result<()> {
    for (rel, bytes) in files {
        let p = dir.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent)?;
        }
        replace(&p, bytes)?;
    }
    for rel in removed {
        match std::fs::remove_file(dir.join(rel)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
            _ => {}
        }
        if let Some(parent) = dir.join(rel).parent() {
            // Only when empty: a folder with other files stays.
            let _ = std::fs::remove_dir(parent);
        }
    }
    Ok(())
}

/// Replaces `p` with `bytes` through a temporary file renamed over it: a crash cannot leave it cut short.
fn replace(p: &Path, bytes: &[u8]) -> Result<()> {
    let mut name = p.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".tmp-{}", std::process::id()));
    let tmp = p.with_file_name(name);
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, p)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skill(name: &str, body: &str) -> String {
        format!("---\nname: {name}\ndescription: Does one thing.\n---\n\n{body}\n")
    }

    /// A new skill and the reference its SKILL.md names are written together (alone, the SKILL.md was refused for its
    /// missing reference); files are added, changed and removed in one step; a path out of the three folders, or a
    /// SKILL.md naming a file that is not there, changes nothing.
    #[test]
    fn a_skill_is_written_with_its_references_and_scripts() {
        let t = tempfile::tempdir().unwrap();
        let reg = Registry::new(t.path().join("agents"), t.path().join("roles"));
        let body = "## Steps\nRead `references/api.md`, then run `scripts/check.sh`.";
        let files = vec![
            ("references/api.md".to_owned(), b"# API".to_vec()),
            ("scripts/check.sh".to_owned(), b"echo ok".to_vec()),
        ];
        assert!(reg
            .write(&Owner::Library, "demo", &skill("demo", body), true)
            .is_err());
        reg.write_with_files(
            &Owner::Library,
            "demo",
            &skill("demo", body),
            true,
            &files,
            &[],
        )
        .unwrap();
        let dir = reg.dir_of(&Owner::Library).join("demo");
        assert_eq!(
            std::fs::read_to_string(dir.join("scripts/check.sh")).unwrap(),
            "echo ok"
        );
        // Changed, added and removed together; the SKILL.md no longer names the script.
        let body2 = "## Steps\nRead `references/api.md` and `references/more.md`.";
        reg.write_with_files(
            &Owner::Library,
            "demo",
            &skill("demo", body2),
            false,
            &[
                ("references/api.md".to_owned(), b"# API v2".to_vec()),
                ("references/more.md".to_owned(), b"more".to_vec()),
            ],
            &["scripts/check.sh".to_owned()],
        )
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("references/api.md")).unwrap(),
            "# API v2"
        );
        assert!(dir.join("references/more.md").exists());
        assert!(!dir.join("scripts").exists(), "an emptied folder goes");
        // Refused: a path out of the folders; removing a file the SKILL.md still names. Nothing changes.
        for bad in [
            "../x.md",
            "notes/x.md",
            "references/../../x",
            "references/a b.md",
        ] {
            assert!(file_path(bad).is_err(), "{bad}");
        }
        let r = reg.write_with_files(
            &Owner::Library,
            "demo",
            &skill("demo", body2),
            false,
            &[],
            &["references/more.md".to_owned()],
        );
        assert!(r.is_err());
        assert!(dir.join("references/more.md").exists());
        assert!(!reg.dir_of(&Owner::Library).read_dir().unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".edit-")));
    }
}
