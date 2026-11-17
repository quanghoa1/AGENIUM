//! `fetch`: the allowlist, redirects, private addresses and the size cap.

mod common;

use std::sync::Arc;

use agenium_tools::*;
use common::*;
use serde_json::json;

#[tokio::test]
async fn fetch_needs_allowlist_and_marks_untrusted() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let (_t, mut c) = setup();
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (mut s, _) = l.accept().await.unwrap();
        let mut b = [0u8; 4096];
        let _ = s.read(&mut b).await;
        let body = "<html><head><script>steal()</script></head><body><h1>Docs</h1><p>Use &amp; enjoy</p></body></html>";
        let resp = format!("HTTP/1.1 200 OK\r\ncontent-type: text/html\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len());
        s.write_all(resp.as_bytes()).await.unwrap();
    });
    let url = format!("http://127.0.0.1:{port}/docs");
    assert_eq!(
        call(&c, "fetch", json!({"url": url}))
            .await
            .unwrap_err()
            .code,
        "domain_not_allowed"
    );
    assert_eq!(
        call(&c, "fetch", json!({"url": "file:///etc/passwd"}))
            .await
            .unwrap_err()
            .code,
        "invalid_url"
    );
    c.fetch_allow = vec!["127.0.0.1".into()];
    let out = call(&c, "fetch", json!({"url": url})).await.unwrap();
    assert!(out.untrusted);
    // ADR-142: the page in a fence that names its host and closes with its own id.
    assert!(
        out.text
            .starts_with("<untrusted_content source=\"127.0.0.1\" id=\"")
            && out.text.trim_end().ends_with("\">"),
        "{}",
        out.text
    );
    assert!(
        out.text.contains("Docs")
            && out.text.contains("Use & enjoy")
            && !out.text.contains("steal"),
        "{}",
        out.text
    );
    assert!(
        domain_allowed("api.crates.io", &["*.crates.io".into()])
            && !domain_allowed("evilcrates.io", &["*.crates.io".into()])
    );
}

/// A local HTTP server answering every connection with `reply`; returns its port.
async fn serve(reply: impl Fn() -> Vec<u8> + Send + Sync + 'static) -> u16 {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    let reply = Arc::new(reply);
    tokio::spawn(async move {
        while let Ok((mut s, _)) = l.accept().await {
            let reply = reply.clone();
            tokio::spawn(async move {
                let mut b = [0u8; 4096];
                let _ = s.read(&mut b).await;
                let _ = s.write_all(&reply()).await;
                // Keep the connection open: a body without a length ends only when it closes.
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            });
        }
    });
    port
}

fn ok_reply(body: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 200 OK\r\ncontent-type: text/plain\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

/// Review of 2026-10-27: the fetch client followed redirects, so the allowlist held for the first URL only, a host
/// let in by a wildcard could be a private address, and the whole body was read before the 2 MB cut.
#[tokio::test]
async fn fetch_checks_every_redirect_and_private_addresses_and_stops_at_the_cap() {
    let (_t, mut c) = setup();
    let secret = serve(|| ok_reply("internal admin page")).await;
    let redirect = serve(move || {
        format!("HTTP/1.1 302 Found\r\nlocation: http://127.0.0.1:{secret}/admin\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
            .into_bytes()
    })
    .await;
    // `localhost` is allowlisted by name; the redirect goes to 127.0.0.1, which is not.
    c.fetch_allow = vec!["localhost".into()];
    let e = call(
        &c,
        "fetch",
        json!({"url": format!("http://localhost:{redirect}/")}),
    )
    .await
    .unwrap_err();
    assert_eq!(e.code, "domain_not_allowed", "{e:?}");
    // Allowlisted by its exact name, the same redirect is followed.
    c.fetch_allow = vec!["localhost".into(), "127.0.0.1".into()];
    let out = call(
        &c,
        "fetch",
        json!({"url": format!("http://localhost:{redirect}/")}),
    )
    .await
    .unwrap();
    assert!(out.text.contains("internal admin page"), "{}", out.text);
    // A wildcard lets in a loopback address: refused.
    c.fetch_allow = vec!["*.0.0.1".into()];
    let e = call(
        &c,
        "fetch",
        json!({"url": format!("http://127.0.0.1:{secret}/")}),
    )
    .await
    .unwrap_err();
    assert_eq!(e.code, "address_not_allowed", "{e:?}");

    // A body without a length that never ends: read up to the cap, then the tool returns.
    let endless = serve(|| {
        let mut r =
            b"HTTP/1.1 200 OK\r\ncontent-type: text/plain\r\nconnection: close\r\n\r\n".to_vec();
        r.extend(std::iter::repeat_n(b'a', 3 * 1024 * 1024));
        r
    })
    .await;
    c.fetch_allow = vec!["127.0.0.1".into()];
    let out = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        call(
            &c,
            "fetch",
            json!({"url": format!("http://127.0.0.1:{endless}/")}),
        ),
    )
    .await
    .expect("the fetch stops at the cap instead of waiting for the end of the body")
    .unwrap();
    let full = std::fs::read_to_string(c.root.join(out.full.unwrap())).unwrap();
    let body = full.matches('a').count();
    assert!(
        (2 * 1024 * 1024..=2 * 1024 * 1024 + 64).contains(&body),
        "{body}"
    );

    use std::net::IpAddr;
    for ip in [
        "127.0.0.1",
        "10.1.2.3",
        "172.16.0.1",
        "192.168.1.1",
        "169.254.169.254",
        "100.64.0.1",
        "0.0.0.0",
        "::1",
        "::",
        "fe80::1",
        "fd00::1",
        "::ffff:127.0.0.1",
        "::ffff:169.254.169.254",
        "64:ff9b::a9fe:a9fe",
    ] {
        assert!(!public_ip(ip.parse::<IpAddr>().unwrap()), "{ip}");
    }
    for ip in ["93.184.215.14", "2606:4700::1111", "::ffff:8.8.8.8"] {
        assert!(public_ip(ip.parse::<IpAddr>().unwrap()), "{ip}");
    }
}

/// ADR-163: open to the public internet (the run's commands reach it anyway), still never the private network.
#[tokio::test]
async fn an_open_fetch_still_refuses_the_private_network() {
    let (_t, mut c) = setup();
    let port = serve(|| ok_reply("internal admin page")).await;
    c.fetch_open = true;
    for host in ["127.0.0.1", "localhost"] {
        let e = call(
            &c,
            "fetch",
            json!({"url": format!("http://{host}:{port}/")}),
        )
        .await
        .unwrap_err();
        assert_eq!(e.code, "address_not_allowed", "{host}: {e:?}");
    }
    // Allowed by its exact name, as before.
    c.fetch_allow = vec!["127.0.0.1".into()];
    let out = call(
        &c,
        "fetch",
        json!({"url": format!("http://127.0.0.1:{port}/")}),
    )
    .await
    .unwrap();
    assert!(out.text.contains("internal admin page"), "{}", out.text);
}

/// ADR-163: `web_search` sends the key to the API it belongs to and reads its results as hits to fetch and cite.
#[tokio::test]
async fn web_search_asks_the_api_of_its_key() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let (_t, c) = setup();
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    let seen = tokio::spawn(async move {
        let (mut s, _) = l.accept().await.unwrap();
        let mut req = Vec::new();
        let mut b = [0u8; 4096];
        // Headers and the small JSON body arrive together or in two reads.
        while !String::from_utf8_lossy(&req).contains("\"query\"") {
            let n = s.read(&mut b).await.unwrap();
            if n == 0 {
                break;
            }
            req.extend_from_slice(&b[..n]);
        }
        let body = r#"{"results":[{"title":"Agenium docs","url":"https://example.org/docs","content":"How to install","published_date":"2026-11-01"}]}"#;
        let resp = format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len());
        s.write_all(resp.as_bytes()).await.unwrap();
        String::from_utf8_lossy(&req).into_owned()
    });
    let tool = WebSearchTool::with(vec![Engine::at(
        "tvly-test-key",
        &format!("http://127.0.0.1:{port}"),
    )]);
    let out = tool
        .call(&c, json!({"query": "agenium install", "recent": "month"}))
        .await
        .unwrap();
    let req = seen.await.unwrap().to_ascii_lowercase();
    assert!(req.starts_with("post /search"), "{req}");
    assert!(req.contains("authorization: bearer tvly-test-key"), "{req}");
    assert!(req.contains("\"time_range\":\"month\""), "{req}");
    assert!(out.untrusted);
    assert!(
        out.text
            .contains("1. Agenium docs\n   https://example.org/docs\n   date: 2026-11-01"),
        "{}",
        out.text
    );
    let e = tool
        .call(&c, json!({"query": "x", "recent": "decade"}))
        .await
        .unwrap_err();
    assert_eq!(e.code, "invalid_arguments");
}

/// ADR-163: no key, Exa's free search: `initialize` gives a session, `tools/call` of `web_search_exa` carries it and
/// no key; the event-stream answer reads as hits.
#[tokio::test]
async fn web_search_without_a_key_goes_to_exa() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let (_t, c) = setup();
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    let seen = tokio::spawn(async move {
        let mut requests = Vec::new();
        for answer in [
            r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-03-26","capabilities":{}}}"#,
            r#"{"jsonrpc":"2.0","id":2,"result":{"content":[{"type":"text","text":"Title: Docs\nURL: https://example.org/docs\nPublished: 2026-11-01T00:00:00Z\nHighlights:\nHow to install."}]}}"#,
        ] {
            let (mut s, _) = l.accept().await.unwrap();
            let mut req = Vec::new();
            let mut b = [0u8; 8192];
            while !String::from_utf8_lossy(&req).contains("\"method\"") {
                let n = s.read(&mut b).await.unwrap();
                if n == 0 {
                    break;
                }
                req.extend_from_slice(&b[..n]);
            }
            let body = format!("event: message\ndata: {answer}\n\n");
            let resp = format!("HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nmcp-session-id: s-42\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len());
            s.write_all(resp.as_bytes()).await.unwrap();
            requests.push(String::from_utf8_lossy(&req).to_ascii_lowercase());
        }
        requests
    });
    let tool = WebSearchTool::with(vec![Engine::at(
        "",
        &format!("http://127.0.0.1:{port}/mcp"),
    )]);
    let out = tool
        .call(&c, json!({"query": "agenium install"}))
        .await
        .unwrap();
    let req = seen.await.unwrap();
    assert!(req[0].contains("\"method\":\"initialize\""), "{}", req[0]);
    assert!(
        req[1].contains("\"name\":\"web_search_exa\"") && req[1].contains("mcp-session-id: s-42"),
        "{}",
        req[1]
    );
    assert!(!req[1].contains("x-api-key"), "no key is sent: {}", req[1]);
    assert!(
        out.text.contains("(Exa)")
            && out.text.contains(
                "1. Docs\n   https://example.org/docs\n   date: 2026-11-01\n   How to install."
            ),
        "{}",
        out.text
    );
}

/// ADR-163: an API out of its limit hands the search to the next, says so, and rests: the next search skips it.
#[tokio::test]
async fn web_search_goes_to_the_next_api_when_one_is_out_of_its_limit() {
    let (_t, c) = setup();
    let tavily = serve(|| {
        let body = r#"{"detail":{"error":"This request exceeds your plan's set usage limit."}}"#;
        format!("HTTP/1.1 432 Plan Limit\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len()).into_bytes()
    })
    .await;
    let brave = serve(|| {
        let body = r#"{"web":{"results":[{"title":"Docs","url":"https://example.org/docs","description":"How to install"}]}}"#;
        format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len()).into_bytes()
    })
    .await;
    let tool = WebSearchTool::with(vec![
        Engine::at("tvly-limited", &format!("http://127.0.0.1:{tavily}")),
        Engine::at("BSA-next", &format!("http://127.0.0.1:{brave}")),
    ]);
    for round in ["first", "second"] {
        let out = tool
            .call(&c, json!({"query": "agenium install"}))
            .await
            .unwrap();
        assert!(
            out.text.contains("(Brave Search)") && out.text.contains("1. Docs"),
            "{round}: {}",
            out.text
        );
        let note = if round == "first" {
            "Tavily answered 432"
        } else {
            "Tavily is out of its limit for now"
        };
        assert!(out.text.contains(note), "{round}: {}", out.text);
    }
    let keyed = WebSearchTool::new(&["BSA-b".into(), "tvly-t".into()]);
    assert_eq!(
        keyed.order(),
        [SearchApi::Tavily, SearchApi::Brave, SearchApi::Exa],
        "the keys first (Tavily, then Brave), Exa's free search last"
    );
    assert!(keyed.free(2) && !keyed.free(0));
    let all = WebSearchTool::new(&["BSA-b".into(), "exa-key-123".into(), "tvly-t".into()]);
    assert_eq!(
        all.order(),
        [
            SearchApi::Exa,
            SearchApi::Tavily,
            SearchApi::Brave,
            SearchApi::Exa
        ],
        "three keys: Exa's, Tavily's, Brave's, then Exa's free search"
    );
    assert!(!all.free(0) && all.free(3));
}
