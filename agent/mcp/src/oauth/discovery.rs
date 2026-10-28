//! Finding the authorization server: the `WWW-Authenticate` challenge, the protected-resource and authorization-server
//! metadata, PKCE, and which URLs may be used at all.

use super::*;

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

pub(super) fn b64url(bytes: &[u8]) -> String {
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
pub(super) fn allowed(u: &Url) -> bool {
    u.scheme() == "https"
        || (u.scheme() == "http"
            && matches!(
                u.host_str(),
                Some("127.0.0.1" | "localhost" | "[::1]" | "::1")
            ))
}

pub(super) fn parse_allowed(s: &str, what: &str) -> Result<Url> {
    let u = Url::parse(s).map_err(|e| auth(format!("{what} {s:?} is not a URL: {e}")))?;
    if !allowed(&u) {
        return Err(auth(format!(
            "{what} {s} must use HTTPS (HTTP only on this machine)"
        )));
    }
    Ok(u)
}

/// `origin + /.well-known/<name> + path` (RFC 8615 with the path appended, as RFC 8414 and 9728 do).
pub(super) fn well_known(base: &Url, name: &str, with_path: bool) -> Option<Url> {
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

pub(super) fn dedup(v: Vec<Url>) -> Vec<Url> {
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

pub(super) fn http() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(HTTP_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| auth(e.to_string()))
}

pub(super) async fn get_json(c: &reqwest::Client, u: &Url) -> Option<Value> {
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
pub(super) fn resource_covers(resource: &str, server: &Url) -> bool {
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
