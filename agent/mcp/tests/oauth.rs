//! OAuth against the test server binary (agenium-mcp-oauth-test-server): discovery, dynamic registration, the
//! authorization code flow with PKCE through the loopback redirect, refresh before expiry, recovery after a 401,
//! and the user asked to sign in again when nothing works.

use std::io::BufRead;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;

use agenium_config::McpServerConfig;
use agenium_mcp::*;
use agenium_tools::{AllowAll, Tool, ToolCtx};
use serde_json::{json, Value};

struct TestServer {
    child: Child,
    url: String,
}

impl Drop for TestServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

impl TestServer {
    fn start(auto_approve: bool) -> TestServer {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_agenium-mcp-oauth-test-server"));
        if auto_approve {
            cmd.arg("--auto-approve");
        }
        let mut child = cmd.stdout(Stdio::piped()).spawn().unwrap();
        let mut line = String::new();
        std::io::BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        let url = line.trim().strip_prefix("listening ").unwrap().to_owned();
        TestServer { child, url }
    }

    fn base(&self) -> String {
        self.url.trim_end_matches("/mcp").to_owned()
    }

    async fn post(&self, path: &str) {
        reqwest::Client::new()
            .post(format!("{}{path}", self.base()))
            .send()
            .await
            .unwrap();
    }

    async fn stats(&self) -> Value {
        reqwest::get(format!("{}/stats", self.base()))
            .await
            .unwrap()
            .json()
            .await
            .unwrap()
    }
}

fn cfg(url: &str) -> McpServerConfig {
    McpServerConfig {
        id: "oauthy".into(),
        command: None,
        args: vec![],
        env: Default::default(),
        url: Some(url.to_owned()),
        headers: Default::default(),
        timeout_secs: 10,
        oauth: None,
        approval: None,
    }
}

fn binding() -> Binding {
    Binding {
        server: "oauthy".into(),
        credentials: Default::default(),
        keys: Default::default(),
        tools: None,
    }
}

fn no_secrets() -> SecretFn {
    Arc::new(|_: &str| None)
}

/// The user's browser: follows the authorization URL to the loopback redirect. With `approve: None` the server
/// approves by itself; otherwise the browser clicks Approve or Deny on the server's page.
fn browser(approve: Option<bool>) -> impl Fn(&str) + Send + Sync {
    move |url: &str| {
        let url = url.to_owned();
        tokio::spawn(async move {
            let c = reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap();
            let r = c.get(&url).send().await.unwrap();
            let location = match approve {
                None => r.headers()["location"].to_str().unwrap().to_owned(),
                Some(yes) => {
                    let page = r.text().await.unwrap();
                    let req = page
                        .split("req=")
                        .nth(1)
                        .unwrap()
                        .split('&')
                        .next()
                        .unwrap();
                    let base = url.split("/authorize").next().unwrap();
                    let d = c
                        .get(format!(
                            "{base}/authorize/decide?req={req}&approve={}",
                            u8::from(yes)
                        ))
                        .send()
                        .await
                        .unwrap();
                    d.headers()["location"].to_str().unwrap().to_owned()
                }
            };
            let _ = c.get(&location).send().await;
        });
    }
}

fn ctx(dir: &std::path::Path) -> ToolCtx {
    ToolCtx::new(dir, "run-1", Arc::new(AllowAll))
}

async fn whoami(hub: &McpHub, dir: &std::path::Path, url: &str) -> Result<String, String> {
    let tools = hub.tools_for("dev", &[binding()], &[cfg(url)]).await;
    if let Some(p) = tools.problems.first() {
        return Err(p.clone());
    }
    let t = tools
        .tools
        .iter()
        .find(|t| t.name() == "mcp__oauthy__whoami")
        .unwrap();
    // The answer inside its untrusted-content fence (ADR-142).
    t.call(&ctx(dir), json!({}))
        .await
        .map(|o| {
            let start = o.text.find('\n').map_or(0, |i| i + 1);
            let end = o
                .text
                .rfind("\n</untrusted_content")
                .unwrap_or(o.text.len());
            o.text[start..end].to_owned()
        })
        .map_err(|e| format!("{}: {}", e.code, e.message))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sign_in_use_refresh_and_sign_in_again() {
    let server = TestServer::start(true);
    let tmp = tempfile::tempdir().unwrap();
    let store = Arc::new(MemoryTokens::default());
    let hub = McpHub::new(no_secrets(), tmp.path().join("mcp")).with_tokens(store.clone());
    let c = cfg(&server.url);

    // Before signing in: the server's 401 becomes a sign-in request naming the command.
    let e = hub.test(&c, &binding()).await.unwrap_err();
    assert!(
        matches!(&e, McpError::AuthRequired(m) if m.contains("agenium mcp login oauthy")),
        "{e}"
    );
    assert!(!hub.signed_in(&c));

    // Sign in: discovery, dynamic registration, PKCE, loopback redirect, code exchange.
    let t = hub.login(&c, &[], &browser(None)).await.unwrap();
    assert!(t.client_id.starts_with("dyn-"));
    assert_eq!(t.resource, server.url);
    assert_eq!(t.scope.as_deref(), Some("mcp:tools"));
    assert!(t.refresh_token.is_some() && t.expires_at.is_some());
    assert!(
        !format!("{t:?}").contains(&t.access_token),
        "Debug shows a token"
    );
    assert!(hub.signed_in(&c));
    let (_, _, tools) = hub.test(&c, &binding()).await.unwrap();
    assert_eq!(
        tools.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
        ["whoami", "echo"]
    );
    let who = whoami(&hub, tmp.path(), &server.url).await.unwrap();
    assert!(who.ends_with("token #1"), "{who}");

    // The server forgets its access tokens: the 401 is answered by one refresh, and the call goes through.
    server.post("/test/drop-access").await;
    let who = whoami(&hub, tmp.path(), &server.url).await.unwrap();
    assert!(who.ends_with("token #2"), "{who}");
    assert_eq!(server.stats().await["refreshes"], 1);
    let saved = store.load("oauthy").unwrap();
    assert_ne!(
        saved.refresh_token, t.refresh_token,
        "rotated refresh token kept"
    );

    // An access token about to expire is refreshed before it is sent.
    let mut soon = store.load("oauthy").unwrap();
    soon.expires_at = Some(0);
    store.save("oauthy", &soon).unwrap();
    let hub2 = McpHub::new(no_secrets(), tmp.path().join("mcp2")).with_tokens(store.clone());
    let who = whoami(&hub2, tmp.path(), &server.url).await.unwrap();
    assert!(who.ends_with("token #3"), "{who}");
    assert_eq!(server.stats().await["refreshes"], 2);

    // Every token taken back: the tool tells the agent that only the user can sign in.
    server.post("/test/revoke-all").await;
    let e = whoami(&hub2, tmp.path(), &server.url).await.unwrap_err();
    assert!(e.starts_with("mcp_needs_sign_in"), "{e}");

    // A new login is picked up by the connections that exist.
    hub2.login(&c, &[], &browser(None)).await.unwrap();
    let who = whoami(&hub2, tmp.path(), &server.url).await.unwrap();
    assert!(who.ends_with("token #4"), "{who}");

    // Tokens belong to the URL they were issued for.
    let moved = cfg(&format!("{}/", server.url));
    assert!(!hub2.signed_in(&moved));
    hub2.logout("oauthy").await.unwrap();
    assert!(store.load("oauthy").is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_user_approves_or_denies_on_the_page() {
    let server = TestServer::start(false);
    let tmp = tempfile::tempdir().unwrap();
    let store = Arc::new(MemoryTokens::default());
    let hub = McpHub::new(no_secrets(), tmp.path().join("mcp")).with_tokens(store.clone());
    let c = cfg(&server.url);

    let e = hub.login(&c, &[], &browser(Some(false))).await.unwrap_err();
    assert!(e.to_string().contains("access_denied"), "{e}");
    assert!(store.load("oauthy").is_none());

    // A pre-registered client: no registration, the configured id is used.
    let mut pre = c.clone();
    pre.oauth = Some(agenium_config::McpOAuthConfig {
        client_id: Some("agenium-test-client".into()),
        ..Default::default()
    });
    let t = hub.login(&pre, &[], &browser(Some(true))).await.unwrap();
    assert_eq!(t.client_id, "agenium-test-client");
    assert_eq!(
        server.stats().await["clients"],
        1,
        "only the denied login registered"
    );
    let who = whoami(&hub, tmp.path(), &server.url).await.unwrap();
    assert!(who.starts_with("client agenium-test-client"), "{who}");
}

#[tokio::test]
async fn a_server_without_sign_in_is_said_so() {
    let tmp = tempfile::tempdir().unwrap();
    let hub = McpHub::new(no_secrets(), tmp.path().join("mcp"))
        .with_tokens(Arc::new(MemoryTokens::default()));
    // Nothing listens there: the login fails before any browser is opened.
    let e = hub
        .login(&cfg("http://127.0.0.1:9/mcp"), &[], &|_: &str| {
            panic!("no browser for an unreachable server")
        })
        .await
        .unwrap_err();
    assert!(e.to_string().contains("cannot reach"), "{e}");
    // Plain HTTP to another machine is refused.
    let e = hub
        .login(&cfg("http://example.com/mcp"), &[], &|_: &str| panic!())
        .await
        .unwrap_err();
    assert!(e.to_string().contains("HTTPS"), "{e}");
}

/// RFC 9207 (learned from pi's OAuth fixes): an answer that names another authorization server is refused.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_answer_from_another_authorization_server_is_refused() {
    let server = TestServer::start(true);
    let tmp = tempfile::tempdir().unwrap();
    let store = Arc::new(MemoryTokens::default());
    let hub = McpHub::new(no_secrets(), tmp.path().join("mcp")).with_tokens(store.clone());
    let tampering = |url: &str| {
        let url = url.to_owned();
        tokio::spawn(async move {
            let c = reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap();
            let r = c.get(&url).send().await.unwrap();
            let location = r.headers()["location"].to_str().unwrap().to_owned();
            assert!(
                location.contains("iss="),
                "the test server names itself: {location}"
            );
            let (head, _) = location.split_once("&iss=").unwrap();
            let _ = c
                .get(format!("{head}&iss=https%3A%2F%2Fevil.example"))
                .send()
                .await;
        });
    };
    let e = hub
        .login(&cfg(&server.url), &[], &tampering)
        .await
        .unwrap_err();
    assert!(e.to_string().contains("evil.example"), "{e}");
    assert!(store.load("oauthy").is_none());
}
