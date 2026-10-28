//! Provider commands: configure model providers. Split out of `main.rs`.

use crate::*;

pub(crate) async fn run(cli: &Cli, pc: &ProviderCmd) -> Result<()> {
    let app = open(&root(cli)?)?;
    match pc {
        ProviderCmd::Add {
            id,
            kind,
            preset,
            base_url,
            key_env,
            key_stdin,
            max_concurrency,
        } => {
            let known = match preset {
                Some(n) => Some(
                    agenium_config::presets::preset(n)
                        .ok_or_else(|| anyhow!("no preset {n:?} (see agenium provider presets)"))?,
                ),
                None if kind.is_none() => agenium_config::presets::preset(id),
                None => None,
            };
            let kind = match (kind.as_deref(), known) {
                (Some(k), _) => k.to_owned(),
                (None, Some(pr)) => match pr.kind {
                    ProviderKind::Anthropic => "anthropic",
                    ProviderKind::Gemini => "gemini",
                    _ => "openai_compat",
                }
                .to_owned(),
                (None, None) => {
                    bail!("give --kind, or --preset with a known vendor (agenium provider presets)")
                }
            };
            let base_url = base_url
                .clone()
                .or_else(|| known.and_then(|pr| pr.base_url.map(str::to_owned)));
            if let Some(ph) = base_url
                .as_deref()
                .and_then(agenium_config::presets::placeholder)
            {
                bail!(
                    "the base URL {} has {ph} to fill in: pass --base-url with your own value{}",
                    base_url.as_deref().unwrap_or(""),
                    known
                        .map(|pr| format!(" ({})", pr.note))
                        .unwrap_or_default()
                );
            }
            // The vendor's usual variable is read only when it is set (its value is never printed).
            let usual = known
                .map(|pr| pr.key_env)
                .filter(|v| !v.is_empty() && std::env::var(v).is_ok());
            let key = match (key_env.as_deref().or(usual), key_stdin) {
                (Some(v), _) => Some(
                    std::env::var(v)
                        .with_context(|| format!("environment variable {v} is not set"))?,
                ),
                (None, true) => Some(read_secret_stdin()?),
                (None, false) => None,
            };
            if key.is_none() && known.is_some_and(|pr| !pr.key_env.is_empty()) {
                eprintln!(
                    "note: no key given; set {} or pass --key-env/--key-stdin",
                    known.map(|pr| pr.key_env).unwrap_or_default()
                );
            }
            let secret_name = format!("provider/{id}");
            if let Some(k) = &key {
                app.secrets().set(&secret_name, k.as_bytes())?;
            }
            let kind = match kind.as_str() {
                "anthropic" => ProviderKind::Anthropic,
                "gemini" => ProviderKind::Gemini,
                _ => ProviderKind::OpenaiCompat,
            };
            let pc = ProviderConfig {
                body: vec![],
                effort_field: None,
                headers: Default::default(),
                id: id.clone(),
                kind,
                base_url: base_url.clone(),
                api_key_ref: if key.is_some() {
                    secret_name
                } else {
                    String::new()
                },
                max_concurrency: *max_concurrency,
                models: vec![],
                model_caps: vec![],
            };
            app.update_config(|c| {
                c.providers.retain(|p| p.id != pc.id);
                c.providers.push(pc);
            })?;
            println!(
                "provider {id} saved{}",
                if key.is_some() {
                    " (key stored in the vault)"
                } else {
                    ""
                }
            );
        }
        ProviderCmd::Remove { id } => {
            app.remove_provider(id)?;
            println!("provider {id} removed");
        }
        ProviderCmd::Local => {
            let http = agenium_llm::http::Http::default_client().map_err(|e| anyhow!("{e}"))?;
            let found = agenium_llm::local::discover(&http).await;
            if found.is_empty() {
                println!("no local model server answers (Ollama on 11434, LM Studio on 1234)");
            }
            for s in found {
                println!("{}\t{}\t{}", s.kind, s.base_url, s.models.join(", "));
                println!(
                    "  add: agenium provider add {} --kind openai_compat --base-url {}",
                    s.kind, s.base_url
                );
            }
        }
        ProviderCmd::Presets => unreachable!("listed without a root"),
        ProviderCmd::List => {
            for p in app.config().providers {
                println!(
                    "{}\t{:?}\t{}\t{}",
                    p.id,
                    p.kind,
                    p.base_url.unwrap_or_default(),
                    if p.api_key_ref.is_empty() {
                        "no key"
                    } else {
                        "key in vault"
                    }
                );
            }
        }
        ProviderCmd::Models { id } => {
            let ps = ConfiguredProviders::new(app.clone());
            let p = agenium_orchestrator::runtime::ProviderSource::provider(&ps, id)?;
            let pc = app.config().provider(id).cloned();
            let flag = |b: Option<bool>| match b {
                Some(true) => "yes",
                Some(false) => "no",
                None => "?",
            };
            for mut m in p.list_models().await.map_err(|e| anyhow!("{e}"))? {
                if let Some(pc) = &pc {
                    agenium_orchestrator::runtime::apply_caps(pc, &mut m);
                }
                println!(
                    "{}\ttools {}\tvision {}\t{}{}",
                    m.id,
                    flag(m.supports_tools),
                    flag(m.supports_vision),
                    m.context_window
                        .map(|c| format!("{c} tokens"))
                        .unwrap_or_default(),
                    if m.supports_tools == Some(false) {
                        "\t(no tool calling: Agenium agents cannot use it)"
                    } else {
                        ""
                    }
                );
            }
        }
    }
    Ok(())
}
