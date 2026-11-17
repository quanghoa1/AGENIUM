//! The web search (ADR-163): agents whose CLI does not search the web itself get Agenium's `web_search`. It goes to
//! the APIs the user set keys for, the strongest first (Exa, Tavily, Brave Search), and to Exa's free search always last:
//! one out of its limit hands the search to the next. One key per API, told apart by the key, kept in the secret store
//! like the API keys.

use agenium_tools::SearchApi;
use serde::Serialize;

use crate::{App, OrchError, Result};

const APIS: [SearchApi; 3] = [SearchApi::Exa, SearchApi::Tavily, SearchApi::Brave];

fn secret(api: SearchApi) -> String {
    format!(
        "web-search/{}",
        api.name().to_ascii_lowercase().replace(' ', "-")
    )
}

/// One API in the order searches go to it.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SearchStep {
    pub api: &'static str,
    /// Whether the user's key is set for it (Exa runs without one, free and limited).
    pub key: bool,
}

/// What Settings shows: the order searches go in and which keys are set (the keys themselves never leave the store).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct WebSearchStatus {
    pub order: Vec<SearchStep>,
}

impl App {
    /// The user's search keys, registered with the redactor so they never show in a log or a transcript.
    pub fn web_search_keys(&self) -> Vec<String> {
        APIS.iter()
            .filter_map(|a| self.secret(&secret(*a)).ok().flatten())
            .filter_map(|s| s.expose_str().ok().map(str::to_owned))
            .collect()
    }

    pub fn web_search_status(&self) -> WebSearchStatus {
        let keys = self.web_search_keys();
        let tool = agenium_tools::WebSearchTool::new(&keys);
        WebSearchStatus {
            order: tool
                .order()
                .into_iter()
                .enumerate()
                .map(|(i, a)| SearchStep {
                    api: a.name(),
                    key: !tool.free(i),
                })
                .collect(),
        }
    }

    /// Keeps `key` as the key of its API (Tavily `tvly-`, Brave `BSA`, Exa otherwise), replacing that API's key.
    pub fn set_web_search_key(&self, key: &str) -> Result<WebSearchStatus> {
        let key = key.trim();
        if key.len() < 8 || key.chars().any(char::is_whitespace) {
            return Err(OrchError::Invalid(
                "that does not look like a search API key of Exa, Tavily or Brave".into(),
            ));
        }
        self.secrets()
            .set(&secret(SearchApi::of_key(key)), key.as_bytes())?;
        Ok(self.web_search_status())
    }

    /// Removes the key of `api` (its name as the status gives it).
    pub fn remove_web_search_key(&self, api: &str) -> Result<WebSearchStatus> {
        let a = APIS
            .into_iter()
            .find(|a| a.name() == api)
            .ok_or_else(|| OrchError::NotFound(format!("search API {api:?}")))?;
        self.secrets().delete(&secret(a))?;
        Ok(self.web_search_status())
    }

    /// The `web_search` tool for a run, unless the agent's CLI searches by itself.
    pub(crate) fn web_search_tool(
        &self,
        cli_searches: bool,
    ) -> Option<std::sync::Arc<dyn agenium_tools::Tool>> {
        if cli_searches {
            return None;
        }
        Some(std::sync::Arc::new(agenium_tools::WebSearchTool::new(
            &self.web_search_keys(),
        )))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    fn order(app: &App) -> Vec<(&'static str, bool)> {
        app.web_search_status()
            .order
            .into_iter()
            .map(|s| (s.api, s.key))
            .collect()
    }

    #[test]
    fn keys_of_several_apis_are_kept_in_order_and_removed_one_by_one() {
        let tmp = tempfile::tempdir().unwrap();
        let (app, _) = App::open(
            &tmp.path().join("root"),
            crate::AppOptions {
                keyring: Arc::new(agenium_secrets::MemoryKeyring::default()),
                checks: Arc::new(crate::NoCheckFactory),
            },
        )
        .unwrap();
        assert_eq!(order(&app), [("Exa", false)], "no key: Exa's free search");
        assert!(app.web_search_tool(false).is_some());
        assert!(
            app.web_search_tool(true).is_none(),
            "a CLI that searches by itself"
        );
        assert!(app.set_web_search_key("tvly x").is_err());
        app.set_web_search_key("BSA0123456789").unwrap();
        app.set_web_search_key(" tvly-dev-0123456789 ").unwrap();
        assert_eq!(
            order(&app),
            [("Tavily", true), ("Brave Search", true), ("Exa", false)],
            "the keys first, the free search last"
        );
        app.set_web_search_key("tvly-dev-replaced1").unwrap();
        assert_eq!(app.web_search_keys().len(), 2, "one key per API");
        assert!(app
            .web_search_keys()
            .contains(&"tvly-dev-replaced1".to_owned()));
        app.remove_web_search_key("Tavily").unwrap();
        assert_eq!(order(&app), [("Brave Search", true), ("Exa", false)]);
        app.set_web_search_key("exa-0123456789").unwrap();
        assert_eq!(
            order(&app),
            [("Exa", true), ("Brave Search", true), ("Exa", false)],
            "an Exa key goes first; the free search stays last"
        );
        assert!(app.remove_web_search_key("Bing").is_err());
    }
}
