//! Remotes: testing one, its URL, pushing.

use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteTest {
    pub read_ok: bool,
    pub read_detail: String,
    pub write_ok: bool,
    pub write_detail: String,
}

impl Repo {
    /// Tests a remote: read (`ls-remote`) and write (`push --dry-run`).
    pub fn test_remote(&self, url: &str, env: &[(&str, &str)]) -> Result<RemoteTest, VcsError> {
        let read = self
            .git
            .run_env(&self.root, ["ls-remote", "--heads", url], env)?;
        let write = if read.ok() {
            Some(self.git.run_env(
                &self.root,
                [
                    "push",
                    "--dry-run",
                    "--porcelain",
                    url,
                    &format!("{}:refs/heads/agenium-probe", self.main),
                ],
                env,
            )?)
        } else {
            None
        };
        Ok(RemoteTest {
            read_ok: read.ok(),
            read_detail: if read.ok() {
                format!("{} branches", read.stdout.lines().count())
            } else {
                read.stderr.trim().to_owned()
            },
            write_ok: write.as_ref().is_some_and(|w| w.ok()),
            write_detail: write
                .map(|w| {
                    if w.ok() {
                        "dry-run push accepted".into()
                    } else {
                        w.stderr.trim().to_owned()
                    }
                })
                .unwrap_or_default(),
        })
    }

    /// The URL of remote `name` (e.g. `origin`), or `None` when the repo has no such remote.
    pub fn remote_url(&self, name: &str) -> Option<String> {
        self.git
            .run(&self.root, ["remote", "get-url", name])
            .ok()
            .filter(|o| o.ok())
            .map(|o| o.stdout.trim().to_owned())
            .filter(|u| !u.is_empty())
    }

    /// Pushes a branch to a remote (used when a ticket closes and the project has a remote).
    pub fn push(&self, remote: &str, refspec: &str, env: &[(&str, &str)]) -> Result<(), VcsError> {
        self.git
            .check_env(&self.root, ["push", "--porcelain", remote, refspec], env)?;
        Ok(())
    }
}
