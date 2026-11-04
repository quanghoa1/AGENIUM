//! Where a plugin comes from (ADR-157): a GitHub repository at a ref, read as GitHub's zip of it (the commit is the
//! archive's comment), or a folder on this machine.

use std::path::{Path, PathBuf};

use crate::{OrchError, Result};

/// The largest repository archive read (the unpacked size is capped again by the skill limits).
const MAX_ZIP: u64 = 50 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// `owner/repo` at `reference` (a branch, a tag or a commit; `HEAD` when none is named).
    GitHub {
        owner: String,
        repo: String,
        reference: String,
    },
    /// A folder (an unpacked plugin, or a clone).
    Folder(PathBuf),
}

fn invalid(m: impl Into<String>) -> OrchError {
    OrchError::Invalid(m.into())
}

fn github_part(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 100
        && s != "."
        && s != ".."
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
}

impl Source {
    /// `owner/repo`, `owner/repo@ref`, `https://github.com/owner/repo[.git][@ref]`, or an absolute folder path.
    pub fn parse(text: &str) -> Result<Source> {
        let t = text.trim();
        let p = Path::new(t);
        if p.is_absolute() {
            if !p.is_dir() {
                return Err(invalid(format!("{t} is not a folder")));
            }
            return Ok(Source::Folder(p.to_path_buf()));
        }
        let t = t
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .trim_start_matches("github.com/");
        let (path, reference) = match t.split_once('@') {
            Some((p, r)) => (p, r.trim()),
            None => (t, "HEAD"),
        };
        let path = path.trim_end_matches('/').trim_end_matches(".git");
        let Some((owner, repo)) = path.split_once('/') else {
            return Err(invalid(format!(
                "{text:?}: name a GitHub repository as owner/repo"
            )));
        };
        let reference_ok = !reference.is_empty()
            && reference.len() <= 200
            && !reference.contains("..")
            && reference
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_./".contains(&b));
        if !github_part(owner) || !github_part(repo) || !reference_ok {
            return Err(invalid(format!("{text:?} is not owner/repo[@ref]")));
        }
        Ok(Source::GitHub {
            owner: owner.into(),
            repo: repo.into(),
            reference: reference.into(),
        })
    }

    /// The text that names it again (kept with the plugin, read for its updates).
    pub fn text(&self) -> String {
        match self {
            Source::GitHub { owner, repo, .. } => format!("{owner}/{repo}"),
            Source::Folder(p) => p.display().to_string(),
        }
    }

    /// The repository's name, the plugin's name when nothing else gives one.
    pub fn fallback_name(&self) -> String {
        match self {
            Source::GitHub { repo, .. } => repo.to_ascii_lowercase(),
            Source::Folder(p) => p
                .file_name()
                .map(|n| n.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default(),
        }
    }

    /// The same source at another ref (an install takes the commit its preview showed).
    pub fn at(&self, reference: &str) -> Source {
        match self {
            Source::GitHub { owner, repo, .. } => Source::GitHub {
                owner: owner.clone(),
                repo: repo.clone(),
                reference: reference.into(),
            },
            f => f.clone(),
        }
    }

    pub fn reference(&self) -> &str {
        match self {
            Source::GitHub { reference, .. } => reference,
            Source::Folder(_) => "local",
        }
    }
}

/// The source's files in `staging` (created empty by the caller): the repository's root folder and its commit
/// (`local` for a folder).
pub async fn fetch(source: &Source, staging: &Path) -> Result<(PathBuf, String)> {
    match source {
        Source::Folder(p) => Ok((p.clone(), "local".into())),
        Source::GitHub {
            owner,
            repo,
            reference,
        } => {
            // codeload is GitHub's archive host: no API call, so no rate limit for an anonymous user.
            let url = format!("https://codeload.github.com/{owner}/{repo}/zip/{reference}");
            let http = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .map_err(|e| invalid(e.to_string()))?;
            let r = http
                .get(&url)
                .header(reqwest::header::USER_AGENT, "agenium")
                .send()
                .await
                .and_then(|r| r.error_for_status())
                .map_err(|e| invalid(format!("cannot download {owner}/{repo}@{reference}: {e}")))?;
            if r.content_length().is_some_and(|n| n > MAX_ZIP) {
                return Err(invalid(format!(
                    "{owner}/{repo} is larger than {} MiB",
                    MAX_ZIP >> 20
                )));
            }
            let bytes = r
                .bytes()
                .await
                .map_err(|e| invalid(format!("download of {owner}/{repo}: {e}")))?;
            if bytes.len() as u64 > MAX_ZIP {
                return Err(invalid(format!(
                    "{owner}/{repo} is larger than {} MiB",
                    MAX_ZIP >> 20
                )));
            }
            let zip_path = staging.join("repo.zip");
            std::fs::write(&zip_path, &bytes)?;
            unpack(&zip_path, &staging.join("repo"))
        }
    }
}

/// A GitHub archive unpacked: its one top folder, and the commit `git archive` wrote as the archive's comment.
pub fn unpack(zip_path: &Path, dst: &Path) -> Result<(PathBuf, String)> {
    let commit = {
        let z = zip::ZipArchive::new(std::fs::File::open(zip_path)?)
            .map_err(|e| invalid(format!("not a zip archive: {e}")))?;
        String::from_utf8_lossy(z.comment()).trim().to_owned()
    };
    if commit.len() != 40 || !commit.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(invalid("the archive does not name its commit"));
    }
    std::fs::create_dir_all(dst)?;
    agenium_skills::extract_zip(zip_path, dst).map_err(|e| invalid(e.to_string()))?;
    let tops: Vec<PathBuf> = std::fs::read_dir(dst)?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.path())
        .collect();
    match tops.as_slice() {
        [one] => Ok((one.clone(), commit)),
        _ => Err(invalid("the archive does not hold one top folder")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_repository_is_named_as_github_shows_it() {
        let gh = |o: &str, r: &str, f: &str| Source::GitHub {
            owner: o.into(),
            repo: r.into(),
            reference: f.into(),
        };
        assert_eq!(
            Source::parse("obra/superpowers").unwrap(),
            gh("obra", "superpowers", "HEAD")
        );
        assert_eq!(
            Source::parse("https://github.com/obra/superpowers.git@v7.0.2").unwrap(),
            gh("obra", "superpowers", "v7.0.2")
        );
        assert_eq!(
            Source::parse("github.com/a/b/@feature/x").unwrap(),
            gh("a", "b", "feature/x")
        );
        for bad in ["superpowers", "a/b@../x", "../b", "a/b c", "a/b@"] {
            assert!(Source::parse(bad).is_err(), "{bad}");
        }
        assert_eq!(gh("o", "R", "HEAD").fallback_name(), "r");
    }

    #[test]
    fn an_archive_names_its_commit() {
        let tmp = tempfile::tempdir().unwrap();
        let zip_path = tmp.path().join("a.zip");
        let sha = "22f0b463e7a5e579904dd6211a520321035526f7";
        {
            let mut w = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
            let o = zip::write::SimpleFileOptions::default();
            w.start_file("repo-HEAD/README.md", o).unwrap();
            std::io::Write::write_all(&mut w, b"hi").unwrap();
            w.set_comment(sha).unwrap();
            w.finish().unwrap();
        }
        let (root, commit) = unpack(&zip_path, &tmp.path().join("out")).unwrap();
        assert_eq!(commit, sha);
        assert!(root.ends_with("repo-HEAD") && root.join("README.md").is_file());
    }
}
