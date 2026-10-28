//! Dynamic client registration and the token endpoint (code exchange, refresh).

use super::*;

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

pub(super) fn error_text(v: &Value) -> String {
    match (v["error"].as_str(), v["error_description"].as_str()) {
        (Some(e), Some(d)) => format!("{e}: {d}"),
        (Some(e), None) => e.to_owned(),
        _ => "no details".into(),
    }
}

pub(super) fn form(pairs: &[(&str, &str)]) -> String {
    let mut u = Url::parse("http://x/").expect("static URL");
    u.query_pairs_mut().extend_pairs(pairs);
    u.query().unwrap_or("").to_owned()
}

/// A token request (code exchange or refresh); the answer merged into `base`.
pub(super) async fn token_request(base: &Tokens, pairs: &[(&str, &str)]) -> Result<Tokens> {
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
