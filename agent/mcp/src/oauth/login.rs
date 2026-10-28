//! The interactive sign-in: the browser, the loopback redirect, the code.

use super::*;

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

pub(super) async fn respond(stream: &mut tokio::net::TcpStream, status: &str, body: &str) {
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
pub(super) async fn wait_for_code(
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
