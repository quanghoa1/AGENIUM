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
