//! The vendors' subscription usage for the app's footer (display only, decision 3k). Claude reports it during its
//! own runs or, when the user asks, from one tiny Haiku turn; Codex and OpenCode Go are read on a schedule through their honest channels
//! (`agenium_llm::vendor_usage`). Antigravity has no way to report quota to another app yet (agy 1.4.7 runs
//! `/usage` as a model turn), so it is listed without numbers. The last reading of each vendor is kept in
//! `<root>/vendor-usage.json`.

use std::collections::BTreeMap;

use agenium_cli_backend::{CliKind, Login};
use agenium_llm::vendor_usage::{fetch_opencode_go, is_opencode_go, VendorUsage};

use crate::app::App;
use crate::{OrchError, Result};

/// Polling interval of the readings that cost nothing (Codex RPC, OpenCode Go endpoint).
pub const REFRESH_MINUTES: u64 = 15;

impl App {
    fn read_vendor_usage(&self) -> BTreeMap<String, VendorUsage> {
        std::fs::read_to_string(self.layout().vendor_usage())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    fn write_vendor_usage(&self, all: &BTreeMap<String, VendorUsage>) -> Result<()> {
        let path = self.layout().vendor_usage();
        let text =
            serde_json::to_string_pretty(all).map_err(|e| OrchError::Invalid(e.to_string()))?;
        Ok(agenium_config::write_atomic(&path, text.as_bytes())?)
    }

    /// A vendor's last reading (`claude`, `codex`, `opencode-go:<provider id>`), if any.
    pub fn vendor_reading(&self, vendor: &str) -> Option<VendorUsage> {
        self.read_vendor_usage().remove(vendor)
    }

    /// Keeps a vendor's reading. A failed reading keeps the earlier windows (shown as stale) with the error.
    pub fn record_vendor_usage(&self, u: VendorUsage) -> Result<()> {
        let _guard = self
            .vendor_usage_lock
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let mut all = self.read_vendor_usage();
        let entry = match (u.error.is_some(), all.remove(&u.vendor)) {
            (true, Some(mut old)) => {
                old.error = u.error;
                old
            }
            _ => u,
        };
        all.insert(entry.vendor.clone(), entry);
        self.write_vendor_usage(&all)
    }

    /// One row per vendor in use: the CLIs installed on this machine and the OpenCode Go providers configured.
    pub fn vendor_usage(&self, clis: &[agenium_cli_backend::CliInfo]) -> Vec<VendorUsage> {
        let known = self.read_vendor_usage();
        let mut out = Vec::new();
        let mut clis: Vec<_> = clis.iter().collect();
        clis.sort_by_key(|c| {
            [CliKind::Claude, CliKind::Codex, CliKind::Agy]
                .iter()
                .position(|k| *k == c.kind)
        });
        let row = |id: &str| {
            known
                .get(id)
                .cloned()
                .unwrap_or_else(|| VendorUsage::new(id, ""))
        };
        for c in clis.into_iter().filter(|c| c.path.is_some()) {
            let id = match c.kind {
                CliKind::Claude => "claude",
                CliKind::Codex => "codex",
                CliKind::Agy => "agy",
                _ => continue,
            };
            let mut r = row(id);
            if c.login == Login::NotLoggedIn {
                r.error = Some("not signed in".into());
            } else if c.kind == CliKind::Claude && r.updated_at.is_empty() {
                r.note = Some("updates when a Claude agent runs, or read it now".into());
            } else if c.kind == CliKind::Agy {
                r.note = Some("Antigravity does not report its quota to other apps".into());
            }
            out.push(r);
        }
        for p in self.config().providers {
            if p.base_url.as_deref().is_some_and(is_opencode_go) {
                out.push(row(&format!("opencode-go:{}", p.id)));
            }
        }
        out
    }
}

/// Reads the vendors that can be asked at no cost (Codex, OpenCode Go) and records the result.
pub async fn refresh_vendor_usage(app: &App, clis: &[agenium_cli_backend::CliInfo]) -> Result<()> {
    let now = agenium_core::now_iso();
    if let Some(codex) = clis
        .iter()
        .find(|c| c.kind == CliKind::Codex && c.compatible && c.login != Login::NotLoggedIn)
    {
        if let Some(program) = &codex.path {
            let scratch = app.layout().agents().join(".usage-probe");
            let u = agenium_cli_backend::codex::rate_limits(program, &scratch, &now)
                .await
                .unwrap_or_else(|e| VendorUsage {
                    error: Some(e),
                    ..VendorUsage::new("codex", &now)
                });
            app.record_vendor_usage(u)?;
        }
    }
    for p in app.config().providers {
        let Some(base) = p.base_url.as_deref().filter(|b| is_opencode_go(b)) else {
            continue;
        };
        let id = format!("opencode-go:{}", p.id);
        let key = match app.secret(&p.api_key_ref) {
            Ok(Some(s)) => s.expose_str().map(str::to_owned).ok(),
            _ => None,
        };
        let u = match key {
            None => VendorUsage {
                error: Some(format!("no key for provider {}", p.id)),
                ..VendorUsage::new(&id, &now)
            },
            Some(k) => fetch_opencode_go(&p.id, base, &k, &now)
                .await
                .unwrap_or_else(|e| VendorUsage {
                    error: Some(e.to_string()),
                    ..VendorUsage::new(&id, &now)
                }),
        };
        app.record_vendor_usage(u)?;
    }
    Ok(())
}

/// Reads Claude's usage now with one tiny Haiku turn (the user asked for it: it costs a little quota).
pub async fn read_claude_usage(app: &App, clis: &[agenium_cli_backend::CliInfo]) -> Result<()> {
    let now = agenium_core::now_iso();
    let program = clis
        .iter()
        .find(|c| c.kind == CliKind::Claude && c.compatible)
        .and_then(|c| c.path.clone())
        .ok_or_else(|| OrchError::Invalid("Claude Code is not installed".into()))?;
    let scratch = app.layout().agents().join(".usage-probe");
    let u = agenium_cli_backend::claude::rate_limits(&program, &scratch)
        .await
        .unwrap_or_else(|e| VendorUsage {
            error: Some(e),
            ..VendorUsage::new("claude", &now)
        });
    app.record_vendor_usage(u)
}
