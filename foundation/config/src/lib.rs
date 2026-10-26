//! Agenium root, `config.toml`, setup wizard backend and system requirements (decisions 12 and 24).

pub mod config;
pub mod root;
pub mod setup;
pub mod sysreq;

use std::io::Write;
use std::path::{Path, PathBuf};

pub use config::{
    valid_mcp_id, ApprovalPolicy, CliSection, Config, JudgeSection, McpOAuthConfig,
    McpServerConfig, ModelCaps, ProviderConfig, ProviderKind, SecretsBackend,
};
pub use root::{resolve_root, resolve_root_with, Bootstrap, RootInputs, RootLayout, RootSource};

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("no home directory found; set AGENIUM_HOME")]
    NoHome,
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{path}: {message}")]
    Parse { path: PathBuf, message: String },
    #[error("invalid configuration: {0}")]
    Invalid(String),
}

impl ConfigError {
    pub(crate) fn io(path: &Path, source: std::io::Error) -> Self {
        ConfigError::Io {
            path: path.to_path_buf(),
            source,
        }
    }
}

/// Writes `bytes` to `path` via a temporary sibling file + rename, creating parent directories.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), ConfigError> {
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir).map_err(|e| ConfigError::io(dir, e))?;
    let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
    let mut f = std::fs::File::create(&tmp).map_err(|e| ConfigError::io(&tmp, e))?;
    f.write_all(bytes)
        .and_then(|_| f.sync_all())
        .map_err(|e| ConfigError::io(&tmp, e))?;
    drop(f);
    std::fs::rename(&tmp, path).map_err(|e| ConfigError::io(path, e))
}
