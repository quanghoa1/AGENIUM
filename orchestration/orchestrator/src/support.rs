//! Support bundle (M5): a folder of diagnostics the user can share — versions, the configuration (secrets are stored
//! by reference, so no values), store counts, and the tail of recent log files with secret values masked. Nothing in
//! it is a secret; it is written under a folder the user chooses.

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::app::App;
use crate::{OrchError, Result};

/// Bytes of each log file kept (its tail).
const LOG_TAIL: usize = 200_000;

fn io(e: std::io::Error) -> OrchError {
    OrchError::Invalid(e.to_string())
}

impl App {
    /// Writes a support bundle under `out` and returns its folder. Secret values are masked; configuration holds
    /// only secret names.
    pub fn support_bundle(&self, out: &Path) -> Result<PathBuf> {
        let dir = out.join(format!(
            "agenium-support-{}",
            chrono::Utc::now().format("%Y%m%dT%H%M%SZ")
        ));
        std::fs::create_dir_all(&dir).map_err(io)?;
        let write = |name: &str, body: &str| -> Result<()> {
            std::fs::write(dir.join(name), body).map_err(io)
        };

        // Versions and environment.
        let clis = agenium_cli_backend::detect::detect_all();
        let mut v = format!(
            "agenium {}\nos: {}\narch: {}\ncore.db schema (migrations): {}\nagent.db schema (migrations): {}\nconfig schema_version: {}\n\nCLIs:\n",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::env::consts::ARCH,
            agenium_store::core::CORE_MIGRATIONS.len(),
            agenium_store::agent::AGENT_MIGRATIONS.len(),
            self.config().schema_version,
        );
        for c in &clis {
            v.push_str(&format!(
                "  {:<9} {:<10} {:?}{}\n",
                c.kind.program(),
                c.version.clone().unwrap_or_else(|| "-".into()),
                c.login,
                c.note
                    .as_deref()
                    .map(|n| format!(" ({n})"))
                    .unwrap_or_default()
            ));
        }
        let sandbox = agenium_sandbox::detect();
        v.push_str(&format!(
            "\nsandbox backend: {}\n",
            sandbox
                .backend
                .map(|b| format!("{b:?}"))
                .unwrap_or_else(|| format!("none: {}", sandbox.reason))
        ));
        v.push_str("\nproviders:\n");
        for p in self.config().providers {
            v.push_str(&format!(
                "  {:<14} {:?}{}\n",
                p.id,
                p.kind,
                p.base_url.map(|u| format!(" {u}")).unwrap_or_default()
            ));
        }
        write("versions.txt", &v)?;

        // Configuration (secrets are references, not values).
        write(
            "config.toml",
            &toml::to_string_pretty(&self.config()).unwrap_or_default(),
        )?;

        // Store counts.
        let mut stats = String::new();
        let projects = self.projects()?;
        stats.push_str(&format!(
            "projects: {}\nagents: {}\n",
            projects.len(),
            self.agents()?.len()
        ));
        for p in &projects {
            let tickets = self.tickets(p.id)?;
            let mut by = std::collections::BTreeMap::<String, usize>::new();
            for t in &tickets {
                *by.entry(format!("{:?}", t.status)).or_default() += 1;
            }
            let runs = self
                .store()
                .read(|c| agenium_store::core::runs::recent(c, usize::MAX >> 1))?
                .len();
            stats.push_str(&format!(
                "\n[{}] {} — {} ticket(s): {}; {} run(s) total (store-wide)\n  open: {}\n",
                p.key_prefix,
                p.name,
                tickets.len(),
                by.iter()
                    .map(|(k, n)| format!("{k} {n}"))
                    .collect::<Vec<_>>()
                    .join(", "),
                runs,
                tickets.iter().filter(|t| !t.status.is_terminal()).count(),
            ));
        }
        write("stats.txt", &stats)?;

        // Recent log files, secret values masked.
        let logs_dir = dir.join("logs");
        std::fs::create_dir_all(&logs_dir).map_err(io)?;
        if let Ok(rd) = std::fs::read_dir(self.layout().logs()) {
            let mut files: Vec<PathBuf> = rd
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == "log"))
                .collect();
            files.sort();
            for f in files.iter().rev().take(5) {
                if let Ok(text) = std::fs::read_to_string(f) {
                    let tail: String = text
                        .chars()
                        .rev()
                        .take(LOG_TAIL)
                        .collect::<String>()
                        .chars()
                        .rev()
                        .collect();
                    let masked = self.redactor().redact(&tail);
                    let name = f
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned();
                    let mut w = std::fs::File::create(logs_dir.join(name)).map_err(io)?;
                    w.write_all(masked.as_bytes()).map_err(io)?;
                }
            }
        }
        write(
            "README.txt",
            "Agenium support bundle. Diagnostics only: versions, configuration (secret names, not values), store \
counts, and masked log tails. Review before sharing.\n",
        )?;
        Ok(dir)
    }
}
