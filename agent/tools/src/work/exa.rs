//! `web_search` through Exa's hosted MCP server (ADR-163): free without a key (rate-limited), more with an Exa key.
//! Two JSON-RPC calls over streamable HTTP: `initialize` (its session id), then `tools/call` of `web_search_exa`. The
//! answer is one text of results, each `Title:`, `URL:`, `Published:`, ... `Highlights:` and the page's text.

use super::web_search::Hit;
use super::*;

/// Exa's hosted MCP endpoint.
pub const EXA_MCP: &str = "https://mcp.exa.ai/mcp";

/// The JSON-RPC message of a reply: the body itself, or the `data:` line of a server-sent event that carries it.
fn rpc_message(body: &str) -> Option<serde_json::Value> {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(body.trim()) {
        return Some(v);
    }
    body.lines()
        .filter_map(|l| l.strip_prefix("data:"))
        .filter_map(|d| serde_json::from_str::<serde_json::Value>(d.trim()).ok())
        .find(|v| v.get("result").is_some() || v.get("error").is_some())
}

/// The results in Exa's text, one per `Title:` line.
pub fn exa_hits(text: &str) -> Vec<Hit> {
    let mut hits: Vec<Hit> = Vec::new();
    let mut in_body = false;
    for line in text.lines() {
        if let Some(t) = line.strip_prefix("Title: ") {
            hits.push(Hit {
                title: t.trim().to_owned(),
                url: String::new(),
                extract: String::new(),
                date: None,
            });
            in_body = false;
            continue;
        }
        let Some(h) = hits.last_mut() else { continue };
        if in_body {
            h.extract.push_str(line.trim());
            h.extract.push(' ');
        } else if let Some(u) = line.strip_prefix("URL: ") {
            h.url = u.trim().to_owned();
        } else if let Some(d) = line.strip_prefix("Published: ") {
            let d = d.trim();
            if !d.is_empty() && d != "N/A" {
                h.date = Some(d.split('T').next().unwrap_or(d).to_owned());
            }
        } else if line.starts_with("Highlights:") || line.starts_with("Text:") {
            in_body = true;
        }
    }
    for h in &mut hits {
        h.extract = h.extract.split_whitespace().collect::<Vec<_>>().join(" ");
    }
    hits.retain(|h| !h.url.is_empty());
    hits
}

pub(super) async fn search(
    http: &reqwest::Client,
    endpoint: &str,
    key: Option<&str>,
    query: &str,
    objective: &str,
    n: u8,
) -> Result<Vec<Hit>, ToolError> {
    let failed = |e: reqwest::Error| ToolError::new("search_failed", format!("Exa: {e}"));
    let post = |body: serde_json::Value, session: Option<&str>| {
        let mut r = http
            .post(endpoint)
            .timeout(Duration::from_secs(45))
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .body(body.to_string());
        if let Some(k) = key {
            r = r.header("x-api-key", k);
        }
        if let Some(s) = session {
            r = r.header("mcp-session-id", s);
        }
        r.send()
    };
    let init = post(
        serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
            "protocolVersion": "2025-03-26", "capabilities": {},
            "clientInfo": {"name": "agenium", "version": env!("CARGO_PKG_VERSION")}}}),
        None,
    )
    .await
    .map_err(failed)?;
    let session = init
        .headers()
        .get("mcp-session-id")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let status = init.status();
    if !status.is_success() {
        return Err(refused(status, &init.text().await.unwrap_or_default()));
    }
    let resp = post(
        serde_json::json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {
            "name": "web_search_exa",
            "arguments": {"query": query, "objective": objective, "numResults": n}}}),
        session.as_deref(),
    )
    .await
    .map_err(failed)?;
    let status = resp.status();
    let body = resp.text().await.map_err(failed)?;
    if !status.is_success() {
        return Err(refused(status, &body));
    }
    let v = rpc_message(&body).ok_or_else(|| {
        ToolError::new(
            "search_failed",
            format!(
                "Exa answered: {}",
                body.chars().take(300).collect::<String>()
            ),
        )
    })?;
    let text = v["result"]["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| c["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let said = match v.get("error") {
        Some(e) => Some(e["message"].as_str().unwrap_or("error").to_owned()),
        None => (v["result"]["isError"] == true).then(|| text.chars().take(300).collect()),
    };
    if let Some(said) = said {
        // A free search out of its limit says so in words, not with a status.
        let l = said.to_ascii_lowercase();
        let code =
            if l.contains("rate limit") || l.contains("limit exceeded") || l.contains("quota") {
                "search_limited"
            } else {
                "search_failed"
            };
        return Err(ToolError::new(code, format!("Exa: {said}")));
    }
    Ok(exa_hits(&text))
}

fn refused(status: reqwest::StatusCode, body: &str) -> ToolError {
    super::web_search::refusal(super::web_search::SearchApi::Exa, status, body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exa_results_and_its_event_stream_read() {
        let text = "Title: 15.2.0\nURL: https://github.com/BurntSushi/ripgrep/releases/tag/15.2.0\nPublished: \
                    2026-07-15T16:26:10Z\nAuthor: N/A\nHighlights:\n# 15.2.0\n\nThis release fixes bugs.\n\n\
                    Title: Releases\nURL: https://github.com/BurntSushi/ripgrep/releases\nPublished: N/A\n\
                    Highlights:\nAll releases.";
        let hits = exa_hits(text);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].title, "15.2.0");
        assert_eq!(hits[0].date.as_deref(), Some("2026-07-15"));
        assert_eq!(hits[0].extract, "# 15.2.0 This release fixes bugs.");
        assert_eq!(hits[1].date, None);
        let sse = format!(
            "event: message\ndata: {}\n\n",
            serde_json::json!({"jsonrpc": "2.0", "id": 2, "result": {"content": [{"type": "text", "text": "x"}]}})
        );
        assert_eq!(
            rpc_message(&sse).unwrap()["result"]["content"][0]["text"],
            "x"
        );
        assert!(rpc_message("{\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{}}").is_some());
    }
}
