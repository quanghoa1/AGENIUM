//! Model providers: added, changed (headers, keys in the vault) and removed.

use super::*;

/// One extra header of a provider. A secret one is kept in the vault (`provider/<id>/header/<name>`) and the
/// configuration holds a `${secret:...}` placeholder; its value `None` keeps the stored secret.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct HeaderInput {
    pub name: String,
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub secret: bool,
}

/// A provider as the user enters it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderInput {
    pub id: String,
    pub kind: ProviderKind,
    #[serde(default)]
    pub base_url: Option<String>,
    /// New API key (stored in the vault as `provider/<id>`); `None` keeps the stored one.
    #[serde(default)]
    pub api_key: Option<String>,
    /// Drop the stored key (a local endpoint without a key).
    #[serde(default)]
    pub clear_key: bool,
    #[serde(default = "four")]
    pub max_concurrency: u32,
    /// Models listed by hand (endpoints that cannot list them).
    #[serde(default)]
    pub models: Vec<String>,
    /// Extra headers (OpenAI-compatible endpoints); `None` keeps the stored ones.
    #[serde(default)]
    pub headers: Option<Vec<HeaderInput>>,
    /// Per-model capabilities set by hand (tools, vision, context, effort levels); `None` keeps the stored ones.
    #[serde(default)]
    pub model_caps: Option<Vec<agenium_config::ModelCaps>>,
    /// Extra request body fields (OpenAI-compatible endpoints); `None` keeps the stored ones.
    #[serde(default)]
    pub body: Option<Vec<agenium_config::BodyField>>,
    /// Where the effort level goes; `None` keeps the stored one, `""` goes back to `reasoning_effort`.
    #[serde(default)]
    pub effort_field: Option<String>,
}

pub(super) fn four() -> u32 {
    4
}

impl App {
    /// Adds or replaces a provider. A new key goes to the vault (`provider/<id>`), never to config.toml.
    /// A provider's extra headers as stored: plain values as they are, secret ones in the vault behind a placeholder
    /// (a secret header left empty keeps its stored value). Headers no longer listed lose their secret.
    pub(super) fn provider_headers(
        &self,
        id: &str,
        hs: &[HeaderInput],
        old: Option<&ProviderConfig>,
    ) -> Result<std::collections::BTreeMap<String, String>> {
        let mut out = std::collections::BTreeMap::new();
        for h in hs {
            let name = h.name.trim();
            if name.is_empty() {
                continue;
            }
            let secret = format!("provider/{id}/header/{}", name.to_ascii_lowercase());
            let value = h.value.as_deref().map(str::trim).unwrap_or("");
            agenium_llm::custom::check_header(name, value).map_err(invalid)?;
            if h.secret {
                if !value.is_empty() {
                    self.secrets().set(&secret, value.as_bytes())?;
                } else if self.secret(&secret)?.is_none() {
                    return Err(invalid(format!("header {name}: give its value")));
                }
                out.insert(name.to_owned(), format!("${{secret:{secret}}}"));
            } else {
                if value.contains("${") {
                    return Err(invalid(format!(
                        "header {name}: mark it secret instead of writing a placeholder"
                    )));
                }
                let _ = self.secrets().delete(&secret);
                out.insert(name.to_owned(), value.to_owned());
            }
        }
        for gone in old
            .into_iter()
            .flat_map(|o| o.headers.keys())
            .filter(|k| !out.contains_key(*k))
        {
            let _ = self.secrets().delete(&format!(
                "provider/{id}/header/{}",
                gone.to_ascii_lowercase()
            ));
        }
        Ok(out)
    }

    pub fn put_provider(&self, p: &ProviderInput) -> Result<ProviderConfig> {
        if !slug_ok(&p.id) {
            return Err(invalid(format!(
                "provider id {:?}: lowercase letters, digits and '-'",
                p.id
            )));
        }
        if p.kind == ProviderKind::Mock {
            return Err(invalid("mock providers are for tests"));
        }
        if let Some(u) = &p.base_url {
            if !(u.starts_with("http://") || u.starts_with("https://")) {
                return Err(invalid(format!(
                    "base URL {u:?} must start with http:// or https://"
                )));
            }
            if let Some(ph) = agenium_config::presets::placeholder(u) {
                return Err(invalid(format!(
                    "base URL {u:?}: replace {ph} with your own value"
                )));
            }
        }
        if p.kind == ProviderKind::OpenaiCompat && p.base_url.as_deref().unwrap_or("").is_empty() {
            return Err(invalid("an OpenAI-compatible provider needs a base URL"));
        }
        let secret = format!("provider/{}", p.id);
        let old = self.config().provider(&p.id).cloned();
        let key_ref = match (&p.api_key, p.clear_key) {
            (Some(k), _) if !k.trim().is_empty() => {
                self.secrets().set(&secret, k.trim().as_bytes())?;
                secret.clone()
            }
            (_, true) => {
                let _ = self.secrets().delete(&secret);
                String::new()
            }
            _ => old
                .as_ref()
                .map(|o| o.api_key_ref.clone())
                .unwrap_or_default(),
        };
        if key_ref.is_empty() && matches!(p.kind, ProviderKind::Anthropic | ProviderKind::Gemini) {
            return Err(invalid("this provider needs an API key"));
        }
        let headers = match &p.headers {
            Some(hs) => self.provider_headers(&p.id, hs, old.as_ref())?,
            None => old.as_ref().map(|o| o.headers.clone()).unwrap_or_default(),
        };
        for c in p.model_caps.iter().flatten() {
            for e in c.efforts.iter().flatten().chain(c.default_effort.iter()) {
                e.parse::<agenium_core::Effort>().map_err(invalid)?;
            }
        }
        // Extra body fields and the effort field follow the same rules here as when a request is built.
        let body = match &p.body {
            Some(fs) => {
                let fs: Vec<&agenium_config::BodyField> =
                    fs.iter().filter(|f| !f.path.trim().is_empty()).collect();
                if fs.len() > agenium_llm::custom::MAX_FIELDS {
                    return Err(invalid(format!(
                        "at most {} extra fields",
                        agenium_llm::custom::MAX_FIELDS
                    )));
                }
                for f in &fs {
                    agenium_llm::custom::check_field(&f.path, &f.value, f.model.as_deref())
                        .map_err(invalid)?;
                }
                fs.into_iter()
                    .map(|f| agenium_config::BodyField {
                        path: f.path.trim().to_owned(),
                        value: f.value.trim().to_owned(),
                        model: f
                            .model
                            .as_deref()
                            .map(str::trim)
                            .filter(|m| !m.is_empty())
                            .map(str::to_owned),
                    })
                    .collect()
            }
            None => old.as_ref().map(|o| o.body.clone()).unwrap_or_default(),
        };
        let effort_field = match &p.effort_field {
            Some(f) if f.trim().is_empty() => None,
            Some(f) => {
                agenium_llm::custom::check_path(f).map_err(invalid)?;
                Some(f.trim().to_owned())
            }
            None => old.as_ref().and_then(|o| o.effort_field.clone()),
        };
        let pc = ProviderConfig {
            body,
            effort_field,
            headers,
            id: p.id.clone(),
            kind: p.kind,
            base_url: p.base_url.clone().filter(|u| !u.trim().is_empty()),
            api_key_ref: key_ref,
            max_concurrency: p.max_concurrency.max(1),
            models: p.models.clone(),
            model_caps: match &p.model_caps {
                Some(c) => c
                    .iter()
                    .filter(|c| !c.model.trim().is_empty())
                    .cloned()
                    .collect(),
                None => old.map(|o| o.model_caps).unwrap_or_default(),
            },
        };
        self.update_config(|c| {
            c.providers.retain(|x| x.id != pc.id);
            c.providers.push(pc.clone());
            c.providers.sort_by(|a, b| a.id.cmp(&b.id));
        })?;
        // Its endpoint, key or named models may have changed: its models are listed again when next shown.
        self.forget_models(&crate::model_cache::api_key(&pc.id))?;
        Ok(pc)
    }

    /// Removes a provider no agent (and no CLI fallback) uses, with its key.
    pub fn remove_provider(&self, id: &str) -> Result<()> {
        let cfg = self.config();
        let pc = cfg
            .provider(id)
            .cloned()
            .ok_or_else(|| OrchError::NotFound(format!("provider {id:?}")))?;
        let users: Vec<String> = self
            .agents()?
            .into_iter()
            .filter(|a| a.backend == BackendKind::Api && a.model.provider == id)
            .map(|a| a.name)
            .collect();
        if !users.is_empty() {
            return Err(OrchError::Conflict(format!(
                "agents use provider {id}: {}; switch their model first",
                users.join(", ")
            )));
        }
        if cfg.cli.fallback_provider == id {
            return Err(OrchError::Conflict(format!(
                "provider {id} is the CLI fallback; change it first"
            )));
        }
        self.update_config(|c| c.providers.retain(|x| x.id != id))?;
        if !pc.api_key_ref.is_empty() {
            let _ = self.secrets().delete(&pc.api_key_ref);
        }
        self.forget_models(&crate::model_cache::api_key(id))?;
        Ok(())
    }
}
