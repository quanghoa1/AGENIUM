//! Agenium root: `AGENIUM_HOME` > root chosen in the setup wizard (bootstrap file) > `~/.agenium` (decision 12).

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::ConfigError;

pub const ENV_HOME: &str = "AGENIUM_HOME";

/// Where the root came from (shown in Settings/diagnostics).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum RootSource {
    Env,
    Bootstrap,
    Default,
}

/// Small file in the OS config directory (`%APPDATA%\agenium\bootstrap.toml`, `~/.config/agenium/...`,
/// `~/Library/Application Support/agenium/...`) that remembers a non-default root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bootstrap {
    pub root: PathBuf,
}

impl Bootstrap {
    pub fn default_path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("agenium").join("bootstrap.toml"))
    }

    pub fn load(path: &Path) -> Result<Option<Bootstrap>, ConfigError> {
        match fs::read_to_string(path) {
            Ok(s) => toml::from_str(s.strip_prefix('\u{feff}').unwrap_or(&s))
                .map(Some)
                .map_err(|e| ConfigError::Parse {
                    path: path.into(),
                    message: e.to_string(),
                }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(ConfigError::io(path, e)),
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), ConfigError> {
        let text = toml::to_string(self).map_err(|e| ConfigError::Parse {
            path: path.into(),
            message: e.to_string(),
        })?;
        crate::write_atomic(path, text.as_bytes())
    }
}

/// Inputs of root resolution, injectable for tests.
#[derive(Debug, Clone, Default)]
pub struct RootInputs {
    pub env: Option<OsString>,
    pub bootstrap_file: Option<PathBuf>,
    pub home: Option<PathBuf>,
}

impl RootInputs {
    pub fn from_system() -> Self {
        Self {
            env: std::env::var_os(ENV_HOME).filter(|v| !v.is_empty()),
            bootstrap_file: Bootstrap::default_path(),
            home: dirs::home_dir(),
        }
    }
}

pub fn resolve_root_with(inputs: &RootInputs) -> Result<(PathBuf, RootSource), ConfigError> {
    if let Some(p) = &inputs.env {
        return Ok((PathBuf::from(p), RootSource::Env));
    }
    if let Some(file) = &inputs.bootstrap_file {
        if let Some(b) = Bootstrap::load(file)? {
            return Ok((b.root, RootSource::Bootstrap));
        }
    }
    let home = inputs.home.as_ref().ok_or(ConfigError::NoHome)?;
    Ok((home.join(".agenium"), RootSource::Default))
}

/// Resolves the root for this process.
pub fn resolve_root() -> Result<(PathBuf, RootSource), ConfigError> {
    resolve_root_with(&RootInputs::from_system())
}

/// Paths inside the root. Secrets are never stored here (only the encrypted vault, whose key lives in the OS
/// keyring).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootLayout {
    pub root: PathBuf,
}

impl RootLayout {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
    pub fn config(&self) -> PathBuf {
        self.root.join("config.toml")
    }
    pub fn pricing(&self) -> PathBuf {
        self.root.join("pricing.toml")
    }
    /// The vendors' subscription usage as last read (a cache: rewritten by every reading).
    /// Judge health (ADR-81): off after an auth/quota failure until the user re-enables.
    pub fn judge_health(&self) -> PathBuf {
        self.root.join("judge-health.json")
    }
    /// Each time the judge was asked (JSON lines, the newest last).
    pub fn judge_activity(&self) -> PathBuf {
        self.root.join("judge-activity.jsonl")
    }
    pub fn vendor_usage(&self) -> PathBuf {
        self.root.join("vendor-usage.json")
    }
    /// Prices downloaded from the public model catalogue (a cache: refreshed, never edited).
    pub fn price_catalogue(&self) -> PathBuf {
        self.root.join("price-catalogue.json")
    }
    /// The agent CLIs found and the models each source lists, with when (a cache: listed again, never edited).
    pub fn model_cache(&self) -> PathBuf {
        self.root.join("model-cache.json")
    }
    pub fn db(&self) -> PathBuf {
        self.root.join("db")
    }
    pub fn core_db(&self) -> PathBuf {
        self.db().join("core.db")
    }
    pub fn agent_dbs(&self) -> PathBuf {
        self.db().join("agents")
    }
    pub fn agent_db(&self, agent: &agenium_core::AgentId) -> PathBuf {
        self.agent_dbs().join(format!("{agent}.db"))
    }
    pub fn models(&self) -> PathBuf {
        self.root.join("models")
    }
    pub fn logs(&self) -> PathBuf {
        self.root.join("logs")
    }
    pub fn default_projects(&self) -> PathBuf {
        self.root.join("projects")
    }
    pub fn agents(&self) -> PathBuf {
        self.root.join("agents")
    }
    pub fn agent_dir(&self, agent: &agenium_core::AgentId) -> PathBuf {
        self.agents().join(agent.to_string())
    }
    /// Working folders of MCP stdio servers (`mcp/<agent>/<server>`).
    pub fn mcp_work(&self) -> PathBuf {
        self.root.join("mcp")
    }
    pub fn roles(&self) -> PathBuf {
        self.root.join("roles")
    }
    pub fn templates(&self) -> PathBuf {
        self.root.join("templates")
    }
    pub fn backups(&self) -> PathBuf {
        self.root.join("backups")
    }
    pub fn vault(&self) -> PathBuf {
        self.root.join("vault.bin")
    }

    /// Creates every directory of the layout (idempotent).
    pub fn ensure(&self) -> Result<(), ConfigError> {
        for d in [
            self.root.clone(),
            self.db(),
            self.agent_dbs(),
            self.models(),
            self.logs(),
            self.default_projects(),
            self.agents(),
            self.roles(),
            self.templates(),
            self.backups(),
        ] {
            fs::create_dir_all(&d).map_err(|e| ConfigError::io(&d, e))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn priority_env_then_bootstrap_then_default() {
        let tmp = tempfile::tempdir().unwrap();
        let boot = tmp.path().join("cfg").join("bootstrap.toml");
        let home = tmp.path().join("home");
        let mut inputs = RootInputs {
            env: Some("E:/x".into()),
            bootstrap_file: Some(boot.clone()),
            home: Some(home.clone()),
        };

        Bootstrap {
            root: tmp.path().join("chosen"),
        }
        .save(&boot)
        .unwrap();
        assert_eq!(
            resolve_root_with(&inputs).unwrap(),
            (PathBuf::from("E:/x"), RootSource::Env)
        );
        inputs.env = None;
        assert_eq!(
            resolve_root_with(&inputs).unwrap(),
            (tmp.path().join("chosen"), RootSource::Bootstrap)
        );
        fs::remove_file(&boot).unwrap();
        assert_eq!(
            resolve_root_with(&inputs).unwrap(),
            (home.join(".agenium"), RootSource::Default)
        );
        inputs.home = None;
        assert!(matches!(
            resolve_root_with(&inputs),
            Err(ConfigError::NoHome)
        ));
    }

    #[test]
    fn layout_creates_all_directories() {
        let tmp = tempfile::tempdir().unwrap();
        let l = RootLayout::new(tmp.path().join("root"));
        l.ensure().unwrap();
        l.ensure().unwrap();
        for d in [
            l.db(),
            l.agent_dbs(),
            l.models(),
            l.logs(),
            l.default_projects(),
            l.agents(),
            l.roles(),
            l.templates(),
            l.backups(),
        ] {
            assert!(d.is_dir(), "{}", d.display());
        }
    }
}
