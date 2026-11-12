//! markitdown in Settings (ADR-152). Agenium installs its own copy into `<root>/tools/markitdown` on request, at a
//! pinned version, with uv when it is on the PATH, else in a virtual environment of the Python found, else with a uv it
//! downloads (pinned, checksummed); a copy is kept only when it reads Agenium's sample documents of each format (the
//! self-test). Once a week it looks on PyPI for a newer version and only says so; an update installs side by side and
//! is self-tested, and the copy in use stays when the new one fails. Removing deletes the folder, with uv, the Python
//! it fetched and the cache.

mod install;
mod uv;

use std::path::{Path, PathBuf};

use agenium_documents::markitdown::{self, own};
use serde::Serialize;

use crate::app::App;
use crate::{OrchError, Result};

pub use install::Method;

/// The version installed (the one Agenium's samples were read with).
pub const VERSION: &str = "0.1.9";
/// markitdown's readers installed: the document formats (no audio, YouTube or Azure readers, which need the network).
pub const EXTRAS: &str = "pdf,docx,pptx,xlsx,xls,outlook";
const CHECK_EVERY_DAYS: i64 = 7;
const PYPI: &str = "https://pypi.org/pypi/markitdown/json";

/// One install, update, test or removal at a time.
static BUSY: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// What Settings shows about markitdown.
#[derive(Debug, Clone, Serialize)]
pub struct MarkitdownStatus {
    /// `agenium` (its own copy), `path` (the user's, found on the PATH) or `none`.
    pub source: &'static str,
    pub program: Option<String>,
    pub version: Option<String>,
    /// How Agenium's copy was installed: `uv`, `python` or `uv-standalone`.
    pub method: Option<String>,
    pub installed_at: Option<String>,
    /// The version an install puts in place.
    pub pinned: &'static str,
    /// The newest version on PyPI at the last weekly check, and whether it is newer than Agenium's copy.
    pub latest: Option<String>,
    pub update: bool,
    pub checked_at: Option<String>,
    /// The last self-test of Agenium's copy: (format, `ok` or why not).
    pub tested_at: Option<String>,
    pub tests: Vec<(String, String)>,
    /// An install, update or removal is under way.
    pub busy: bool,
    /// Disk used by Agenium's copy (with its Python and uv).
    pub size_mb: u64,
    /// No sandbox works here and running without one is off: documents are read by Agenium's converters only.
    pub blocked: bool,
}

fn some(s: String) -> Option<String> {
    (!s.is_empty()).then_some(s)
}

fn size_of(dir: &Path) -> u64 {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return 0;
    };
    rd.flatten()
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => size_of(&e.path()),
            Ok(_) => e.metadata().map(|m| m.len()).unwrap_or(0),
            Err(_) => 0,
        })
        .sum()
}

/// Whether version `a` is newer than `b` (dotted numbers; a pre-release suffix counts as older than its release).
pub fn newer(a: &str, b: &str) -> bool {
    fn parts(v: &str) -> Vec<(u64, bool)> {
        v.split('.')
            .map(|p| {
                let digits: String = p.chars().take_while(char::is_ascii_digit).collect();
                (digits.parse().unwrap_or(0), digits.len() == p.len())
            })
            .collect()
    }
    let (pa, pb) = (parts(a), parts(b));
    for i in 0..pa.len().max(pb.len()) {
        let x = pa.get(i).copied().unwrap_or((0, true));
        let y = pb.get(i).copied().unwrap_or((0, true));
        if x != y {
            return x.0 > y.0 || (x.0 == y.0 && x.1 && !y.1);
        }
    }
    false
}

/// The newest markitdown on PyPI.
async fn latest_on_pypi() -> Result<String> {
    let http =
        agenium_llm::http::Http::default_client().map_err(|e| OrchError::Invalid(e.to_string()))?;
    let v: serde_json::Value = http
        .client()
        .get(PYPI)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| OrchError::Invalid(format!("PyPI: {e}")))?
        .json()
        .await
        .map_err(|e| OrchError::Invalid(format!("PyPI: {e}")))?;
    v["info"]["version"]
        .as_str()
        .filter(|s| valid_version(s))
        .map(str::to_owned)
        .ok_or_else(|| OrchError::Invalid("PyPI did not name a version".into()))
}

/// A version as PyPI writes one (it goes into a pip requirement and a folder name).
fn valid_version(v: &str) -> bool {
    !v.is_empty()
        && v.len() <= 32
        && v.starts_with(|c: char| c.is_ascii_digit())
        && v.chars().all(|c| c.is_ascii_alphanumeric() || c == '.')
}

impl App {
    /// The folder of Agenium's own markitdown.
    pub fn markitdown_dir(&self) -> PathBuf {
        self.layout().tools().join("markitdown")
    }

    pub async fn markitdown_status(&self) -> MarkitdownStatus {
        let dir = self.markitdown_dir();
        let s = own::read_state(&dir);
        let busy = BUSY.try_lock().is_err();
        let blocked = agenium_sandbox::detect().backend.is_none()
            && !self.config().sandbox.runs_unsandboxed();
        let mut st = MarkitdownStatus {
            source: "none",
            program: None,
            version: None,
            method: None,
            installed_at: None,
            pinned: VERSION,
            latest: None,
            update: false,
            checked_at: None,
            tested_at: None,
            tests: Vec::new(),
            busy,
            size_mb: size_of(&dir) >> 20,
            blocked,
        };
        if let Some((s, program)) = own::current(&dir) {
            st.source = "agenium";
            st.program = Some(program.display().to_string());
            st.update = !s.latest.is_empty() && newer(&s.latest, &s.version);
            st.version = some(s.version);
            st.method = some(s.method);
            st.installed_at = some(s.installed_at);
            st.latest = some(s.latest);
            st.checked_at = some(s.checked_at);
            st.tested_at = some(s.tested_at);
            st.tests = s.tests;
        } else if let Some(m) = markitdown::on_path() {
            st.source = "path";
            st.program = Some(m.program.display().to_string());
            st.version = m.version().await;
            st.latest = some(s.latest);
        }
        st
    }

    /// Whether the weekly look for a newer version is due (only for Agenium's own copy).
    pub fn markitdown_check_due(&self) -> bool {
        let Some((s, _)) = own::current(&self.markitdown_dir()) else {
            return false;
        };
        chrono::DateTime::parse_from_rfc3339(&s.checked_at).map_or(true, |at| {
            chrono::Utc::now().signed_duration_since(at) > chrono::Duration::days(CHECK_EVERY_DAYS)
        })
    }

    /// Looks on PyPI for the newest markitdown and keeps what it found with Agenium's copy (nothing is installed).
    pub async fn check_markitdown_update(&self) -> Result<MarkitdownStatus> {
        let latest = latest_on_pypi().await?;
        let dir = self.markitdown_dir();
        if let Ok(_g) = BUSY.try_lock() {
            if own::current(&dir).is_some() {
                let mut s = own::read_state(&dir);
                s.latest = latest;
                s.checked_at = agenium_core::now_iso();
                own::write_state(&dir, &s)?;
            }
        }
        Ok(self.markitdown_status().await)
    }

    /// Reads the samples again with the markitdown in use (Agenium's copy keeps the result).
    pub async fn test_markitdown(&self) -> Result<Vec<(String, String)>> {
        let _g = BUSY
            .try_lock()
            .map_err(|_| OrchError::Conflict("markitdown is being installed or removed".into()))?;
        let dir = self.markitdown_dir();
        if let Some((mut s, program)) = own::current(&dir) {
            s.tests = install::self_test(program).await;
            s.tested_at = agenium_core::now_iso();
            own::write_state(&dir, &s)?;
            return Ok(s.tests);
        }
        let m = markitdown::on_path()
            .ok_or_else(|| OrchError::NotFound("markitdown is not installed".into()))?;
        Ok(install::self_test(m.program).await)
    }

    /// Removes Agenium's own markitdown: its folder, with uv, the Python it fetched and the cache.
    pub async fn remove_markitdown(&self) -> Result<MarkitdownStatus> {
        {
            let _g = BUSY.try_lock().map_err(|_| {
                OrchError::Conflict("markitdown is being installed or tested".into())
            })?;
            let dir = self.markitdown_dir();
            if dir.exists() {
                std::fs::remove_dir_all(&dir)?;
            }
        }
        Ok(self.markitdown_status().await)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_by_number_and_a_pre_release_is_older() {
        assert!(newer("0.1.10", "0.1.9"));
        assert!(newer("0.2", "0.1.9"));
        assert!(!newer("0.1.9", "0.1.9"));
        assert!(!newer("0.1.8", "0.1.9"));
        assert!(newer("0.1.9", "0.1.9b1"));
        assert!(!newer("0.1.9b1", "0.1.9"));
        assert!(valid_version("0.1.9") && valid_version("0.1.8b2"));
        assert!(!valid_version("0.1; rm") && !valid_version("") && !valid_version("v1"));
    }
}
