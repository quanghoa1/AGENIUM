//! Using the tokens: the bearer header of a server's requests, renewed before it expires.

use super::*;

/// The bearer token of one server, shared by every connection to it.
pub struct Bearer {
    server: String,
    url: String,
    store: Arc<dyn TokenStore>,
    current: tokio::sync::Mutex<Option<Tokens>>,
}

impl Bearer {
    pub fn new(server: &str, url: &str, store: Arc<dyn TokenStore>) -> Self {
        Self {
            server: server.to_owned(),
            url: url.to_owned(),
            store,
            current: tokio::sync::Mutex::new(None),
        }
    }

    pub fn server(&self) -> &str {
        &self.server
    }

    /// Whether this holder is for `url` (a changed config URL gets a new one).
    pub fn matches(&self, url: &str) -> bool {
        self.url == url
    }

    /// The stored tokens if they were issued for this server's URL.
    pub(super) fn stored(&self) -> Option<Tokens> {
        self.store
            .load(&self.server)
            .filter(|t| t.server_url == self.url)
    }

    pub(super) async fn renew(&self, t: &Tokens) -> Option<Tokens> {
        match refresh(t).await {
            Ok(n) => {
                if let Err(e) = self.store.save(&self.server, &n) {
                    tracing::warn!(server = %self.server, "refreshed MCP token not saved: {e}");
                }
                Some(n)
            }
            Err(e) => {
                tracing::info!(server = %self.server, "MCP token refresh failed: {e}");
                None
            }
        }
    }

    /// The access token to send (refreshed when it is about to expire); `None` = not signed in.
    pub async fn token(&self) -> Option<String> {
        let mut g = self.current.lock().await;
        if g.is_none() {
            *g = self.stored();
        }
        let t = g.clone()?;
        if !t.expires_soon() {
            return Some(t.access_token);
        }
        // Another connection or a new login may have stored a newer token.
        if let Some(s) = self.stored().filter(|s| !s.expires_soon()) {
            *g = Some(s.clone());
            return Some(s.access_token);
        }
        if t.refresh_token.is_some() {
            if let Some(n) = self.renew(&t).await {
                *g = Some(n.clone());
                return Some(n.access_token);
            }
        }
        Some(t.access_token) // let the server judge it
    }

    /// After a 401 to `sent`: a newer stored token, else one refresh. `true` = send again.
    pub async fn recover(&self, sent: Option<&str>) -> bool {
        let mut g = self.current.lock().await;
        if let Some(s) = self.stored() {
            if Some(s.access_token.as_str()) != sent {
                *g = Some(s);
                return true;
            }
        }
        let Some(t) = g.clone().filter(|t| t.refresh_token.is_some()) else {
            return false;
        };
        match self.renew(&t).await {
            Some(n) => {
                *g = Some(n);
                true
            }
            None => false,
        }
    }

    pub fn sign_in_hint(&self) -> String {
        format!(
            "MCP server {} needs sign-in: run `agenium mcp login {}` (or Sign in on the server in the app)",
            self.server, self.server
        )
    }
}
