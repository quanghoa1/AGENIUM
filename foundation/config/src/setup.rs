//! Setup wizard backend (M0-3): first run creates the root, writes `config.toml` and, when the user picked a
//! non-default root, the bootstrap file.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::root::{Bootstrap, RootLayout};
use crate::sysreq::{self, SysReport};
use crate::ConfigError;

/// `true` when the root has no `config.toml` yet.
pub fn needs_setup(root: &Path) -> bool {
    !RootLayout::new(root).config().is_file()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetupInput {
    /// `None` = default root (`~/.agenium`), no bootstrap file is written.
    pub root: Option<PathBuf>,
    /// UI language: "en" | "vi".
    pub ui_language: String,
    /// Language used with the Lead/Solo (ADR-40).
    pub user_language: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SetupOutcome {
    pub root: PathBuf,
    pub config: PathBuf,
    pub report: SysReport,
}

/// Runs the wizard's final step. Idempotent: an existing `config.toml` is kept (only languages are updated).
pub fn complete_setup(
    input: &SetupInput,
    default_root: &Path,
    bootstrap_file: Option<&Path>,
) -> Result<SetupOutcome, ConfigError> {
    if input.user_language.trim().is_empty() {
        return Err(ConfigError::Invalid(
            "user_language must not be empty".into(),
        ));
    }
    let root = input
        .root
        .clone()
        .unwrap_or_else(|| default_root.to_path_buf());
    let layout = RootLayout::new(&root);
    layout.ensure()?;
    let report = sysreq::check(&root);

    let mut cfg = if layout.config().is_file() {
        Config::load(&layout.config())?
    } else {
        Config::default()
    };
    cfg.app.language = input.ui_language.clone();
    cfg.app.user_language = input.user_language.trim().to_owned();
    cfg.runtime.max_parallel_agents = cfg
        .runtime
        .max_parallel_agents
        .min(report.suggested_parallel)
        .max(1);
    cfg.check().map_err(ConfigError::Invalid)?;
    cfg.save(&layout.config())?;

    if let (Some(chosen), Some(file)) = (&input.root, bootstrap_file) {
        if chosen != default_root {
            Bootstrap {
                root: chosen.clone(),
            }
            .save(file)?;
        }
    }
    Ok(SetupOutcome {
        root,
        config: layout.config(),
        report,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::root::{resolve_root_with, RootInputs, RootSource};

    #[test]
    fn first_run_creates_root_and_bootstrap() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let default_root = home.join(".agenium");
        let boot = tmp.path().join("cfg/agenium/bootstrap.toml");
        let chosen = tmp.path().join("data/agenium");
        assert!(needs_setup(&chosen));

        let input = SetupInput {
            root: Some(chosen.clone()),
            ui_language: "vi".into(),
            user_language: "vi".into(),
        };
        let out = complete_setup(&input, &default_root, Some(&boot)).unwrap();
        assert_eq!(out.root, chosen);
        assert!(!needs_setup(&chosen));
        let cfg = Config::load(&out.config).unwrap();
        assert_eq!(
            (cfg.app.language.as_str(), cfg.app.user_language.as_str()),
            ("vi", "vi")
        );
        assert!(cfg.runtime.max_parallel_agents <= out.report.suggested_parallel);

        let inputs = RootInputs {
            env: None,
            bootstrap_file: Some(boot),
            home: Some(home),
        };
        assert_eq!(
            resolve_root_with(&inputs).unwrap(),
            (chosen, RootSource::Bootstrap)
        );
    }

    #[test]
    fn default_root_writes_no_bootstrap_and_rejects_empty_language() {
        let tmp = tempfile::tempdir().unwrap();
        let default_root = tmp.path().join(".agenium");
        let boot = tmp.path().join("bootstrap.toml");
        let input = SetupInput {
            root: None,
            ui_language: "en".into(),
            user_language: " ".into(),
        };
        assert!(complete_setup(&input, &default_root, Some(&boot)).is_err());
        let input = SetupInput {
            user_language: "en".into(),
            ..input
        };
        complete_setup(&input, &default_root, Some(&boot)).unwrap();
        assert!(!boot.exists());
        assert!(default_root.join("config.toml").is_file());
    }
}
