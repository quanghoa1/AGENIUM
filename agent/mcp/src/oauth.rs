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

// ---------------------------------------------------------------- pieces

/// A `WWW-Authenticate: Bearer ...` challenge.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Challenge {
    pub resource_metadata: Option<String>,
    pub scope: Option<String>,
    pub error: Option<String>,
}

/// Parses the Bearer challenge of a `WWW-Authenticate` value (`None` when there is none).
pub fn parse_challenge(header: &str) -> Option<Challenge> {
    let lower = header.to_ascii_lowercase();
    let start = lower.find("bearer")?;
    let mut rest = &header[start + 6..];
    let mut c = Challenge::default();
    loop {
        rest = rest.trim_start_matches([' ', ',', '\t']);
        let Some(eq) = rest.find('=') else { break };
        let key = rest[..eq].trim().to_ascii_lowercase();
        if key.contains(' ') {
            break; // the next challenge's scheme
        }
        rest = &rest[eq + 1..];
        let value = if let Some(q) = rest.strip_prefix('"') {
            let mut v = String::new();
            let mut chars = q.char_indices();
            let mut end = q.len();
            while let Some((i, ch)) = chars.next() {
                match ch {
                    '\\' => {
                        if let Some((_, n)) = chars.next() {
                            v.push(n);
                        }
                    }
                    '"' => {
                        end = i + 1;
                        break;
                    }
                    _ => v.push(ch),
                }
            }
            rest = &q[end.min(q.len())..];
            v
        } else {
            let end = rest.find(',').unwrap_or(rest.len());
            let v = rest[..end].trim().to_owned();
            rest = &rest[end..];
            v
        };
        match key.as_str() {
            "resource_metadata" => c.resource_metadata = Some(value),
            "scope" => c.scope = Some(value),
            "error" => c.error = Some(value),
            _ => {}
        }
    }
    Some(c)
}

/// PKCE (RFC 7636): a verifier and its S256 challenge.
pub fn pkce() -> (String, String) {
    let verifier = b64url(&rand::random::<[u8; 32]>());
    let challenge = s256(&verifier);
    (verifier, challenge)
}

pub fn s256(verifier: &str) -> String {
    b64url(&Sha256::digest(verifier.as_bytes()))
}

fn b64url(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(bytes.len() * 4 / 3 + 2);
    for chunk in bytes.chunks(3) {
        let n = (chunk[0] as u32) << 16
            | (*chunk.get(1).unwrap_or(&0) as u32) << 8
            | *chunk.get(2).unwrap_or(&0) as u32;
        for i in 0..=chunk.len() {
            out.push(A[(n >> (18 - 6 * i)) as usize & 63] as char);
        }
    }
    out
}

/// Endpoints Agenium talks to must be HTTPS, or HTTP on this machine.
fn allowed(u: &Url) -> bool {
    u.scheme() == "https"
        || (u.scheme() == "http"
            && matches!(
                u.host_str(),
                Some("127.0.0.1" | "localhost" | "[::1]" | "::1")
            ))
}

fn parse_allowed(s: &str, what: &str) -> Result<Url> {
    let u = Url::parse(s).map_err(|e| auth(format!("{what} {s:?} is not a URL: {e}")))?;
    if !allowed(&u) {
        return Err(auth(format!(
            "{what} {s} must use HTTPS (HTTP only on this machine)"
        )));
    }
    Ok(u)
}

/// `origin + /.well-known/<name> + path` (RFC 8615 with the path appended, as RFC 8414 and 9728 do).
fn well_known(base: &Url, name: &str, with_path: bool) -> Option<Url> {
    let path = base.path().trim_end_matches('/');
    let mut u = base.clone();
    u.set_query(None);
    u.set_fragment(None);
    u.set_path(&if with_path && !path.is_empty() {
        format!("/.well-known/{name}{path}")
    } else {
        format!("/.well-known/{name}")
    });
    Some(u)
}

fn dedup(v: Vec<Url>) -> Vec<Url> {
    let mut out: Vec<Url> = Vec::new();
    for u in v {
        if !out.contains(&u) {
            out.push(u);
        }
    }
    out
}

/// Where the protected-resource metadata of `server` may be (path-specific first).
pub fn resource_metadata_urls(server: &Url) -> Vec<Url> {
    dedup(
        [
            well_known(server, "oauth-protected-resource", true),
            well_known(server, "oauth-protected-resource", false),
        ]
        .into_iter()
        .flatten()
        .collect(),
    )
}

/// Where the metadata of the authorization server `issuer` may be (MCP 2025-11-25 order).
pub fn authorization_metadata_urls(issuer: &Url) -> Vec<Url> {
    let mut v = vec![
        well_known(issuer, "oauth-authorization-server", true),
        well_known(issuer, "openid-configuration", true),
    ];
    if !issuer.path().trim_end_matches('/').is_empty() {
        let mut u = issuer.clone();
        u.set_path(&format!(
            "{}/.well-known/openid-configuration",
            issuer.path().trim_end_matches('/')
        ));
        v.push(Some(u));
    }
    dedup(v.into_iter().flatten().collect())
}

/// The endpoints and the parameters a login needs.
#[derive(Debug, Clone)]
pub struct Discovery {
    pub resource: String,
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub registration_endpoint: Option<String>,
    /// What the server asks for (its challenge's `scope`, else the resource's `scopes_supported`).
    pub scopes: Option<String>,
    /// The authorization server says it names itself in its answer (`iss`, RFC 9207): it must then be there.
    pub iss_in_answer: bool,
}

fn http() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(HTTP_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| auth(e.to_string()))
}

async fn get_json(c: &reqwest::Client, u: &Url) -> Option<Value> {
    let r = c
        .get(u.clone())
        .header("Accept", "application/json")
        .header("MCP-Protocol-Version", PROTOCOL_VERSION)
        .send()
        .await
        .ok()?;
    if !r.status().is_success() {
        return None;
    }
    r.json::<Value>().await.ok().filter(Value::is_object)
}

/// Whether the `resource` named by protected-resource metadata covers `server` (RFC 9728 §3.3): the same origin, and
/// the server's path or a parent of it on a segment boundary (`/mcp` covers `/mcp` and `/mcp/x`, not `/mcpx`). A
/// string prefix let `https://victim.com` cover `https://victim.com.evil.net/mcp`, so a token for the victim's
/// resource was asked for on the other server's behalf.
fn resource_covers(resource: &str, server: &Url) -> bool {
    let Ok(r) = Url::parse(resource) else {
        return false;
    };
    let (rp, sp) = (r.path().trim_end_matches('/'), server.path());
    r.origin() == server.origin()
        && (sp.trim_end_matches('/') == rp || sp.starts_with(&format!("{rp}/")))
}

/// Asks the server how to sign in: an unauthenticated `initialize` must come back 401 with a Bearer challenge.
pub async fn discover(server_url: &str) -> Result<Discovery> {
    let server = parse_allowed(server_url, "MCP server URL")?;
    let c = http()?;
    let probe = c
        .post(server.clone())
        .header("Accept", "application/json, text/event-stream")
        .header("Content-Type", "application/json")
        .body(
            json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
                "protocolVersion": PROTOCOL_VERSION, "capabilities": {},
                "clientInfo": {"name": "agenium", "version": env!("CARGO_PKG_VERSION")}}})
            .to_string(),
        )
        .send()
        .await
        .map_err(|e| auth(format!("cannot reach {server}: {e}")))?;
    if probe.status().is_success() {
        return Err(auth(format!("{server} does not ask for sign-in")));
    }
    if probe.status().as_u16() != 401 {
        return Err(auth(format!(
            "{server} answered HTTP {} instead of asking for sign-in (401)",
            probe.status().as_u16()
        )));
    }
    let challenge = probe
        .headers()
        .get_all("www-authenticate")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find_map(parse_challenge)
        .unwrap_or_default();

    // Protected-resource metadata: the challenge's pointer, else the well-known places.
    let mut candidates = Vec::new();
    if let Some(m) = &challenge.resource_metadata {
        candidates.push(parse_allowed(m, "resource metadata URL")?);
    }
    candidates.extend(resource_metadata_urls(&server));
    let mut prm = None;
    for u in &candidates {
        if let Some(v) = get_json(&c, u).await {
            prm = Some(v);
            break;
        }
    }
    let canonical = {
        let mut u = server.clone();
        u.set_fragment(None);
        u.to_string()
    };
    let (resource, issuer, prm_scopes) = match &prm {
        Some(p) => {
            let resource = p["resource"].as_str().unwrap_or(&canonical).to_owned();
            // The metadata must describe this server (RFC 9728 §3.3), not another one.
            if !resource_covers(&resource, &server) {
                return Err(auth(format!(
                    "the resource metadata names {resource}, not this server ({canonical})"
                )));
            }
            let issuer = p["authorization_servers"][0]
                .as_str()
                .ok_or_else(|| auth("the resource metadata names no authorization server"))?
                .to_owned();
            let scopes = p["scopes_supported"].as_array().map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(" ")
            });
            (resource, issuer, scopes.filter(|s| !s.is_empty()))
        }
        // Servers of revision 2025-03-26 have no resource metadata: the server's origin is the issuer.
        None => (
            canonical.clone(),
            server.origin().ascii_serialization(),
            None,
        ),
    };
    let issuer_url = parse_allowed(&issuer, "authorization server")?;
    let mut meta = None;
    for u in authorization_metadata_urls(&issuer_url) {
        if let Some(v) = get_json(&c, &u).await {
            meta = Some(v);
            break;
        }
    }
    // An empty `scope=""` is no scope (learned from pi).
    let scopes = challenge
        .scope
        .clone()
        .filter(|s| !s.trim().is_empty())
        .or(prm_scopes);
    let Some(m) = meta else {
        if prm.is_some() {
            return Err(auth(format!(
                "no metadata found for the authorization server {issuer}"
            )));
        }
        // 2025-03-26 fallback: default endpoints on the server's origin.
        let base = issuer.trim_end_matches('/');
        return Ok(Discovery {
            resource,
            authorization_endpoint: format!("{base}/authorize"),
            token_endpoint: format!("{base}/token"),
            registration_endpoint: Some(format!("{base}/register")),
            issuer,
            scopes,
            iss_in_answer: false,
        });
    };
    let methods: Vec<&str> = m["code_challenge_methods_supported"]
        .as_array()
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    if !methods.contains(&"S256") {
        return Err(auth(format!(
            "the authorization server {issuer} does not support PKCE S256, which MCP requires"
        )));
    }
    let endpoint = |k: &str| -> Result<String> {
        let s = m[k]
            .as_str()
            .ok_or_else(|| auth(format!("the authorization server metadata has no {k}")))?;
        parse_allowed(s, k).map(|u| u.to_string())
    };
    Ok(Discovery {
        resource,
        issuer: m["issuer"].as_str().unwrap_or(&issuer).to_owned(),
        authorization_endpoint: endpoint("authorization_endpoint")?,
        token_endpoint: endpoint("token_endpoint")?,
        registration_endpoint: endpoint("registration_endpoint").ok(),
        scopes,
        iss_in_answer: m["authorization_response_iss_parameter_supported"] == json!(true),
    })
}

/// A client registered at, or configured for, the authorization server.
#[derive(Debug, Clone)]
pub struct OAuthClient {
    pub client_id: String,
    pub client_secret: Option<String>,
}

/// Dynamic client registration (RFC 7591) as a public client of this redirect URI.
pub async fn register(d: &Discovery, redirect_uri: &str) -> Result<OAuthClient> {
    let endpoint = d.registration_endpoint.as_deref().ok_or_else(|| {
        auth(format!(
            "the authorization server {} does not register clients; configure a client id for this server \
             (agenium mcp add --oauth-client-id …)",
            d.issuer
        ))
    })?;
    let mut body = json!({
        "client_name": "Agenium",
        "redirect_uris": [redirect_uri],
        "grant_types": ["authorization_code", "refresh_token"],
        "response_types": ["code"],
        "token_endpoint_auth_method": "none",
    });
    if let Some(s) = &d.scopes {
        body["scope"] = json!(s);
    }
    let r = http()?
        .post(endpoint)
        .json(&body)
        .send()
        .await
        .map_err(|e| auth(format!("client registration failed: {e}")))?;
    let status = r.status();
    let v: Value = r.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        return Err(auth(format!(
            "client registration refused (HTTP {}): {}",
            status.as_u16(),
            error_text(&v)
        )));
    }
    Ok(OAuthClient {
        client_id: v["client_id"]
            .as_str()
            .ok_or_else(|| auth("the registration answer has no client_id"))?
            .to_owned(),
        client_secret: v["client_secret"].as_str().map(str::to_owned),
    })
}

fn error_text(v: &Value) -> String {
    match (v["error"].as_str(), v["error_description"].as_str()) {
        (Some(e), Some(d)) => format!("{e}: {d}"),
        (Some(e), None) => e.to_owned(),
        _ => "no details".into(),
    }
}

fn form(pairs: &[(&str, &str)]) -> String {
    let mut u = Url::parse("http://x/").expect("static URL");
    u.query_pairs_mut().extend_pairs(pairs);
    u.query().unwrap_or("").to_owned()
}

/// A token request (code exchange or refresh); the answer merged into `base`.
async fn token_request(base: &Tokens, pairs: &[(&str, &str)]) -> Result<Tokens> {
    let mut all: Vec<(&str, &str)> = pairs.to_vec();
    all.push(("client_id", &base.client_id));
    all.push(("resource", &base.resource));
    if let Some(s) = &base.client_secret {
        all.push(("client_secret", s));
    }
    let r = http()?
        .post(&base.token_endpoint)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("Accept", "application/json")
        .body(form(&all))
        .send()
        .await
        .map_err(|e| auth(format!("token request failed: {e}")))?;
    let status = r.status();
    let v: Value = r.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        return Err(auth(format!(
            "the authorization server refused the token request (HTTP {}): {}",
            status.as_u16(),
            error_text(&v)
        )));
    }
    let access = v["access_token"]
        .as_str()
        .ok_or_else(|| auth("the token answer has no access_token"))?;
    if !v["token_type"]
        .as_str()
        .is_some_and(|t| t.eq_ignore_ascii_case("bearer"))
    {
        return Err(auth("the token answer is not a Bearer token"));
    }
    let mut t = base.clone();
    t.access_token = access.to_owned();
    if let Some(r) = v["refresh_token"].as_str() {
        t.refresh_token = Some(r.to_owned());
    }
    t.expires_at = v["expires_in"].as_u64().map(|s| now() + s);
    if let Some(s) = v["scope"].as_str().filter(|s| !s.trim().is_empty()) {
        t.scope = Some(s.to_owned());
    }
    Ok(t)
}

/// Refreshes `t` (keeps the refresh token when the server does not rotate it).
pub async fn refresh(t: &Tokens) -> Result<Tokens> {
    let r = t
        .refresh_token
        .clone()
        .ok_or_else(|| auth("no refresh token"))?;
    token_request(t, &[("grant_type", "refresh_token"), ("refresh_token", &r)]).await
}

// ---------------------------------------------------------------- login

/// Settings of one login.
#[derive(Debug, Clone, Default)]
pub struct LoginOptions {
    /// A configured client; `None` = dynamic registration.
    pub client: Option<OAuthClient>,
    /// Scopes to ask for; empty = what the server asks for.
    pub scopes: Vec<String>,
    /// Fixed loopback port; 0 = any.
    pub callback_port: u16,
    pub timeout: Option<Duration>,
}

/// Signs the user in to `server_url`: `open` gets the authorization URL (Agenium opens the browser), the loopback
/// redirect brings the code back. The tokens are saved under `server` and returned.
pub async fn login(
    server: &str,
    server_url: &str,
    opts: &LoginOptions,
    store: &dyn TokenStore,
    open: &(dyn Fn(&str) + Send + Sync),
) -> Result<Tokens> {
    let d = discover(server_url).await?;
    let listener = TcpListener::bind(("127.0.0.1", opts.callback_port))
        .await
        .map_err(|e| auth(format!("cannot listen for the sign-in redirect: {e}")))?;
    let port = listener
        .local_addr()
        .map_err(|e| auth(e.to_string()))?
        .port();
    let redirect = format!("http://127.0.0.1:{port}/callback");
    let client = match &opts.client {
        Some(c) => c.clone(),
        None => register(&d, &redirect).await?,
    };
    let (verifier, challenge) = pkce();
    let state = b64url(&rand::random::<[u8; 16]>());
    let scope = if opts.scopes.is_empty() {
        d.scopes.clone()
    } else {
        Some(opts.scopes.join(" "))
    };
    let mut url = parse_allowed(&d.authorization_endpoint, "authorization endpoint")?;
    {
        let mut q = url.query_pairs_mut();
        q.append_pair("response_type", "code")
            .append_pair("client_id", &client.client_id)
            .append_pair("redirect_uri", &redirect)
            .append_pair("code_challenge", &challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair("state", &state)
            .append_pair("resource", &d.resource);
        if let Some(s) = &scope {
            q.append_pair("scope", s);
        }
    }
    open(url.as_str());
    let code = tokio::time::timeout(
        opts.timeout.unwrap_or(LOGIN_TIMEOUT),
        wait_for_code(&listener, &state, server, &d.issuer, d.iss_in_answer),
    )
    .await
    .map_err(|_| auth("sign-in was not finished in time"))??;
    let base = Tokens {
        server_url: server_url.to_owned(),
        resource: d.resource.clone(),
        issuer: d.issuer.clone(),
        token_endpoint: d.token_endpoint.clone(),
        client_id: client.client_id,
        client_secret: client.client_secret,
        access_token: String::new(),
        refresh_token: None,
        expires_at: None,
        scope,
    };
    let t = token_request(
        &base,
        &[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("redirect_uri", &redirect),
            ("code_verifier", &verifier),
        ],
    )
    .await?;
    store.save(server, &t).map_err(auth)?;
    Ok(t)
}

async fn respond(stream: &mut tokio::net::TcpStream, status: &str, body: &str) {
    let page = format!(
        "<!doctype html><meta charset=utf-8><title>Agenium</title>\
         <body style=\"font-family:sans-serif;margin:3em\"><p>{body}</p></body>"
    );
    let _ = stream
        .write_all(
            format!(
                "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\n\
                 Connection: close\r\n\r\n{page}",
                page.len()
            )
            .as_bytes(),
        )
        .await;
}

/// Waits for the browser's redirect to `/callback` and returns the code. The state must match, and the answer's
/// `iss`, when there is one (or when the server promised one), must name the authorization server Agenium asked
/// (RFC 9207: an answer from another server, mixed up by an attacker, is refused).
async fn wait_for_code(
    listener: &TcpListener,
    state: &str,
    server: &str,
    issuer: &str,
    iss_required: bool,
) -> Result<String> {
    loop {
        let (mut stream, _) = listener.accept().await.map_err(|e| auth(e.to_string()))?;
        let mut buf = vec![0u8; 16 * 1024];
        let mut n = 0;
        while n < buf.len() {
            let Ok(k) = stream.read(&mut buf[n..]).await else {
                break;
            };
            if k == 0 {
                break;
            }
            n += k;
            if buf[..n].windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        let head = String::from_utf8_lossy(&buf[..n]);
        let target = head
            .lines()
            .next()
            .and_then(|l| l.strip_prefix("GET "))
            .and_then(|l| l.split(' ').next())
            .unwrap_or("");
        let Ok(u) = Url::parse(&format!("http://127.0.0.1{target}")) else {
            respond(&mut stream, "400 Bad Request", "Bad request.").await;
            continue;
        };
        if u.path() != "/callback" {
            respond(&mut stream, "404 Not Found", "Not found.").await;
            continue;
        }
        let q: HashMap<String, String> = u.query_pairs().into_owned().collect();
        if q.get("state").map(String::as_str) != Some(state) {
            respond(
                &mut stream,
                "400 Bad Request",
                "This sign-in answer does not belong to the sign-in Agenium started.",
            )
            .await;
            return Err(auth("the sign-in answer had a wrong state (not ours)"));
        }
        let same = |a: &str| a.trim_end_matches('/') == issuer.trim_end_matches('/');
        match q.get("iss") {
            Some(iss) if !same(iss) => {
                respond(
                    &mut stream,
                    "400 Bad Request",
                    "This sign-in answer comes from another authorization server.",
                )
                .await;
                return Err(auth(format!(
                    "the sign-in answer came from {iss}, not from {issuer}"
                )));
            }
            None if iss_required => {
                respond(
                    &mut stream,
                    "400 Bad Request",
                    "The answer does not name its server.",
                )
                .await;
                return Err(auth(
                    "the sign-in answer did not name its authorization server (iss)",
                ));
            }
            _ => {}
        }
        if let Some(e) = q.get("error") {
            let d = q.get("error_description").cloned().unwrap_or_default();
            respond(
                &mut stream,
                "200 OK",
                "Sign-in was not completed. You can close this tab.",
            )
            .await;
            return Err(auth(format!("sign-in refused: {e} {d}").trim().to_owned()));
        }
        let Some(code) = q.get("code") else {
            respond(&mut stream, "400 Bad Request", "No code in the answer.").await;
            return Err(auth("the sign-in answer had no code"));
        };
        respond(
            &mut stream,
            "200 OK",
            &format!(
                "Agenium is signed in to the MCP server {}. You can close this tab.",
                server.replace(['<', '>', '&'], "")
            ),
        )
        .await;
        return Ok(code.clone());
    }
}

/// Opens `url` in the user's browser; `false` when no opener could be started.
pub fn open_browser(url: &str) -> bool {
    let mut cmd = if cfg!(windows) {
        let mut c = std::process::Command::new("rundll32");
        c.args(["url.dll,FileProtocolHandler", url]);
        c
    } else if cfg!(target_os = "macos") {
        let mut c = std::process::Command::new("open");
        c.arg(url);
        c
    } else {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(url);
        c
    };
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .is_ok()
}

// ---------------------------------------------------------------- use

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
    fn stored(&self) -> Option<Tokens> {
        self.store
            .load(&self.server)
            .filter(|t| t.server_url == self.url)
    }

    async fn renew(&self, t: &Tokens) -> Option<Tokens> {
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
