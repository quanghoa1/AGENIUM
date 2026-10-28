//! OAuth for remote MCP servers (ADR-97): the authorization flow of the MCP specification (revisions 2025-06-18 and
//! 2025-11-25). A 401 from the server names its protected-resource metadata (RFC 9728), which names the authorization
//! server; its metadata (RFC 8414 or OpenID discovery) gives the endpoints. Agenium registers itself (RFC 7591)
//! unless a client is configured, and signs the user in with the authorization code flow, PKCE S256 and a loopback
//! redirect (RFC 8252), asking for a token bound to this server (`resource`, RFC 8707).
//!
//! Tokens are kept per server in a [`TokenStore`] (the vault). A token is sent only to the server it was issued for.
//! One [`Bearer`] per server refreshes an access token before it expires, and after a 401 first takes a newer token
//! from the store (a login since), then refreshes once, then asks the user to sign in again.
//! Not built: Client ID Metadata Documents (2025-11-25; dynamic registration or a configured client instead), and
//! step-up authorization asks the user to sign in again with the wider scope rather than doing it in place.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use crate::client::{McpError, PROTOCOL_VERSION};

mod discovery;
mod exchange;
mod login;

pub use bearer::*;
pub use discovery::*;
pub use exchange::*;
pub use login::*;
mod bearer;

type Result<T> = std::result::Result<T, McpError>;

/// How long the user has to finish signing in.
pub const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);
/// An access token this close to its expiry is refreshed before use.
const EXPIRY_MARGIN: u64 = 30;
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);

/// What Agenium keeps for one server after a login.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Tokens {
    /// The MCP endpoint the tokens were issued for; a changed `url` in the config needs a new login.
    pub server_url: String,
    /// The `resource` the tokens are bound to (RFC 8707).
    pub resource: String,
    pub issuer: String,
    pub token_endpoint: String,
    pub client_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_secret: Option<String>,
    pub access_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    /// Unix seconds; `None` when the server gave no lifetime.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
}

impl std::fmt::Debug for Tokens {
    // Never print a token.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tokens")
            .field("server_url", &self.server_url)
            .field("issuer", &self.issuer)
            .field("client_id", &self.client_id)
            .field("refresh", &self.refresh_token.is_some())
            .field("expires_at", &self.expires_at)
            .field("scope", &self.scope)
            .finish()
    }
}

impl Tokens {
    fn expires_soon(&self) -> bool {
        self.expires_at
            .is_some_and(|t| now().saturating_add(EXPIRY_MARGIN) >= t)
    }
}

/// Where tokens live between runs (the vault in Agenium, memory in tests).
pub trait TokenStore: Send + Sync {
    fn load(&self, server: &str) -> Option<Tokens>;
    fn save(&self, server: &str, tokens: &Tokens) -> std::result::Result<(), String>;
    fn remove(&self, server: &str) -> std::result::Result<(), String>;
}

/// A [`TokenStore`] in memory.
#[derive(Default)]
pub struct MemoryTokens(Mutex<HashMap<String, Tokens>>);

impl TokenStore for MemoryTokens {
    fn load(&self, server: &str) -> Option<Tokens> {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(server)
            .cloned()
    }
    fn save(&self, server: &str, tokens: &Tokens) -> std::result::Result<(), String> {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(server.to_owned(), tokens.clone());
        Ok(())
    }
    fn remove(&self, server: &str) -> std::result::Result<(), String> {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(server);
        Ok(())
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn auth(msg: impl Into<String>) -> McpError {
    McpError::Auth(msg.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn challenges() {
        let c = parse_challenge(
            r#"Bearer resource_metadata="https://mcp.example.com/.well-known/oauth-protected-resource/mcp", scope="files:read files:write""#,
        )
        .unwrap();
        assert_eq!(
            c.resource_metadata.as_deref(),
            Some("https://mcp.example.com/.well-known/oauth-protected-resource/mcp")
        );
        assert_eq!(c.scope.as_deref(), Some("files:read files:write"));
        let c = parse_challenge(r#"Bearer error="insufficient_scope", scope="a\"b""#).unwrap();
        assert_eq!(c.error.as_deref(), Some("insufficient_scope"));
        assert_eq!(c.scope.as_deref(), Some("a\"b"));
        assert_eq!(parse_challenge("Bearer").unwrap(), Challenge::default());
        assert!(parse_challenge(r#"Basic realm="x""#).is_none());
    }

    /// RFC 7636 appendix B.
    #[test]
    fn pkce_s256() {
        assert_eq!(
            s256("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        let (v, c) = pkce();
        assert_eq!(v.len(), 43);
        assert_eq!(s256(&v), c);
        assert_eq!(b64url(b"f"), "Zg");
        assert_eq!(b64url(b"fo"), "Zm8");
        assert_eq!(b64url(b"foo"), "Zm9v");
    }

    #[test]
    fn metadata_places() {
        let s = Url::parse("https://example.com/public/mcp").unwrap();
        let v: Vec<String> = resource_metadata_urls(&s)
            .iter()
            .map(Url::to_string)
            .collect();
        assert_eq!(
            v,
            [
                "https://example.com/.well-known/oauth-protected-resource/public/mcp",
                "https://example.com/.well-known/oauth-protected-resource"
            ]
        );
        let i = Url::parse("https://auth.example.com/tenant1").unwrap();
        let v: Vec<String> = authorization_metadata_urls(&i)
            .iter()
            .map(Url::to_string)
            .collect();
        assert_eq!(
            v,
            [
                "https://auth.example.com/.well-known/oauth-authorization-server/tenant1",
                "https://auth.example.com/.well-known/openid-configuration/tenant1",
                "https://auth.example.com/tenant1/.well-known/openid-configuration"
            ]
        );
        let i = Url::parse("https://auth.example.com").unwrap();
        assert_eq!(authorization_metadata_urls(&i).len(), 2);
    }

    /// Review of 2026-10-27: the check was a string prefix, so `https://victim.com` covered
    /// `https://victim.com.evil.net/mcp`.
    #[test]
    fn the_resource_must_be_this_server_or_a_parent_path_of_it() {
        let s = Url::parse("https://mcp.example.com/api/mcp").unwrap();
        for ok in [
            "https://mcp.example.com/api/mcp",
            "https://mcp.example.com/api/mcp/",
            "https://mcp.example.com/api",
            "https://mcp.example.com",
            "https://mcp.example.com/",
            "https://MCP.example.com:443/api",
        ] {
            assert!(resource_covers(ok, &s), "{ok}");
        }
        for bad in [
            "https://mcp.example.co",
            "https://mcp.example.com.evil.net/api/mcp",
            "https://mcp.example.com/ap",
            "https://mcp.example.com/api/mcpx",
            "http://mcp.example.com/api/mcp",
            "https://mcp.example.com:8443/api/mcp",
            "https://evil.net/api/mcp",
            "not a url",
        ] {
            assert!(!resource_covers(bad, &s), "{bad}");
        }
        let evil = Url::parse("https://victim.com.evil.net/mcp").unwrap();
        assert!(!resource_covers("https://victim.com", &evil));
    }

    #[test]
    fn only_https_or_this_machine() {
        assert!(parse_allowed("https://a.example/x", "u").is_ok());
        assert!(parse_allowed("http://127.0.0.1:9/x", "u").is_ok());
        assert!(parse_allowed("http://localhost/x", "u").is_ok());
        assert!(parse_allowed("http://a.example/x", "u").is_err());
        assert!(parse_allowed("file:///etc/passwd", "u").is_err());
    }
}
