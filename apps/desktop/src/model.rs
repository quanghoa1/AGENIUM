//! The local embedding model is required (ADR-47): memory recalls and learns by meaning with it, and a root opens only
//! once it is in place. It comes with the installer (copied into `<root>/models`), else it is downloaded once (pinned
//! revision, sha256-checked); until then the window shows the preparation instead of the app.

use std::path::Path;

use agenium_memory::local::{self, BGE_SMALL_EN as SPEC};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::state::Desktop;

/// Where the preparation stands: `checking`, `installing`, `ready` or `error`.
#[derive(Debug, Clone, Serialize)]
pub struct ModelPrep {
    pub phase: &'static str,
    pub step: String,
    pub error: Option<String>,
    pub mb: u64,
}

impl Default for ModelPrep {
    fn default() -> Self {
        Self {
            phase: "checking",
            step: String::new(),
            error: None,
            mb: SPEC.total_bytes() >> 20,
        }
    }
}

fn set(handle: &AppHandle, phase: &'static str, step: &str, error: Option<String>) {
    let d = handle.state::<Desktop>();
    let prep = {
        let mut m = d.model.lock().unwrap_or_else(|e| e.into_inner());
        m.phase = phase;
        m.step = step.to_owned();
        m.error = error;
        m.clone()
    };
    let _ = handle.emit("agenium://model", prep);
}

/// Puts the model in place, then opens the root when it is set up. Run at start and on the user's retry.
pub async fn ensure(handle: AppHandle) {
    let d = handle.state::<Desktop>();
    let models = agenium_config::RootLayout::new(&d.root).models();
    set(&handle, "checking", "", None);
    match prepare(&handle, &models).await {
        Ok(()) => {
            set(&handle, "ready", "", None);
            if !agenium_config::setup::needs_setup(&d.root) && d.opened().is_err() {
                let h = handle.clone();
                let opened = tauri::async_runtime::spawn_blocking(move || {
                    h.state::<Desktop>().open(&h).map(|_| ())
                })
                .await
                .map_err(|e| e.to_string())
                .and_then(|r| r);
                if let Err(e) = opened {
                    tracing::error!("cannot open the Agenium root: {e}");
                }
            }
            let _ = handle.emit("agenium://model", d.model_prep());
        }
        Err(e) => {
            tracing::error!("the embedding model is not in place: {e}");
            set(&handle, "error", "", Some(e));
        }
    }
}

/// Copies the model's files from `src` to `dst`.
fn copy(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for f in SPEC.files {
        std::fs::copy(src.join(f.name), dst.join(f.name))?;
    }
    Ok(())
}

async fn prepare(handle: &AppHandle, models: &Path) -> Result<(), String> {
    if local::installed(&SPEC, models) {
        return Ok(());
    }
    set(handle, "installing", "", None);
    // The installer's copy, when there is one; a development build also finds the copy the installer build fetched.
    let mut bundled: Vec<std::path::PathBuf> = Vec::new();
    if let Ok(res) = handle.path().resource_dir() {
        bundled.push(res.join("models"));
    }
    if cfg!(debug_assertions) {
        bundled.push(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("resources")
                .join("models"),
        );
    }
    for res in bundled {
        let src = SPEC.dir(&res);
        if local::installed(&SPEC, &res) {
            set(handle, "installing", "copying the bundled model", None);
            match copy(&src, &SPEC.dir(models)).map_err(|e| e.to_string()) {
                Ok(()) if local::verify(&SPEC, models).is_empty() => return Ok(()),
                Ok(()) => tracing::warn!(
                    "the bundled embedding model does not match its sha256; downloading it"
                ),
                Err(e) => {
                    tracing::warn!(error = %e, "the bundled embedding model could not be copied; downloading it")
                }
            }
        }
    }
    // The model this machine's own root already has (a second root, such as a UI test's, copies it).
    let mut own = agenium_config::root::RootInputs::from_system();
    own.env = None;
    if let Ok((root, _)) = agenium_config::root::resolve_root_with(&own) {
        let theirs = agenium_config::RootLayout::new(&root).models();
        if theirs != models && local::installed(&SPEC, &theirs) {
            set(
                handle,
                "installing",
                "copying the model from this computer's Agenium",
                None,
            );
            if copy(&SPEC.dir(&theirs), &SPEC.dir(models)).is_ok()
                && local::verify(&SPEC, models).is_empty()
            {
                return Ok(());
            }
        }
    }
    let http = agenium_llm::http::Http::default_client().map_err(|e| e.to_string())?;
    let h = handle.clone();
    local::install(&SPEC, models, &http, &move |m| {
        set(&h, "installing", m, None)
    })
    .await?;
    match local::verify(&SPEC, models) {
        bad if bad.is_empty() => Ok(()),
        bad => Err(format!("files missing or corrupt: {}", bad.join(", "))),
    }
}
