//! An MCP server that asks for OAuth, for agenium-mcp's tests and for trying `agenium mcp login` by hand (not
//! shipped to users). One process is both the authorization server and the MCP server (Streamable HTTP, JSON
//! answers): resource metadata (RFC 9728), authorization server metadata (RFC 8414), dynamic client registration
//! (RFC 7591), the authorization code flow with PKCE S256 only, tokens bound to the `/mcp` resource (RFC 8707),
//! access tokens that expire, refresh tokens that rotate. Tools: `whoami` (which client and which token answered)
//! and `echo`.
//!
//! Usage: agenium-mcp-oauth-test-server [--port N] [--token-ttl SECONDS] [--auto-approve]
//! Prints `listening http://127.0.0.1:<port>/mcp` first. Without `--auto-approve` the authorization page asks the
//! person in the browser to Approve or Deny. The client id `agenium-test-client` is pre-registered (any loopback
//! redirect).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use agenium_mcp::oauth::s256;
use reqwest::Url;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const PREREGISTERED: &str = "agenium-test-client";
const SCOPE: &str = "mcp:tools";

struct Grant {
    client_id: String,
    redirect_uri: String,
    challenge: String,
    resource: String,
    scope: String,
}

struct Access {
    client_id: String,
    resource: String,
    expires: Instant,
    serial: u64,
}

struct Refresh {
    client_id: String,
    resource: String,
}

#[derive(Default)]
struct State {
    clients: HashMap<String, Vec<String>>,
    pending: HashMap<String, (Grant, String)>,
    codes: HashMap<String, Grant>,
    access: HashMap<String, Access>,
    refresh: HashMap<String, Refresh>,
    issued: u64,
    refreshes: u64,
}

struct Server {
    base: String,
    ttl: Duration,
    auto: bool,
    state: Mutex<State>,
}

fn random() -> String {
    rand::random::<[u8; 16]>()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn loopback(uri: &str) -> bool {
    Url::parse(uri).is_ok_and(|u| {
        u.scheme() == "http" && matches!(u.host_str(), Some("127.0.0.1" | "localhost"))
    })
}

struct Request {
    method: String,
    path: String,
    query: HashMap<String, String>,
    headers: HashMap<String, String>,
    body: String,
}

fn pairs(s: &str) -> HashMap<String, String> {
    Url::parse(&format!("http://x/?{s}"))
        .map(|u| u.query_pairs().into_owned().collect())
        .unwrap_or_default()
}

async fn read_request(stream: &mut TcpStream) -> Option<Request> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    let head_end = loop {
        let n = stream.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
        if buf.len() > 64 * 1024 {
            return None;
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let mut lines = head.lines();
    let mut first = lines.next()?.split(' ');
    let method = first.next()?.to_owned();
    let target = first.next()?.to_owned();
    let headers: HashMap<String, String> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_owned()))
        .collect();
    let len: usize = headers
        .get("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let mut body = buf[head_end..].to_vec();
    while body.len() < len {
        let n = stream.read(&mut chunk).await.ok()?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..n]);
    }
    let (path, q) = target.split_once('?').unwrap_or((&target, ""));
    Some(Request {
        method,
        path: path.to_owned(),
        query: pairs(q),
        headers,
        body: String::from_utf8_lossy(&body).to_string(),
    })
}

struct Response {
    status: &'static str,
    headers: Vec<(String, String)>,
    body: String,
    ctype: &'static str,
}

fn json_response(status: &'static str, v: Value) -> Response {
    Response {
        status,
        headers: vec![],
        body: v.to_string(),
        ctype: "application/json",
    }
}

fn page(status: &'static str, html: &str) -> Response {
    Response {
        status,
        headers: vec![],
        body: format!(
            "<!doctype html><meta charset=utf-8><title>Test sign-in</title>\
             <body style=\"font-family:sans-serif;margin:3em\">{html}</body>"
        ),
        ctype: "text/html; charset=utf-8",
    }
}

fn redirect(to: &str) -> Response {
    Response {
        status: "302 Found",
        headers: vec![("Location".into(), to.to_owned())],
        body: String::new(),
        ctype: "text/plain",
    }
}

fn with_query(base: &str, q: &[(&str, &str)]) -> String {
    let mut u = Url::parse(base).expect("validated redirect");
    u.query_pairs_mut().extend_pairs(q);
    u.to_string()
}

impl Server {
    fn resource(&self) -> String {
        format!("{}/mcp", self.base)
    }

    fn unauthorized(&self, error: Option<&str>) -> Response {
        let mut challenge = format!(
            "Bearer resource_metadata=\"{}/.well-known/oauth-protected-resource/mcp\", scope=\"{SCOPE}\"",
            self.base
        );
        if let Some(e) = error {
            challenge.push_str(&format!(", error=\"{e}\""));
        }
        let mut r = json_response("401 Unauthorized", json!({"error": "unauthorized"}));
        r.headers.push(("WWW-Authenticate".into(), challenge));
        r
    }

    fn handle(&self, r: Request) -> Response {
        match (r.method.as_str(), r.path.as_str()) {
            ("GET", "/.well-known/oauth-protected-resource/mcp")
            | ("GET", "/.well-known/oauth-protected-resource") => json_response(
                "200 OK",
                json!({
                    "resource": self.resource(),
                    "authorization_servers": [self.base],
                    "scopes_supported": [SCOPE],
                    "bearer_methods_supported": ["header"],
                }),
            ),
            ("GET", "/.well-known/oauth-authorization-server") => json_response(
                "200 OK",
                json!({
                    "issuer": self.base,
                    "authorization_endpoint": format!("{}/authorize", self.base),
                    "token_endpoint": format!("{}/token", self.base),
                    "registration_endpoint": format!("{}/register", self.base),
                    "response_types_supported": ["code"],
                    "grant_types_supported": ["authorization_code", "refresh_token"],
                    "code_challenge_methods_supported": ["S256"],
                    "token_endpoint_auth_methods_supported": ["none"],
                    "scopes_supported": [SCOPE],
                }),
            ),
            ("POST", "/register") => self.register(&r.body),
            ("GET", "/authorize") => self.authorize(&r.query),
            ("GET", "/authorize/decide") => self.decide(&r.query),
            ("POST", "/token") => self.token(&pairs(&r.body)),
            ("POST", "/mcp") => self.mcp(&r),
            // Test hooks: the server forgets its access tokens (as after a restart), or every token.
            ("POST", "/test/drop-access") => {
                self.state.lock().unwrap().access.clear();
                json_response("200 OK", json!({}))
            }
            ("POST", "/test/revoke-all") => {
                let mut s = self.state.lock().unwrap();
                s.access.clear();
                s.refresh.clear();
                json_response("200 OK", json!({}))
            }
            ("GET", "/stats") => {
                let s = self.state.lock().unwrap();
                json_response(
                    "200 OK",
                    json!({"issued": s.issued, "refreshes": s.refreshes, "clients": s.clients.len()}),
                )
            }
            _ => json_response("404 Not Found", json!({"error": "not_found"})),
        }
    }

    fn register(&self, body: &str) -> Response {
        let v: Value = serde_json::from_str(body).unwrap_or(Value::Null);
        let uris: Vec<String> = v["redirect_uris"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|u| u.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        if uris.is_empty() || !uris.iter().all(|u| loopback(u)) {
            return json_response(
                "400 Bad Request",
                json!({"error": "invalid_redirect_uri", "error_description": "loopback redirect URIs only"}),
            );
        }
        let id = format!("dyn-{}", random());
        self.state
            .lock()
            .unwrap()
            .clients
            .insert(id.clone(), uris.clone());
        json_response(
            "201 Created",
            json!({"client_id": id, "redirect_uris": uris, "token_endpoint_auth_method": "none",
                   "grant_types": ["authorization_code", "refresh_token"]}),
        )
    }

    fn redirect_allowed(&self, client: &str, uri: &str) -> bool {
        if client == PREREGISTERED {
            return loopback(uri);
        }
        self.state
            .lock()
            .unwrap()
            .clients
            .get(client)
            .is_some_and(|u| u.iter().any(|x| x == uri))
    }

    fn authorize(&self, q: &HashMap<String, String>) -> Response {
        let get = |k: &str| q.get(k).cloned().unwrap_or_default();
        let (client, uri) = (get("client_id"), get("redirect_uri"));
        // A bad client or redirect is never redirected to (RFC 6749 §4.1.2.1).
        if !self.redirect_allowed(&client, &uri) {
            return page("400 Bad Request", "<p>Unknown client or redirect URI.</p>");
        }
        let state = get("state");
        let fail = |e: &str| redirect(&with_query(&uri, &[("error", e), ("state", &state)]));
        if get("response_type") != "code" {
            return fail("unsupported_response_type");
        }
        if get("code_challenge").is_empty() || get("code_challenge_method") != "S256" {
            return fail("invalid_request");
        }
        if get("resource") != self.resource() {
            return fail("invalid_target");
        }
        let grant = Grant {
            client_id: client.clone(),
            redirect_uri: uri.clone(),
            challenge: get("code_challenge"),
            resource: get("resource"),
            scope: q.get("scope").cloned().unwrap_or_else(|| SCOPE.into()),
        };
        let id = random();
        self.state
            .lock()
            .unwrap()
            .pending
            .insert(id.clone(), (grant, state));
        if self.auto {
            return self.decide(&[("req".into(), id), ("approve".into(), "1".into())].into());
        }
        page(
            "200 OK",
            &format!(
                "<h2>Agenium OAuth test server</h2><p>Client <code>{}</code> asks for <code>{}</code> on \
                 <code>{}</code>.</p><p><a href=\"/authorize/decide?req={id}&approve=1\">Approve</a> &nbsp; \
                 <a href=\"/authorize/decide?req={id}&approve=0\">Deny</a></p>",
                html(&client),
                html(&q.get("scope").cloned().unwrap_or_else(|| SCOPE.into())),
                html(&self.resource())
            ),
        )
    }

    fn decide(&self, q: &HashMap<String, String>) -> Response {
        let Some((grant, state)) = self
            .state
            .lock()
            .unwrap()
            .pending
            .remove(q.get("req").map(String::as_str).unwrap_or(""))
        else {
            return page("400 Bad Request", "<p>Unknown or used request.</p>");
        };
        if q.get("approve").map(String::as_str) != Some("1") {
            return redirect(&with_query(
                &grant.redirect_uri,
                &[("error", "access_denied"), ("state", &state)],
            ));
        }
        let code = random();
        let to = with_query(&grant.redirect_uri, &[("code", &code), ("state", &state)]);
        self.state.lock().unwrap().codes.insert(code, grant);
        redirect(&to)
    }

    fn issue(&self, s: &mut State, client_id: &str, resource: &str, scope: &str) -> Value {
        s.issued += 1;
        let (at, rt) = (format!("at-{}", random()), format!("rt-{}", random()));
        s.access.insert(
            at.clone(),
            Access {
                client_id: client_id.to_owned(),
                resource: resource.to_owned(),
                expires: Instant::now() + self.ttl,
                serial: s.issued,
            },
        );
        s.refresh.insert(
            rt.clone(),
            Refresh {
                client_id: client_id.to_owned(),
                resource: resource.to_owned(),
            },
        );
        json!({"access_token": at, "token_type": "Bearer", "expires_in": self.ttl.as_secs(),
               "refresh_token": rt, "scope": scope})
    }

    fn token(&self, f: &HashMap<String, String>) -> Response {
        let get = |k: &str| f.get(k).cloned().unwrap_or_default();
        let bad = |e: &str| json_response("400 Bad Request", json!({"error": e}));
        let mut s = self.state.lock().unwrap();
        match get("grant_type").as_str() {
            "authorization_code" => {
                let Some(g) = s.codes.remove(&get("code")) else {
                    return bad("invalid_grant");
                };
                if g.client_id != get("client_id")
                    || g.redirect_uri != get("redirect_uri")
                    || s256(&get("code_verifier")) != g.challenge
                    || (f.contains_key("resource") && get("resource") != g.resource)
                {
                    return bad("invalid_grant");
                }
                let v = self.issue(&mut s, &g.client_id, &g.resource, &g.scope);
                json_response("200 OK", v)
            }
            "refresh_token" => {
                let Some(r) = s.refresh.remove(&get("refresh_token")) else {
                    return bad("invalid_grant");
                };
                if r.client_id != get("client_id")
                    || (f.contains_key("resource") && get("resource") != r.resource)
                {
                    return bad("invalid_grant");
                }
                s.refreshes += 1;
                let v = self.issue(&mut s, &r.client_id, &r.resource, SCOPE);
                json_response("200 OK", v)
            }
            _ => bad("unsupported_grant_type"),
        }
    }

    fn mcp(&self, r: &Request) -> Response {
        let token = r
            .headers
            .get("authorization")
            .and_then(|v| v.strip_prefix("Bearer "))
            .map(str::to_owned);
        let Some(token) = token else {
            return self.unauthorized(None);
        };
        let who = {
            let s = self.state.lock().unwrap();
            match s.access.get(&token) {
                Some(a) if a.expires > Instant::now() && a.resource == self.resource() => {
                    format!("client {}, token #{}", a.client_id, a.serial)
                }
                _ => return self.unauthorized(Some("invalid_token")),
            }
        };
        let msg: Value = serde_json::from_str(&r.body).unwrap_or(Value::Null);
        let id = msg["id"].clone();
        if id.is_null() {
            return Response {
                status: "202 Accepted",
                headers: vec![],
                body: String::new(),
                ctype: "text/plain",
            };
        }
        let result = match msg["method"].as_str().unwrap_or("") {
            "initialize" => json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {"tools": {"listChanged": false}},
                "serverInfo": {"name": "agenium-oauth-test-server", "version": "1"}
            }),
            "ping" => json!({}),
            "tools/list" => json!({"tools": [
                {"name": "whoami", "description": "Which client and which access token made this call.",
                 "inputSchema": {"type": "object"}, "annotations": {"readOnlyHint": true}},
                {"name": "echo", "description": "Echo text.",
                 "inputSchema": {"type": "object", "properties": {"text": {"type": "string"}}, "required": ["text"]},
                 "annotations": {"readOnlyHint": true}}
            ]}),
            "tools/call" => {
                let text = match msg["params"]["name"].as_str() {
                    Some("whoami") => who,
                    Some("echo") => msg["params"]["arguments"]["text"]
                        .as_str()
                        .unwrap_or("")
                        .to_owned(),
                    _ => {
                        return json_response(
                            "200 OK",
                            json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32602, "message": "unknown tool"}}),
                        )
                    }
                };
                json!({"content": [{"type": "text", "text": text}], "isError": false})
            }
            _ => {
                return json_response(
                    "200 OK",
                    json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "method not found"}}),
                )
            }
        };
        json_response(
            "200 OK",
            json!({"jsonrpc": "2.0", "id": id, "result": result}),
        )
    }
}

fn html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

async fn serve(server: Arc<Server>, mut stream: TcpStream) {
    let Some(req) = read_request(&mut stream).await else {
        return;
    };
    let r = server.handle(req);
    let mut out = format!(
        "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n",
        r.status,
        r.ctype,
        r.body.len()
    );
    for (k, v) in &r.headers {
        out.push_str(&format!("{k}: {v}\r\n"));
    }
    out.push_str("\r\n");
    out.push_str(&r.body);
    let _ = stream.write_all(out.as_bytes()).await;
    let _ = stream.shutdown().await;
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let arg = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .and_then(|v| v.parse::<u64>().ok())
    };
    let port = arg("--port").unwrap_or(0) as u16;
    let ttl = Duration::from_secs(arg("--token-ttl").unwrap_or(3600));
    let auto = args.iter().any(|a| a == "--auto-approve");
    let listener = TcpListener::bind(("127.0.0.1", port))
        .await
        .expect("bind 127.0.0.1");
    let addr = listener.local_addr().expect("local address");
    let base = format!("http://127.0.0.1:{}", addr.port());
    println!("listening {base}/mcp");
    let server = Arc::new(Server {
        base,
        ttl,
        auto,
        state: Mutex::new(State::default()),
    });
    while let Ok((stream, _)) = listener.accept().await {
        tokio::spawn(serve(server.clone(), stream));
    }
}
