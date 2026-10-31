//! The agent's own git config, kept in the run's temp dir.

use std::path::{Path, PathBuf};

/// The agent's own git config in the run's temp dir, written with `body` when it is not there. The temp dir is the
/// sandboxed commands' to write: a link left at the name (a dangling one too) had Agenium, which runs outside the
/// sandbox, create or write the file wherever it pointed (audit H2, 2026-10-31). Only a plain file is kept; anything
/// else is removed and the file made new, never through a link.
pub(crate) fn agent_gitconfig(dir: &Path, body: &str) -> std::io::Result<PathBuf> {
    use std::io::Write;
    let gc = dir.join("agent.gitconfig");
    match std::fs::symlink_metadata(&gc) {
        Ok(m) if m.file_type().is_file() => return Ok(gc),
        Ok(m) if m.is_dir() => std::fs::remove_dir_all(&gc)?,
        Ok(_) => std::fs::remove_file(&gc).or_else(|_| std::fs::remove_dir(&gc))?,
        Err(_) => {}
    }
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&gc)?
        .write_all(body.as_bytes())?;
    Ok(gc)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Audit H2 (2026-10-31): the temp dir is the sandboxed commands' to write; a link they leave at the git config's
    /// name is removed, never written through, and a plain file is kept as it is.
    #[test]
    fn the_agent_git_config_is_never_written_through_a_link() {
        let t = tempfile::tempdir().unwrap();
        let kept = agent_gitconfig(t.path(), "a").unwrap();
        std::fs::write(&kept, "edited").unwrap();
        assert_eq!(agent_gitconfig(t.path(), "a").unwrap(), kept);
        assert_eq!(std::fs::read_to_string(&kept).unwrap(), "edited");
        #[cfg(unix)]
        {
            let outside = tempfile::tempdir().unwrap();
            let target = outside.path().join("autostart.sh");
            std::fs::remove_file(&kept).unwrap();
            std::os::unix::fs::symlink(&target, &kept).unwrap();
            let gc = agent_gitconfig(t.path(), "[user]").unwrap();
            assert!(
                !target.exists(),
                "the dangling link's target was not created"
            );
            assert!(std::fs::symlink_metadata(&gc)
                .unwrap()
                .file_type()
                .is_file());
            assert_eq!(std::fs::read_to_string(&gc).unwrap(), "[user]");
        }
    }
}
