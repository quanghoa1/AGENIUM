//! Shared HTTP plumbing for adapters: JSON POST with SSE response, idle timeout between chunks, error mapping.

use std::time::Duration;

use futures::StreamExt;
use reqwest::header::HeaderMap;

use crate::error::LlmError;
use crate::sse::{SseDecoder, SseEvent};

#[derive(Clone)]
pub struct Http {
    client: reqwest::Client,
    /// Max silence between two chunks of a stream (providers send pings during long thinking).
    idle: Duration,
}

impl Http {
    pub fn new(connect_timeout: Duration, idle: Duration) -> Result<Self, LlmError> {
        let client = reqwest::Client::builder()
            .connect_timeout(connect_timeout)
            .build()
            .map_err(|e| LlmError::Config(format!("http client: {e}")))?;
        Ok(Self { client, idle })
    }

    pub fn default_client() -> Result<Self, LlmError> {
        Self::new(Duration::from_secs(30), Duration::from_secs(180))
    }

    pub fn client(&self) -> &reqwest::Client {
        &self.client
    }

    fn map_send(e: reqwest::Error) -> LlmError {
        if e.is_timeout() {
            LlmError::Timeout
        } else {
            LlmError::Network(e.to_string())
        }
    }

    /// GET returning JSON; non-2xx mapped by `map_err(status, headers, body)`.
    pub async fn get_json(
        &self,
        url: &str,
        headers: &[(&str, String)],
        map_err: fn(u16, &HeaderMap, &str) -> LlmError,
    ) -> Result<serde_json::Value, LlmError> {
        let mut rb = self.client.get(url).timeout(Duration::from_secs(60));
        for (k, v) in headers {
            rb = rb.header(*k, v);
        }
        let resp = rb.send().await.map_err(Self::map_send)?;
        let status = resp.status().as_u16();
        let h = resp.headers().clone();
        let text = resp.text().await.map_err(Self::map_send)?;
        if !(200..300).contains(&status) {
            return Err(map_err(status, &h, &text));
        }
        serde_json::from_str(&text).map_err(|e| LlmError::Decode(e.to_string()))
    }

    /// POSTs JSON and returns the JSON response; non-2xx mapped by `map_err`.
    pub async fn post_json(
        &self,
        url: &str,
        headers: &[(&str, String)],
        body: &serde_json::Value,
        map_err: fn(u16, &HeaderMap, &str) -> LlmError,
    ) -> Result<serde_json::Value, LlmError> {
        let mut rb = self
            .client
            .post(url)
            .timeout(Duration::from_secs(60))
            .header("content-type", "application/json");
        for (k, v) in headers {
            rb = rb.header(*k, v);
        }
        let resp = rb.json(body).send().await.map_err(Self::map_send)?;
        let status = resp.status().as_u16();
        let h = resp.headers().clone();
        let text = resp.text().await.map_err(Self::map_send)?;
        if !(200..300).contains(&status) {
            return Err(map_err(status, &h, &text));
        }
        serde_json::from_str(&text).map_err(|e| LlmError::Decode(e.to_string()))
    }

    /// POSTs `body` and feeds each SSE event to `on_event` until it returns `Ok(true)` (done) or the stream
    /// ends. A stream that ends before `on_event` reported completion is `StreamDropped`.
    pub async fn post_sse(
        &self,
        url: &str,
        headers: &[(&str, String)],
        body: &serde_json::Value,
        map_err: fn(u16, &HeaderMap, &str) -> LlmError,
        mut on_event: impl FnMut(SseEvent) -> Result<bool, LlmError>,
    ) -> Result<(), LlmError> {
        let mut rb = self
            .client
            .post(url)
            .header("content-type", "application/json")
            .header("accept", "text/event-stream");
        for (k, v) in headers {
            rb = rb.header(*k, v);
        }
        let resp = tokio::time::timeout(self.idle, rb.body(body.to_string()).send())
            .await
            .map_err(|_| LlmError::Timeout)?
            .map_err(Self::map_send)?;
        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            let h = resp.headers().clone();
            let text = resp.text().await.unwrap_or_default();
            return Err(map_err(status, &h, &text));
        }
        let mut stream = resp.bytes_stream();
        let mut dec = SseDecoder::new();
        loop {
            let next = tokio::time::timeout(self.idle, stream.next())
                .await
                .map_err(|_| LlmError::StreamDropped(format!("no data for {:?}", self.idle)))?;
            match next {
                Some(Ok(bytes)) => {
                    for ev in dec.feed(&bytes) {
                        if on_event(ev)? {
                            return Ok(());
                        }
                    }
                }
                Some(Err(e)) => return Err(LlmError::StreamDropped(e.to_string())),
                None => {
                    if let Some(ev) = dec.finish() {
                        if on_event(ev)? {
                            return Ok(());
                        }
                    }
                    return Err(LlmError::StreamDropped(
                        "connection closed before the end of the response".into(),
                    ));
                }
            }
        }
    }
}

pub fn header(h: &HeaderMap, name: &str) -> Option<String> {
    h.get(name).and_then(|v| v.to_str().ok()).map(str::to_owned)
}
