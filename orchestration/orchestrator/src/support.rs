//! Support bundle (M5): a folder of diagnostics the user can share — versions, the configuration (vault secrets are
//! references; plain MCP `env`/`headers` values that look like secrets are masked), store counts, and the tail of
//! recent log files with secret values masked. It is written under a folder the user chooses.

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::app::App;
use crate::Result;

/// Bytes of each log file kept (its tail).
const LOG_TAIL: usize = 200_000;

/// The configuration as TOML with plain secret-looking MCP values replaced and known secrets redacted.
fn masked_config(app: &App) -> String {
    let mut cfg = app.config();
    for server in &mut cfg.mcp {
        for key in crate::mcp::plaintext_secrets(server) {
            for map in [&mut server.env, &mut server.headers] {
                if let Some(v) = map.get_mut(&key) {
                    *v = "[masked]".into();
                }
            }
        }
    }
    app.redactor()
        .redact(&toml::to_string_pretty(&cfg).unwrap_or_default())
}

impl App {
    /// Writes a support bundle under `out` and returns its folder. Secret values are masked.
    pub fn support_bundle(&self, out: &Path) -> Result<PathBuf> {
        let dir = out.join(format!(
            "agenium-support-{}",
            chrono::Utc::now().format("%Y%m%dT%H%M%SZ")
        ));
        std::fs::create_dir_all(&dir)?;
        let write =
            |name: &str, body: &str| -> Result<()> { Ok(std::fs::write(dir.join(name), body)?) };

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

        // Configuration: vault secrets are references, but an MCP server's env/headers may hold a plain key.
        write("config.toml", &masked_config(self))?;

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
            let runs = self.store().read(agenium_store::core::runs::count)?;
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
        std::fs::create_dir_all(&logs_dir)?;
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
                    let mut w = std::fs::File::create(logs_dir.join(name))?;
                    w.write_all(masked.as_bytes())?;
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
