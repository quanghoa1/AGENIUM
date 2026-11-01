//! What a run's model can do (context, vision, effort) and the CLI backends' runs.

use super::*;

/// How long an agent CLI's detection is reused (a login or upgrade shows up within this).
pub(super) const CLI_INFO_TTL: std::time::Duration = std::time::Duration::from_secs(60);

/// `info` with the hand-set capabilities of the provider's configuration applied (they win, ADR-31).
pub fn apply_caps(cfg: &agenium_config::ProviderConfig, info: &mut agenium_llm::ModelInfo) {
    if let Some(c) = cfg.caps(&info.id) {
        info.supports_tools = c.tools.or(info.supports_tools);
        info.supports_vision = c.vision.or(info.supports_vision);
        info.context_window = c.context_window.or(info.context_window);
        if let Some(levels) = &c.efforts {
            info.efforts = Some(levels.iter().filter_map(|e| e.parse().ok()).collect());
        }
        if let Some(d) = &c.default_effort {
            info.default_effort = d.parse().ok();
        }
    }
}

/// The context window assumed for an API model nothing describes.
pub(super) const DEFAULT_WINDOW: u64 = 200_000;

impl Runtime {
    /// Capabilities of `model` (endpoint report + hand-set flags), looked up once per runtime; `None` when the
    /// provider does not list it (the run goes ahead with defaults).
    pub async fn model_info(
        &self,
        provider_id: &str,
        model: &str,
        provider: &dyn Provider,
    ) -> Option<agenium_llm::ModelInfo> {
        let key = (provider_id.to_owned(), model.to_owned());
        if let Some(v) = self
            .model_infos
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&key)
        {
            return v.clone();
        }
        let listed =
            tokio::time::timeout(std::time::Duration::from_secs(15), provider.list_models())
                .await
                .ok()
                .and_then(|r| r.ok());
        // A failed or slow listing is not remembered: the next run asks again.
        let answered = listed.is_some();
        let mut info = listed
            .unwrap_or_default()
            .into_iter()
            .find(|m| m.id == model);
        if let Some(pc) = self.app.config().provider(provider_id) {
            if info.is_none() && pc.caps(model).is_some() {
                info = Some(agenium_llm::ModelInfo {
                    id: model.to_owned(),
                    display_name: None,
                    context_window: None,
                    max_output: None,
                    supports_tools: None,
                    supports_vision: None,
                    efforts: None,
                    default_effort: None,
                });
            }
            if let Some(i) = info.as_mut() {
                apply_caps(pc, i);
            }
        }
        if answered {
            self.model_infos
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(key, info.clone());
        }
        info
    }

    /// The context window `agent`'s model works in (ADR-155): what its provider lists or the user set, else the
    /// models.dev catalogue, else 200 000 for an API model; a CLI's model from the catalogue, else 0 (not known: the
    /// CLI keeps its own context).
    pub async fn window_of(&self, agent: &agenium_store::core::agents::Agent) -> u64 {
        self.window_for(agent.backend, &agent.model.provider, &agent.model.model)
            .await
    }

    /// The context window of `model` of `provider` on `backend`, as [`Runtime::window_of`] finds it.
    pub async fn window_for(&self, backend: BackendKind, provider: &str, model: &str) -> u64 {
        if backend == BackendKind::Cli {
            return self.app.catalogue_window(provider, model).unwrap_or(0);
        }
        let listed = match self.providers.provider(provider) {
            Ok(p) => self
                .model_info(provider, model, p.as_ref())
                .await
                .and_then(|i| i.context_window),
            Err(_) => None,
        };
        listed
            .or_else(|| self.app.catalogue_window(provider, model))
            .unwrap_or(DEFAULT_WINDOW)
    }

    /// Whether `agent`'s API model takes images (unknown counts as no: an image sent to a text model fails the call).
    pub(super) async fn vision(&self, agent: &agenium_store::core::agents::Agent) -> bool {
        match self.providers.provider(&agent.model.provider) {
            Ok(p) => {
                self.model_info(&agent.model.provider, &agent.model.model, p.as_ref())
                    .await
                    .and_then(|i| i.supports_vision)
                    == Some(true)
            }
            Err(_) => false,
        }
    }

    /// The CLI's detection, made off the async threads (it runs the CLI's version and login probes) and reused for
    /// a short while so back-to-back runs do not probe again.
    pub(super) async fn cli_info(
        &self,
        cli: agenium_cli_backend::CliKind,
    ) -> agenium_cli_backend::detect::CliInfo {
        if let Some((at, info)) = self
            .cli_infos
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&cli)
        {
            if at.elapsed() < CLI_INFO_TTL {
                return info.clone();
            }
        }
        let info = tokio::task::spawn_blocking(move || agenium_cli_backend::detect::detect(cli))
            .await
            .unwrap_or_else(|_| agenium_cli_backend::detect::CliInfo {
                kind: cli,
                path: None,
                version: None,
                login: agenium_cli_backend::Login::Unknown,
                compatible: false,
                note: Some("detection failed".into()),
            });
        self.cli_infos
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(cli, (std::time::Instant::now(), info.clone()));
        info
    }

    /// Drops what this runtime remembers about a provider (its client and its models' capabilities) after the
    /// user edits or removes it.
    pub fn forget_provider(&self, id: &str) {
        self.providers.forget(id);
        self.model_infos
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|(p, _), _| p != id);
    }

    /// Whether the agent's CLI can run now: known, driven by Agenium, installed, at least the checked version, not
    /// logged out, and (for a CLI that must run contained) a sandbox backend works. `Err` = why not.
    pub(super) async fn cli_ready(
        &self,
        agent: &agenium_store::core::agents::Agent,
    ) -> std::result::Result<(&'static dyn agenium_cli_backend::Driver, PathBuf), String> {
        self.cli_ready_named(&agent.model.provider).await
    }

    /// [`Self::cli_ready`] for the CLI program `name` (an agent's or a side call's, ADR-151).
    pub(super) async fn cli_ready_named(
        &self,
        name: &str,
    ) -> std::result::Result<(&'static dyn agenium_cli_backend::Driver, PathBuf), String> {
        use agenium_cli_backend::{CliKind, Control, Login};
        let cli = CliKind::ALL
            .into_iter()
            .find(|k| k.program() == name)
            .ok_or_else(|| format!("{name:?} is not a known agent CLI"))?;
        let driver = agenium_cli_backend::driver(cli)
            .ok_or_else(|| format!("Agenium does not drive {name} yet (ADR-56)"))?;
        let info = self.cli_info(cli).await;
        let program = driver.program(
            &info
                .path
                .clone()
                .ok_or_else(|| format!("{name} is not installed (not on PATH)"))?,
        );
        if !info.compatible {
            return Err(format!(
                "{name} cannot be used: {}",
                info.note.unwrap_or_default()
            ));
        }
        if info.login == Login::NotLoggedIn {
            return Err(format!(
                "{name} is not logged in; log in with the {name} CLI first"
            ));
        }
        if driver.control() == Control::Sandbox
            && agenium_sandbox::detect().backend.is_none()
            && !self.app.config().sandbox.unsandboxed
        {
            return Err(format!(
                "{name} runs only inside Agenium's sandbox and no sandbox backend works here (Settings can allow \
running without one)"
            ));
        }
        Ok((driver, program))
    }

    /// CliBackend run (M2.5, ADR-56): the agent's `model.provider` names the CLI, `model.model` the CLI's model.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn cli_run(
        &self,
        agent: &agenium_store::core::agents::Agent,
        driver: &'static dyn agenium_cli_backend::Driver,
        program: PathBuf,
        kind: RunKind,
        ctx: &ToolCtx,
        protocol: &RunProtocol,
        protocol_specs: Vec<ToolSpec>,
        tools: Vec<Arc<dyn Tool>>,
        observer: &RunObserver,
        order: &WorkOrder,
        cancel: Cancel,
        steer: Option<agenium_backend::Steer>,
    ) -> Result<RunReport> {
        use agenium_cli_backend::{CliRun, Limits};
        let name = agent.model.provider.as_str();
        let cfg = self.app.config();
        let rt = cfg.runtime;
        // A work run keeps the CLI's session of its ticket for the ticket's next run, which resumes it with what is new
        // (ADR-134); a fresh start (one message) resumes nothing.
        let session = match (kind, &protocol.ticket) {
            (RunKind::Work, Some(t)) => {
                let kept = self.app.cli_sessions(agent.id);
                agenium_cli_backend::run::SessionStore::sweep(&kept);
                Some(agenium_cli_backend::run::SessionStore {
                    dir: kept.join(t.to_string()),
                    held: order.messages.len().saturating_sub(1),
                })
            }
            _ => None,
        };
        let r = CliRun {
            driver,
            program,
            model: agent.model.model.clone(),
            effort: Some(agent.model.effort.clone()),
            env: ctx.env.clone(),
            cwd: ctx.root.clone(),
            label: format!("{name}:{}", agent.model.model),
            protocol,
            protocol_specs,
            tools,
            ctx,
            session,
            observer,
            cancel,
            limits: Limits {
                max_steps: rt.max_steps_per_run,
                same_call_limit: rt.loop_detect_same_call,
                nudge_limit: 2,
                refusal_limit: rt.tool_error_stop,
                timeout: std::time::Duration::from_secs(
                    u64::from(cfg.cli.run_timeout_mins.max(1)) * 60,
                ),
            },
            chat: kind == RunKind::Chat,
            nudge: nudge_for(kind),
            // Only Solo agents may start the CLI's own subagents (the owner's rule; a team delegates by tickets).
            subagents: protocol.solo,
            // Only a work run changes files: a Lead's chat in the main checkout, a review of the worktree it judges,
            // an inbox or plan run must not edit them behind the tickets (the CLI's own Write/Edit tools included).
            read_only: kind != RunKind::Work,
            // `cli_ready` let it through: no sandbox works and the user accepted running without one (ADR-117).
            uncontained: driver.control() == agenium_cli_backend::Control::Sandbox
                && agenium_sandbox::detect().backend.is_none(),
            on_limits: {
                let app = self.app.clone();
                Some(Arc::new(move |u| {
                    if let Err(e) = app.record_vendor_usage(u) {
                        tracing::warn!(error = %e, "vendor usage not recorded");
                    }
                }))
            },
            steer,
        };
        Ok(r.run(order).await)
    }

    /// Estimated cost of a run of an API model (0 while the model has no price; priced again when the catalogue
    /// is refreshed).
    pub(super) fn cost(&self, provider: &str, model: &str, u: &agenium_llm::Usage) -> f64 {
        let usage = agenium_core::UsageSummary {
            tokens_in: u.total_input(),
            tokens_out: u.output,
            cache_read: u.cache_read,
            cache_write: u.cache_write,
            cache_write_1h: u.cache_write_1h,
            ..Default::default()
        };
        self.app
            .usage_cost(&self.app.prices(), provider, model, &usage)
            .unwrap_or(0.0)
    }
}
