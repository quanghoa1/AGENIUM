//! Contract tests: each adapter talks HTTP to a local fake server that replays recorded-style vendor responses.
//! The request each adapter sends is captured and checked against the vendor contract.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use agenium_core::Effort;
use agenium_llm::*;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

mod effort;
mod vendors;
mod wires;

#[derive(Clone)]
struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
    /// Close the connection after this many body bytes (simulates a dropped stream).
    cut_at: Option<usize>,
}

fn sse(status: u16, body: &str) -> Reply {
    Reply {
        status,
        headers: vec![("content-type".into(), "text/event-stream".into())],
        body: body.into(),
        cut_at: None,
    }
}

fn json_reply(status: u16, body: Value, headers: &[(&str, &str)]) -> Reply {
    let mut h: Vec<(String, String)> = vec![("content-type".into(), "application/json".into())];
    h.extend(headers.iter().map(|(k, v)| (k.to_string(), v.to_string())));
    Reply {
        status,
        headers: h,
        body: body.to_string(),
        cut_at: None,
    }
}

#[derive(Debug, Clone)]
struct Seen {
    path: String,
    headers: Vec<(String, String)>,
    body: Value,
}

impl Seen {
    fn header(&self, k: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(k))
            .map(|(_, v)| v.as_str())
    }
}

/// Serves `replies` in order, one connection each; returns the base URL and the captured requests.
async fn server(replies: Vec<Reply>) -> (String, Arc<Mutex<Vec<Seen>>>) {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let s2 = seen.clone();
    tokio::spawn(async move {
        for r in replies {
            let (mut sock, _) = l.accept().await.unwrap();
            let mut buf = Vec::new();
            let mut tmp = [0u8; 8192];
            let head_end = loop {
                let n = sock.read(&mut tmp).await.unwrap();
                buf.extend_from_slice(&tmp[..n]);
                if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    break p + 4;
                }
            };
            let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
            let mut lines = head.lines();
            let path = lines
                .next()
                .unwrap_or_default()
                .split_whitespace()
                .nth(1)
                .unwrap_or_default()
                .to_owned();
            let headers: Vec<(String, String)> = lines
                .filter_map(|l| {
                    l.split_once(':')
                        .map(|(a, b)| (a.trim().to_owned(), b.trim().to_owned()))
                })
                .collect();
            let len: usize = headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                .map(|(_, v)| v.parse().unwrap())
                .unwrap_or(0);
            while buf.len() - head_end < len {
                let n = sock.read(&mut tmp).await.unwrap();
                buf.extend_from_slice(&tmp[..n]);
            }
            let body: Value =
                serde_json::from_slice(&buf[head_end..head_end + len]).unwrap_or(Value::Null);
            s2.lock().unwrap().push(Seen {
                path,
                headers,
                body,
            });
            let mut resp = format!("HTTP/1.1 {} X\r\nconnection: close\r\n", r.status);
            for (k, v) in &r.headers {
                resp.push_str(&format!("{k}: {v}\r\n"));
            }
            resp.push_str("\r\n");
            sock.write_all(resp.as_bytes()).await.unwrap();
            let b = r.body.as_bytes();
            let b = &b[..r.cut_at.unwrap_or(b.len()).min(b.len())];
            // Send in small pieces to exercise the incremental SSE decoder.
            for chunk in b.chunks(37) {
                sock.write_all(chunk).await.unwrap();
                sock.flush().await.unwrap();
            }
            let _ = sock.shutdown().await;
        }
    });
    (format!("http://{addr}"), seen)
}

fn tool() -> ToolSpec {
    ToolSpec {
        name: "read_file".into(),
        description: "Read a file".into(),
        parameters: json!({"type": "object", "properties": {"path": {"type": "string"}, "limit": {"anyOf": [{"type": "integer"}, {"type": "null"}], "default": null}}, "required": ["path"], "additionalProperties": false}),
    }
}

fn request(model: &str) -> ChatRequest {
    ChatRequest {
        model: model.into(),
        system: vec![
            SystemBlock {
                text: "rules".into(),
                cache: false,
            },
            SystemBlock {
                text: "SOUL + ROLE".into(),
                cache: true,
            },
            SystemBlock {
                text: "manifest".into(),
                cache: false,
            },
        ],
        messages: vec![
            Message::user_text("Fix the bug"),
            Message {
                role: Role::Assistant,
                parts: vec![
                    Part::Thinking {
                        text: "plan".into(),
                        signature: Some("sig-1".into()),
                        provider: "anthropic".into(),
                        redacted: false,
                        model: None,
                    },
                    Part::ToolCall {
                        id: "toolu_1".into(),
                        name: "read_file".into(),
                        args: json!({"path": "a.rs"}),
                        signature: None,
                    },
                ],
            },
            Message {
                role: Role::User,
                parts: vec![Part::ToolResult {
                    call_id: "toolu_1".into(),
                    name: "read_file".into(),
                    content: "fn main(){}".into(),
                    is_error: false,
                }],
            },
        ],
        tools: vec![tool()],
        effort: Effort::HIGH,
        max_tokens: 64_000,
        cache_key: Some("agent_x:AGN-1".into()),
        long_cache: true,
        tools_off: false,
    }
}

fn collect() -> (Arc<Mutex<Vec<ChatEvent>>>, impl Fn(ChatEvent) + Send + Sync) {
    let evs = Arc::new(Mutex::new(Vec::new()));
    let e2 = evs.clone();
    (evs, move |e| e2.lock().unwrap().push(e))
}

fn http() -> agenium_llm::http::Http {
    agenium_llm::http::Http::new(Duration::from_secs(5), Duration::from_secs(5)).unwrap()
}

const ANTHROPIC_STREAM: &str = concat!(
    "event: message_start\n",
    "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"claude-opus-5-5\",\"content\":[],\"usage\":{\"input_tokens\":12,\"cache_creation_input_tokens\":0,\"cache_read_input_tokens\":4096,\"output_tokens\":1}}}\n\n",
    "event: content_block_start\n",
    "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"thinking\",\"thinking\":\"\",\"signature\":\"\"}}\n\n",
    "event: content_block_delta\n",
    "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"Look at main.\"}}\n\n",
    "event: content_block_delta\n",
    "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"signature_delta\",\"signature\":\"EqQBCkYI\"}}\n\n",
    "event: content_block_stop\n",
    "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
    "event: ping\n",
    "data: {\"type\":\"ping\"}\n\n",
    "event: content_block_start\n",
    "data: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
    "event: content_block_delta\n",
    "data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"text_delta\",\"text\":\"Editing now.\"}}\n\n",
    "event: content_block_stop\n",
    "data: {\"type\":\"content_block_stop\",\"index\":1}\n\n",
    "event: content_block_start\n",
    "data: {\"type\":\"content_block_start\",\"index\":2,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_2\",\"name\":\"edit_file\",\"input\":{}}}\n\n",
    "event: content_block_delta\n",
    "data: {\"type\":\"content_block_delta\",\"index\":2,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"path\\\": \\\"a.\"}}\n\n",
    "event: content_block_delta\n",
    "data: {\"type\":\"content_block_delta\",\"index\":2,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"rs\\\"}\"}}\n\n",
    "event: content_block_stop\n",
    "data: {\"type\":\"content_block_stop\",\"index\":2}\n\n",
    "event: message_delta\n",
    "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\",\"stop_sequence\":null},\"usage\":{\"output_tokens\":87}}\n\n",
    "event: message_stop\n",
    "data: {\"type\":\"message_stop\"}\n\n",
);

const OPENAI_STREAM: &str = concat!(
    "data: {\"id\":\"c1\",\"model\":\"gpt-5.5\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"Reading\"},\"finish_reason\":null}]}\n\n",
    "data: {\"id\":\"c1\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_9\",\"type\":\"function\",\"function\":{\"name\":\"read_file\",\"arguments\":\"{\\\"pa\"}}]},\"finish_reason\":null}]}\n\n",
    "data: {\"id\":\"c1\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"th\\\":\\\"b.rs\\\"}\"}}]},\"finish_reason\":null}]}\n\n",
    "data: {\"id\":\"c1\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
    "data: {\"id\":\"c1\",\"choices\":[],\"usage\":{\"prompt_tokens\":2000,\"completion_tokens\":40,\"prompt_tokens_details\":{\"cached_tokens\":1536},\"completion_tokens_details\":{\"reasoning_tokens\":12}}}\n\n",
    "data: [DONE]\n\n",
);

const GEMINI_STREAM: &str = concat!(
    "data: {\"candidates\":[{\"content\":{\"role\":\"model\",\"parts\":[{\"text\":\"Considering the file\",\"thought\":true}]}}],\"modelVersion\":\"gemini-3-pro\"}\n\n",
    "data: {\"candidates\":[{\"content\":{\"role\":\"model\",\"parts\":[{\"functionCall\":{\"name\":\"read_file\",\"args\":{\"path\":\"c.rs\"}},\"thoughtSignature\":\"sigG\"}]},\"finishReason\":\"STOP\"}],\"usageMetadata\":{\"promptTokenCount\":900,\"candidatesTokenCount\":20,\"cachedContentTokenCount\":512,\"thoughtsTokenCount\":30}}\n\n",
);

const RESPONSES_STREAM: &str = concat!(
    "event: response.created\ndata: {\"type\":\"response.created\",\"response\":{\"id\":\"r1\",\"model\":\"gpt-6-luna\"}}\n\n",
    "event: response.reasoning_summary_text.delta\ndata: {\"type\":\"response.reasoning_summary_text.delta\",\"delta\":\"thinking\"}\n\n",
    "event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"Hel\"}\n\n",
    "event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"lo\"}\n\n",
    "event: response.output_item.added\ndata: {\"type\":\"response.output_item.added\",\"output_index\":1,\"item\":{\"type\":\"function_call\",\"id\":\"fc_1\",\"call_id\":\"call_9\",\"name\":\"read_file\",\"arguments\":\"\"}}\n\n",
    "event: response.function_call_arguments.delta\ndata: {\"type\":\"response.function_call_arguments.delta\",\"output_index\":1,\"delta\":\"{\\\"path\\\":\"}\n\n",
    "event: response.function_call_arguments.delta\ndata: {\"type\":\"response.function_call_arguments.delta\",\"output_index\":1,\"delta\":\"\\\"a.txt\\\"}\"}\n\n",
    "event: response.output_item.done\ndata: {\"type\":\"response.output_item.done\",\"output_index\":1,\"item\":{\"type\":\"function_call\",\"id\":\"fc_1\",\"call_id\":\"call_9\",\"name\":\"read_file\",\"arguments\":\"{\\\"path\\\":\\\"a.txt\\\"}\"}}\n\n",
    "event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"model\":\"gpt-6-luna\",\"status\":\"completed\",\"usage\":{\"input_tokens\":120,\"input_tokens_details\":{\"cached_tokens\":100},\"output_tokens\":30,\"output_tokens_details\":{\"reasoning_tokens\":12}}}}\n\n",
);
