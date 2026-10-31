//! Agenium's own markitdown (ADR-152): `<root>/tools/markitdown`, one virtual environment per install under `envs/`,
//! the one in use named in `state.json`. uv, a Python it fetched and the download cache stay in the same folder, so
//! removing the folder removes everything. The root names the folder when it opens; the tools then prefer this copy to
//! one on the PATH.

use std::path::{Path, PathBuf};
use std::sync::RwLock;

use serde::{Deserialize, Serialize};

/// The folder of the open root's copy.
static DIR: RwLock<Option<PathBuf>> = RwLock::new(None);

/// Names the folder of Agenium's own copy (the root's `tools/markitdown`).
pub fn use_dir(dir: PathBuf) {
    *DIR.write().unwrap_or_else(|p| p.into_inner()) = Some(dir);
}

/// The folder of Agenium's own copy, once a root is open.
pub fn dir() -> Option<PathBuf> {
    DIR.read().unwrap_or_else(|p| p.into_inner()).clone()
}

/// What is installed and how it was checked, kept in `state.json`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct State {
    /// The environment in use (a folder of `envs/`), empty when none.
    #[serde(default)]
    pub env: String,
    #[serde(default)]
    pub version: String,
    /// How it was installed: `uv` (on the PATH), `python` (a venv of the Python found) or `uv-standalone`.
    #[serde(default)]
    pub method: String,
    #[serde(default)]
    pub installed_at: String,
    /// The last self-test: each sample format and whether it read well (`ok` or why not).
    #[serde(default)]
    pub tested_at: String,
    #[serde(default)]
    pub tests: Vec<(String, String)>,
    /// The newest version on PyPI at the last weekly check.
    #[serde(default)]
    pub latest: String,
    #[serde(default)]
    pub checked_at: String,
}

pub fn state_file(dir: &Path) -> PathBuf {
    dir.join("state.json")
}

pub fn read_state(dir: &Path) -> State {
    std::fs::read_to_string(state_file(dir))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Writes the state through a temporary file (a crash keeps the old one).
pub fn write_state(dir: &Path, s: &State) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join("state.json.tmp");
    std::fs::write(
        &tmp,
        serde_json::to_vec_pretty(s).map_err(std::io::Error::other)?,
    )?;
    std::fs::rename(tmp, state_file(dir))
}

pub fn envs(dir: &Path) -> PathBuf {
    dir.join("envs")
}

/// The markitdown program of a virtual environment.
pub fn program_in(env: &Path) -> PathBuf {
    if cfg!(windows) {
        env.join("Scripts").join("markitdown.exe")
    } else {
        env.join("bin").join("markitdown")
    }
}

/// The program in use, when its environment is in place.
pub fn current(dir: &Path) -> Option<(State, PathBuf)> {
    let s = read_state(dir);
    if s.env.is_empty() {
        return None;
    }
    let program = program_in(&envs(dir).join(&s.env));
    program.is_file().then_some((s, program))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_copy_in_use_is_the_one_the_state_names_and_present() {
        let t = tempfile::tempdir().unwrap();
        assert!(current(t.path()).is_none());
        let s = State {
            env: "0.1.9-1".into(),
            version: "0.1.9".into(),
            ..State::default()
        };
        write_state(t.path(), &s).unwrap();
        assert!(current(t.path()).is_none(), "named but not installed");
        let p = program_in(&envs(t.path()).join("0.1.9-1"));
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, "").unwrap();
        let (got, program) = current(t.path()).unwrap();
        assert_eq!(got, s);
        assert_eq!(program, p);
    }
}
