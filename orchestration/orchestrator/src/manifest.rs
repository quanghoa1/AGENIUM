//! `agenium.toml`: the project manifest at the repository root (decision 7). It identifies the project and
//! carries per-project settings that belong with the code (worktree preparation, pre-merge check, network
//! allowlist defaults). Coordination data stays in `core.db`.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::{OrchError, Result};

pub const FILE: &str = "agenium.toml";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectManifest {
    pub project: ProjectSection,
    #[serde(default)]
    pub worktree: WorktreeSection,
    #[serde(default)]
    pub checks: ChecksSection,
    #[serde(default)]
    pub network: NetworkSection,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectSection {
    pub id: String,
    pub name: String,
    #[serde(default = "default_prefix")]
    pub key_prefix: String,
    #[serde(default = "default_main")]
    pub main_branch: String,
    /// Team template (`coding@1`, `pentest@1`, `research@1`, `content@1`).
    #[serde(default = "default_template")]
    pub template: String,
}

fn default_prefix() -> String {
    "AGN".into()
}
fn default_main() -> String {
    "main".into()
}
fn default_template() -> String {
    "coding@1".into()
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WorktreeSection {
    /// Untracked files copied into every ticket worktree (e.g. `.env.example`).
    #[serde(default)]
    pub include_files: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ChecksSection {
    /// Command run in the ticket worktree by the merge queue (e.g. `cargo test`). Empty = no check.
    #[serde(default)]
    pub pre_merge: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct NetworkSection {
    /// Default network allowlist of the project (the Lead may approve requests inside it, per run).
    #[serde(default)]
    pub allowlist: Vec<String>,
}

impl ProjectManifest {
    pub fn load(dir: &Path) -> Result<Option<ProjectManifest>> {
        let p = dir.join(FILE);
        match std::fs::read_to_string(&p) {
            Ok(s) => toml::from_str(&s)
                .map(Some)
                .map_err(|e| OrchError::Invalid(format!("{}: {e}", p.display()))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(OrchError::Invalid(format!("{}: {e}", p.display()))),
        }
    }

    pub fn save(&self, dir: &Path) -> Result<()> {
        let text = toml::to_string_pretty(self).map_err(|e| OrchError::Invalid(e.to_string()))?;
        agenium_config::write_atomic(&dir.join(FILE), text.as_bytes())?;
        Ok(())
    }
}
