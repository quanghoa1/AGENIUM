//! Installing a markitdown version into a new environment of `envs/`, self-testing it and switching to it; the
//! environment is removed instead when anything fails, so the copy in use stays (an update's rollback).

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use agenium_documents::markitdown::{self, own};
use agenium_documents::samples;
use serde::{Deserialize, Serialize};

use super::{uv, valid_version, MarkitdownStatus, BUSY, EXTRAS, VERSION};
use crate::app::App;
use crate::{OrchError, Result};

/// How markitdown is installed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Method {
    /// uv on the PATH (a Python 3.12 it finds or fetches into the folder).
    Uv,
    /// A virtual environment of the Python 3.10 to 3.13 on the PATH, with pip.
    Python,
    /// A uv Agenium downloads into the folder (pinned, checksummed), which fetches its Python.
    UvStandalone,
}

impl Method {
    fn name(self) -> &'static str {
        match self {
            Method::Uv => "uv",
            Method::Python => "python",
            Method::UvStandalone => "uv-standalone",
        }
    }
}

/// An install step lasts at most this long (markitdown's readers are a few hundred MB on a slow line).
const STEP_TIMEOUT: Duration = Duration::from_secs(20 * 60);

/// Runs `program args` with `env` set, without a console window. `Err` holds the last lines it printed.
async fn run(
    program: &Path,
    args: &[OsString],
    env: &[(&str, OsString)],
) -> std::result::Result<String, String> {
    let mut cmd = tokio::process::Command::new(program);
    cmd.args(args)
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true)
        .env("PYTHONUTF8", "1")
        .env("PIP_NO_INPUT", "1")
        .env_remove("PYTHONHOME")
        .env_remove("PYTHONPATH")
        .env_remove("VIRTUAL_ENV");
    for (k, v) in env {
        cmd.env(k, v);
    }
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW: the desktop app has no console
    let out = tokio::time::timeout(STEP_TIMEOUT, cmd.output())
        .await
        .map_err(|_| {
            format!(
                "{} took longer than {} min",
                program.display(),
                STEP_TIMEOUT.as_secs() / 60
            )
        })?
        .map_err(|e| format!("{}: {e}", program.display()))?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    if out.status.success() {
        Ok(text)
    } else {
        let tail: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
        let tail = tail[tail.len().saturating_sub(8)..].join("\n");
        Err(format!(
            "{} failed ({}): {tail}",
            program.display(),
            out.status
        ))
    }
}

/// The Python of a virtual environment.
fn python_in(env: &Path) -> PathBuf {
    if cfg!(windows) {
        env.join("Scripts").join("python.exe")
    } else {
        env.join("bin").join("python")
    }
}

/// A Python on the PATH a markitdown environment can be made from (3.10 to 3.13: every reader has wheels for them).
async fn find_python() -> Option<PathBuf> {
    for name in ["python3", "python"] {
        let Some(p) = agenium_sandbox::find_program(name) else {
            continue;
        };
        let probe = run(
            &p,
            &[
                "-c".into(),
                "import sys, venv; print('%d.%d' % sys.version_info[:2])".into(),
            ],
            &[],
        )
        .await;
        if let Ok(out) = probe {
            let minor = out
                .trim()
                .strip_prefix("3.")
                .and_then(|m| m.parse::<u32>().ok());
            if minor.is_some_and(|m| (10..=13).contains(&m)) {
                return Some(p);
            }
        }
    }
    None
}

/// Makes `env` and installs markitdown `version` into it with `uv` (keeping uv's Python and cache in `dir`).
async fn with_uv(
    uv: &Path,
    dir: &Path,
    env: &Path,
    version: &str,
) -> std::result::Result<(), String> {
    let vars = [
        ("UV_CACHE_DIR", dir.join("cache").into_os_string()),
        ("UV_PYTHON_INSTALL_DIR", dir.join("python").into_os_string()),
        ("UV_LINK_MODE", "copy".into()),
    ];
    let flags = ["--no-config".into(), "--no-progress".into()];
    let mut venv: Vec<OsString> = vec!["venv".into()];
    venv.extend(flags.iter().cloned());
    venv.extend(["--python".into(), "3.12".into(), env.as_os_str().to_owned()]);
    run(uv, &venv, &vars).await?;
    let mut pip: Vec<OsString> = vec!["pip".into(), "install".into()];
    pip.extend(flags.iter().cloned());
    pip.extend([
        "--python".into(),
        python_in(env).into_os_string(),
        format!("markitdown[{EXTRAS}]=={version}").into(),
    ]);
    run(uv, &pip, &vars).await.map(|_| ())
}

/// Makes `env` with `python -m venv` and installs markitdown `version` with pip.
async fn with_python(python: &Path, env: &Path, version: &str) -> std::result::Result<(), String> {
    run(
        python,
        &["-m".into(), "venv".into(), env.as_os_str().to_owned()],
        &[],
    )
    .await?;
    run(
        &python_in(env),
        &[
            "-m".into(),
            "pip".into(),
            "install".into(),
            "--disable-pip-version-check".into(),
            "--no-cache-dir".into(),
            format!("markitdown[{EXTRAS}]=={version}").into(),
        ],
        &[],
    )
    .await
    .map(|_| ())
}

/// Reads each sample with the markitdown at `program`: (format, `ok` or why not). Without a sandbox it runs directly:
/// the samples are Agenium's own, not untrusted documents.
pub(super) async fn self_test(program: PathBuf) -> Vec<(String, String)> {
    let m = markitdown::at(program);
    let mut out = Vec::new();
    for s in samples::samples() {
        let format = s.name.rsplit('.').next().unwrap_or(s.name).to_owned();
        let result = match m.convert(s.name, &s.bytes, true).await {
            Ok(c) => samples::check(&s, &c.text).map_or_else(|e| e, |()| "ok".to_owned()),
            Err(e) => e,
        };
        out.push((format, result));
    }
    out
}

impl App {
    /// Installs markitdown (`version`, default the pinned one) into a new environment by `method` (default: uv on the
    /// PATH, else the Python found, else a downloaded uv, each tried in turn), self-tests it and switches to it; the
    /// copy in use stays when anything fails. `progress` hears each step.
    pub async fn install_markitdown(
        &self,
        version: Option<String>,
        method: Option<Method>,
        progress: &(dyn Fn(&str) + Send + Sync),
    ) -> Result<MarkitdownStatus> {
        let version = version.unwrap_or_else(|| VERSION.to_owned());
        if !valid_version(&version) {
            return Err(OrchError::Invalid(format!(
                "not a markitdown version: {version}"
            )));
        }
        let outcome = {
            let _g = BUSY.try_lock().map_err(|_| {
                OrchError::Conflict("markitdown is already being installed or removed".into())
            })?;
            self.install_locked(&version, method, progress).await
        };
        outcome?;
        Ok(self.markitdown_status().await)
    }

    /// Installs the newest version the weekly check found (looking first when it has not looked yet).
    pub async fn update_markitdown(
        &self,
        progress: &(dyn Fn(&str) + Send + Sync),
    ) -> Result<MarkitdownStatus> {
        let dir = self.markitdown_dir();
        let Some((s, _)) = own::current(&dir) else {
            return Err(OrchError::NotFound(
                "Agenium has no markitdown of its own to update".into(),
            ));
        };
        let latest = if s.latest.is_empty() {
            self.check_markitdown_update()
                .await?
                .latest
                .unwrap_or_default()
        } else {
            s.latest.clone()
        };
        if !super::newer(&latest, &s.version) {
            return Err(OrchError::Invalid(format!(
                "{} is the newest version",
                s.version
            )));
        }
        let method = match s.method.as_str() {
            "uv" => Some(Method::Uv),
            "python" => Some(Method::Python),
            "uv-standalone" => Some(Method::UvStandalone),
            _ => None,
        };
        progress(&format!("updating {} to {latest}", s.version));
        self.install_markitdown(Some(latest), method, progress)
            .await
    }

    async fn install_locked(
        &self,
        version: &str,
        method: Option<Method>,
        progress: &(dyn Fn(&str) + Send + Sync),
    ) -> Result<()> {
        let dir = self.markitdown_dir();
        std::fs::create_dir_all(own::envs(&dir))?;
        let env_name = format!("{version}-{}", chrono::Utc::now().timestamp());
        let env = own::envs(&dir).join(&env_name);
        let order: Vec<Method> = match method {
            Some(m) => vec![m],
            None => vec![Method::Uv, Method::Python, Method::UvStandalone],
        };
        let mut why: Vec<String> = Vec::new();
        let mut done = None;
        for m in order {
            let made = match m {
                Method::Uv => match agenium_sandbox::find_program("uv") {
                    Some(uv) => {
                        progress(&format!(
                            "installing markitdown {version} with uv ({})",
                            uv.display()
                        ));
                        with_uv(&uv, &dir, &env, version).await
                    }
                    None => Err("uv is not on the PATH".into()),
                },
                Method::Python => match find_python().await {
                    Some(py) => {
                        progress(&format!(
                            "installing markitdown {version} with {}",
                            py.display()
                        ));
                        with_python(&py, &env, version).await
                    }
                    None => Err("no Python 3.10 to 3.13 with venv on the PATH".into()),
                },
                Method::UvStandalone => match uv::standalone(&dir, progress).await {
                    Ok(uv) => {
                        progress(&format!(
                            "installing markitdown {version} with the downloaded uv"
                        ));
                        with_uv(&uv, &dir, &env, version).await
                    }
                    Err(e) => Err(e),
                },
            };
            match made {
                Ok(()) => {
                    done = Some(m);
                    break;
                }
                Err(e) => {
                    progress(&format!("{}: {e}", m.name()));
                    why.push(format!("{}: {e}", m.name()));
                    let _ = std::fs::remove_dir_all(&env);
                }
            }
        }
        let _ = std::fs::remove_dir_all(dir.join("cache"));
        let Some(m) = done else {
            return Err(OrchError::Invalid(format!(
                "markitdown could not be installed. {}",
                why.join("; ")
            )));
        };
        progress("reading the sample documents (self-test)");
        let tests = self_test(own::program_in(&env)).await;
        let failed: Vec<String> = tests
            .iter()
            .filter(|(_, r)| r != "ok")
            .map(|(f, r)| format!("{f}: {r}"))
            .collect();
        if !failed.is_empty() {
            let _ = std::fs::remove_dir_all(&env);
            return Err(OrchError::Invalid(format!(
                "markitdown {version} did not pass the self-test, so it was not kept ({})",
                failed.join("; ")
            )));
        }
        let mut s = own::read_state(&dir);
        let old = std::mem::replace(&mut s.env, env_name.clone());
        s.version = version.to_owned();
        s.method = m.name().to_owned();
        s.installed_at = agenium_core::now_iso();
        s.tested_at = s.installed_at.clone();
        s.tests = tests;
        own::write_state(&dir, &s)?;
        // The environment replaced, and any an interrupted install left.
        if let Ok(rd) = std::fs::read_dir(own::envs(&dir)) {
            for e in rd.flatten() {
                if e.file_name().to_string_lossy() != env_name {
                    if let Err(err) = std::fs::remove_dir_all(e.path()) {
                        tracing::warn!(env = %old, error = %err, "an old markitdown environment was not removed");
                    }
                }
            }
        }
        progress(&format!("markitdown {version} installed and self-tested"));
        Ok(())
    }
}
