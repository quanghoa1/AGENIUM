//! Contract tests: each adapter talks HTTP to a local fake server that replays recorded-style vendor responses.
//! The request each adapter sends is captured and checked against the vendor contract.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use agenium_core::Effort;
use agenium_llm::*;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

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
        effort: Effort::High,
        max_tokens: 64_000,
        cache_key: Some("agent_x:AGN-1".into()),
        long_cache: true,
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

#[tokio::test]
async fn anthropic_request_contract_and_stream_parsing() {
    let (base, seen) = server(vec![sse(200, ANTHROPIC_STREAM)]).await;
    let p = anthropic::Anthropic::new("anthropic", Some(&base), "sk-test", http());
    let (evs, emit) = collect();
    let r = p.chat(&request("claude-opus-5-5"), &emit).await.unwrap();

    let s = seen.lock().unwrap()[0].clone();
    assert_eq!(s.path, "/v1/messages");
    assert_eq!(s.header("x-api-key"), Some("sk-test"));
    assert_eq!(s.header("anthropic-version"), Some("2026-10-21"));
    let b = &s.body;
    for k in ["temperature", "top_p", "top_k", "tool_choice"] {
        assert!(b.get(k).is_none(), "{k} must not be sent");
    }
    assert_eq!(b["stream"], true);
    assert_eq!(b["thinking"]["type"], "adaptive");
    assert_eq!(b["output_config"]["effort"], "high");
    assert_eq!(
        b["tools"][0]["cache_control"],
        json!({"type": "ephemeral", "ttl": "1h"})
    );
    assert_eq!(b["tools"][0]["eager_input_streaming"], true);
    assert!(b["system"][0].get("cache_control").is_none());
    assert_eq!(b["system"][1]["cache_control"]["ttl"], "1h");
    let msgs = b["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 3);
    assert_eq!(
        msgs[1]["content"][0],
        json!({"type": "thinking", "thinking": "plan", "signature": "sig-1"}),
        "thinking replayed unchanged"
    );
    assert_eq!(msgs[2]["content"][0]["type"], "tool_result");
    assert_eq!(
        msgs[2]["content"][0]["cache_control"],
        json!({"type": "ephemeral"}),
        "last-message breakpoint, short TTL last"
    );
    let breakpoints = b.to_string().matches("cache_control").count();
    assert!(breakpoints <= 4, "{breakpoints} breakpoints");

    assert_eq!(r.stop, StopReason::ToolUse);
    assert_eq!(
        r.usage,
        Usage {
            input: 12,
            output: 87,
            cache_read: 4096,
            cache_write: 0,
            reasoning: 0
        }
    );
    assert_eq!(r.message.parts.len(), 3);
    assert!(
        matches!(&r.message.parts[0], Part::Thinking { signature: Some(s), text, .. } if s == "EqQBCkYI" && text == "Look at main.")
    );
    assert_eq!(r.message.tool_calls()[0].args, json!({"path": "a.rs"}));
    let evs = evs.lock().unwrap();
    assert!(evs.contains(&ChatEvent::TextDelta("Editing now.".into())));
    assert!(evs
        .iter()
        .any(|e| matches!(e, ChatEvent::ToolCall(c) if c.name == "edit_file")));
    assert_eq!(evs.last(), Some(&ChatEvent::Done));
}

#[tokio::test]
async fn anthropic_older_model_uses_budget_and_other_vendor_thinking_is_dropped() {
    let (base, seen) = server(vec![sse(200, ANTHROPIC_STREAM)]).await;
    let p = anthropic::Anthropic::new("anthropic", Some(&base), "k", http());
    let mut req = request("claude-haiku-4-5");
    req.messages[1].parts[0] = Part::Thinking {
        text: "x".into(),
        signature: None,
        provider: "gemini".into(),
        redacted: false,
    };
    p.chat(&req, &|_| {}).await.unwrap();
    let b = seen.lock().unwrap()[0].body.clone();
    assert_eq!(
        b["thinking"],
        json!({"type": "enabled", "budget_tokens": 24576})
    );
    assert!(b.get("output_config").is_none());
    assert_eq!(
        b["messages"][1]["content"][0]["type"], "tool_use",
        "foreign thinking is not replayed"
    );
}

#[tokio::test]
async fn anthropic_errors_classify_and_dropped_stream_retries() {
    let overloaded = json_reply(
        529,
        json!({"type": "error", "error": {"type": "overloaded_error", "message": "Overloaded"}}),
        &[],
    );
    let rate = json_reply(
        429,
        json!({"type": "error", "error": {"type": "rate_limit_error", "message": "slow down"}}),
        &[("retry-after", "0")],
    );
    let mut dropped = sse(200, ANTHROPIC_STREAM);
    dropped.cut_at = Some(ANTHROPIC_STREAM.len() / 2);
    let (base, seen) = server(vec![overloaded, rate, dropped, sse(200, ANTHROPIC_STREAM)]).await;
    let p = anthropic::Anthropic::new("anthropic", Some(&base), "k", http());
    let policy = RetryPolicy {
        base: Duration::from_millis(1),
        cap: Duration::from_millis(5),
        max_attempts: 8,
        max_total: Duration::from_secs(10),
    };
    let (evs, emit) = collect();
    let mut retries = Vec::new();
    let r = chat_with_retry(
        &p,
        &request("claude-opus-5-5"),
        &policy,
        &emit,
        |a, _, e| retries.push((a, e.to_string())),
    )
    .await
    .unwrap();
    assert_eq!(r.stop, StopReason::ToolUse);
    assert_eq!(retries.len(), 3, "{retries:?}");
    assert!(
        retries[0].1.contains("529")
            && retries[1].1.contains("429")
            && retries[2].1.contains("stream dropped")
    );
    assert_eq!(
        evs.lock()
            .unwrap()
            .iter()
            .filter(|e| **e == ChatEvent::Restart)
            .count(),
        3
    );
    // The retried request is byte-identical (frozen request, cache-friendly).
    {
        let s = seen.lock().unwrap();
        assert!(s.windows(2).all(|w| w[0].body == w[1].body));
    }

    let (base, _) = server(vec![json_reply(
        402,
        json!({"type": "error", "error": {"type": "billing_error", "message": "no credit"}}),
        &[],
    )])
    .await;
    let p = anthropic::Anthropic::new("anthropic", Some(&base), "k", http());
    let e = chat_with_retry(
        &p,
        &request("claude-opus-5-5"),
        &policy,
        &|_| {},
        |_, _, _| {},
    )
    .await
    .unwrap_err();
    assert!(matches!(e, CallError::Quota(_)), "{e:?}");
    let (base, _) = server(vec![json_reply(
        401,
        json!({"type": "error", "error": {"type": "authentication_error", "message": "bad key"}}),
        &[],
    )])
    .await;
    let p = anthropic::Anthropic::new("anthropic", Some(&base), "k", http());
    let e = chat_with_retry(
        &p,
        &request("claude-opus-5-5"),
        &policy,
        &|_| {},
        |_, _, _| {},
    )
    .await
    .unwrap_err();
    assert!(
        matches!(e, CallError::Fatal(LlmError::Http { status: 401, .. })),
        "{e:?}"
    );
}

const OPENAI_STREAM: &str = concat!(
    "data: {\"id\":\"c1\",\"model\":\"gpt-5.5\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"Reading\"},\"finish_reason\":null}]}\n\n",
    "data: {\"id\":\"c1\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_9\",\"type\":\"function\",\"function\":{\"name\":\"read_file\",\"arguments\":\"{\\\"pa\"}}]},\"finish_reason\":null}]}\n\n",
    "data: {\"id\":\"c1\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"th\\\":\\\"b.rs\\\"}\"}}]},\"finish_reason\":null}]}\n\n",
    "data: {\"id\":\"c1\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
    "data: {\"id\":\"c1\",\"choices\":[],\"usage\":{\"prompt_tokens\":2000,\"completion_tokens\":40,\"prompt_tokens_details\":{\"cached_tokens\":1536},\"completion_tokens_details\":{\"reasoning_tokens\":12}}}\n\n",
    "data: [DONE]\n\n",
);

#[tokio::test]
async fn openai_compat_contract_and_usage() {
    let (base, seen) = server(vec![sse(200, OPENAI_STREAM)]).await;
    // A fake host named like the real API so the OpenAI flavor is used.
    let p = openai::OpenAiCompat::new(
        "openai",
        &format!("{base}/v1?api.openai.com"),
        Some("sk-o"),
        http(),
    );
    let r = p.chat(&request("gpt-5.5"), &|_| {}).await.unwrap();
    let s = seen.lock().unwrap()[0].clone();
    assert!(
        s.path.starts_with("/v1?api.openai.com/chat/completions")
            || s.path.contains("chat/completions"),
        "{}",
        s.path
    );
    assert_eq!(s.header("authorization"), Some("Bearer sk-o"));
    let b = &s.body;
    assert_eq!(
        b["messages"][0],
        json!({"role": "system", "content": "rules\n\nSOUL + ROLE\n\nmanifest"})
    );
    assert_eq!(
        b["messages"][2]["tool_calls"][0]["function"]["arguments"],
        "{\"path\":\"a.rs\"}"
    );
    assert!(b["messages"][2].get("reasoning_content").is_none());
    assert_eq!(
        b["messages"][3],
        json!({"role": "tool", "tool_call_id": "toolu_1", "content": "fn main(){}"})
    );
    assert_eq!(b["reasoning_effort"], "high");
    assert_eq!(b["prompt_cache_key"], "agent_x:AGN-1");
    assert_eq!(b["max_completion_tokens"], 64000);
    assert_eq!(b["stream_options"]["include_usage"], true);
    assert_eq!(r.stop, StopReason::ToolUse);
    assert_eq!(r.message.tool_calls()[0].args, json!({"path": "b.rs"}));
    assert_eq!(
        r.usage,
        Usage {
            input: 464,
            output: 40,
            cache_read: 1536,
            cache_write: 0,
            reasoning: 12
        }
    );
}

#[tokio::test]
async fn deepseek_reasoning_and_local_endpoint_without_done() {
    let ds = concat!(
        "data: {\"model\":\"deepseek-reasoner\",\"choices\":[{\"index\":0,\"delta\":{\"reasoning_content\":\"think\"}}]}\n\n",
        "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"ok\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":100,\"completion_tokens\":5,\"prompt_cache_hit_tokens\":64,\"prompt_cache_miss_tokens\":36}}\n\n",
        "data: [DONE]\n\n"
    );
    let (base, seen) = server(vec![sse(200, ds)]).await;
    let p = openai::OpenAiCompat::new(
        "deepseek",
        &format!("{base}/deepseek.com"),
        Some("k"),
        http(),
    );
    let mut req = request("deepseek-reasoner");
    req.effort = Effort::Medium;
    req.messages[1].parts[0] = Part::Thinking {
        text: "earlier".into(),
        signature: None,
        provider: "openai_compat".into(),
        redacted: false,
    };
    let r = p.chat(&req, &|_| {}).await.unwrap();
    let b = seen.lock().unwrap()[0].body.clone();
    assert_eq!(b["reasoning_effort"], "high", "DeepSeek medium -> high");
    assert_eq!(b["messages"][2]["reasoning_content"], "earlier");
    assert_eq!(b["max_tokens"], 64000);
    assert!(matches!(&r.message.parts[0], Part::Thinking { text, .. } if text == "think"));
    assert_eq!(r.usage.cache_read, 64);

    // A local server (Ollama-like): no API key, no stream_options, closes after finish_reason without [DONE].
    let local = "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"hi\"},\"finish_reason\":\"stop\"}]}\n\n";
    let (base, seen) = server(vec![sse(200, local)]).await;
    let p = openai::OpenAiCompat::new("local", &format!("{base}/v1"), None, http());
    let r = p.chat(&request("llama3.2"), &|_| {}).await.unwrap();
    assert_eq!(r.message.text(), "hi");
    let s = seen.lock().unwrap()[0].clone();
    assert!(s.header("authorization").is_none());
    assert!(s.body.get("reasoning_effort").is_none() && s.body.get("stream_options").is_none());
}

#[tokio::test]
async fn openai_quota_error_is_quota() {
    let (base, _) = server(vec![json_reply(429, json!({"error": {"message": "You exceeded your current quota", "type": "insufficient_quota", "code": "insufficient_quota"}}), &[])]).await;
    let p = openai::OpenAiCompat::new("openai", &format!("{base}/v1"), Some("k"), http());
    let policy = RetryPolicy {
        base: Duration::from_millis(1),
        cap: Duration::from_millis(1),
        max_attempts: 3,
        max_total: Duration::from_secs(1),
    };
    let e = chat_with_retry(&p, &request("gpt-5.5"), &policy, &|_| {}, |_, _, _| {})
        .await
        .unwrap_err();
    assert!(matches!(e, CallError::Quota(_)), "{e:?}");
}

const GEMINI_STREAM: &str = concat!(
    "data: {\"candidates\":[{\"content\":{\"role\":\"model\",\"parts\":[{\"text\":\"Considering the file\",\"thought\":true}]}}],\"modelVersion\":\"gemini-3-pro\"}\n\n",
    "data: {\"candidates\":[{\"content\":{\"role\":\"model\",\"parts\":[{\"functionCall\":{\"name\":\"read_file\",\"args\":{\"path\":\"c.rs\"}},\"thoughtSignature\":\"sigG\"}]},\"finishReason\":\"STOP\"}],\"usageMetadata\":{\"promptTokenCount\":900,\"candidatesTokenCount\":20,\"cachedContentTokenCount\":512,\"thoughtsTokenCount\":30}}\n\n",
);

#[tokio::test]
async fn gemini_contract_schema_sanitising_and_signatures() {
    let (base, seen) = server(vec![sse(200, GEMINI_STREAM)]).await;
    let p = gemini::Gemini::new("gemini", Some(&base), "gk", http());
    let r = p.chat(&request("gemini-3-pro"), &|_| {}).await.unwrap();
    let s = seen.lock().unwrap()[0].clone();
    assert_eq!(
        s.path,
        "/v1beta/models/gemini-3-pro:streamGenerateContent?alt=sse"
    );
    assert_eq!(s.header("x-goog-api-key"), Some("gk"));
    let b = &s.body;
    assert_eq!(b["systemInstruction"]["parts"].as_array().unwrap().len(), 3);
    assert_eq!(
        b["generationConfig"]["thinkingConfig"]["thinkingLevel"],
        "high"
    );
    let params = &b["tools"][0]["functionDeclarations"][0]["parameters"];
    assert!(params.get("additionalProperties").is_none());
    assert_eq!(
        params["properties"]["limit"],
        json!({"type": "integer", "nullable": true})
    );
    assert_eq!(b["contents"][1]["role"], "model");
    assert_eq!(
        b["contents"][2]["parts"][0]["functionResponse"]["name"],
        "read_file"
    );
    assert_eq!(r.stop, StopReason::ToolUse);
    assert!(
        matches!(&r.message.parts[1], Part::ToolCall { signature: Some(s), .. } if s == "sigG")
    );
    assert_eq!(
        r.usage,
        Usage {
            input: 388,
            output: 50,
            cache_read: 512,
            cache_write: 0,
            reasoning: 30
        }
    );
    // Gemini 2.5 uses thinkingBudget.
    let mut req = request("gemini-2.5-flash");
    req.effort = Effort::Low;
    assert_eq!(
        gemini::build_body(&req)["generationConfig"]["thinkingConfig"]["thinkingBudget"],
        1024
    );
}

#[tokio::test]
async fn limiter_bounds_concurrency() {
    let inner = Arc::new(mock::ScriptedProvider::new("m"));
    for _ in 0..4 {
        inner.push_text("x");
    }
    let lim = Arc::new(Limited::new(inner.clone(), 2));
    let mut hs = Vec::new();
    for _ in 0..4 {
        let l = lim.clone();
        hs.push(tokio::spawn(async move {
            l.chat(&request("scripted"), &|_| {}).await.unwrap()
        }));
    }
    for h in hs {
        h.await.unwrap();
    }
    assert_eq!(inner.requests().len(), 4);
}

#[tokio::test]
async fn opencode_relay_sends_session_affinity_and_deepseek_effort() {
    let (base, seen) = server(vec![sse(200, OPENAI_STREAM), sse(200, OPENAI_STREAM)]).await;
    // Base URL shaped like https://opencode.ai/zen/go/v1 (host check is on the string).
    let p = openai::OpenAiCompat::new(
        "opencode",
        &format!("{base}/opencode.ai/zen/go/v1"),
        Some("oc"),
        http(),
    );
    let mut req = request("deepseek-v4.1-flash");
    req.effort = Effort::Medium;
    p.chat(&req, &|_| {}).await.unwrap();
    req.cache_key = None;
    req.model = "glm-5.3".into();
    p.chat(&req, &|_| {}).await.unwrap();
    let s = seen.lock().unwrap().clone();
    assert_eq!(s[0].header("x-opencode-session"), Some("agent_x:AGN-1"));
    assert_eq!(
        s[0].body["reasoning_effort"], "high",
        "DeepSeek levels behind OpenCode"
    );
    assert_eq!(s[0].body["stream_options"]["include_usage"], true);
    assert!(s[1]
        .header("x-opencode-session")
        .is_some_and(|v| v.starts_with("oneshot-")));
    assert!(s[1].body.get("reasoning_effort").is_none());
}

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

#[tokio::test]
async fn opencode_router_picks_the_wire_per_model() {
    let (base, seen) = server(vec![
        sse(200, ANTHROPIC_STREAM),
        sse(200, OPENAI_STREAM),
        sse(200, RESPONSES_STREAM),
    ])
    .await;
    let p = opencode::OpenCode::new(
        "opencode-go",
        &format!("{base}/opencode.ai/zen/go/v1"),
        "oc",
        http(),
    );
    p.chat(&request("claude-haiku-5-5"), &|_| {}).await.unwrap();
    p.chat(&request("deepseek-v4.1-flash"), &|_| {})
        .await
        .unwrap();
    let (evs, emit) = collect();
    let r = p.chat(&request("gpt-6-luna"), &emit).await.unwrap();
    let s = seen.lock().unwrap().clone();
    assert_eq!(s[0].path, "/opencode.ai/zen/go/v1/messages");
    assert_eq!(s[0].header("x-api-key"), Some("oc"));
    assert_eq!(s[0].header("x-opencode-session"), Some("agent_x:AGN-1"));
    assert_eq!(s[1].path, "/opencode.ai/zen/go/v1/chat/completions");
    assert_eq!(s[1].header("authorization"), Some("Bearer oc"));
    assert_eq!(s[1].header("x-opencode-session"), Some("agent_x:AGN-1"));
    // OpenAI Responses wire for gpt-*.
    assert_eq!(s[2].path, "/opencode.ai/zen/go/v1/responses");
    assert_eq!(s[2].header("authorization"), Some("Bearer oc"));
    assert_eq!(s[2].header("x-opencode-session"), Some("agent_x:AGN-1"));
    let b = &s[2].body;
    assert_eq!(b["store"], false);
    assert_eq!(b["stream"], true);
    assert_eq!(b["instructions"], "rules\n\nSOUL + ROLE\n\nmanifest");
    assert_eq!(b["max_output_tokens"], 64_000);
    assert_eq!(b["prompt_cache_key"], "agent_x:AGN-1");
    assert_eq!(b["reasoning"]["effort"], "high");
    assert_eq!(b["tools"][0]["type"], "function");
    assert_eq!(b["tools"][0]["name"], "read_file");
    let input = b["input"].as_array().unwrap();
    assert_eq!(
        input[0],
        json!({"role": "user", "content": [{"type": "input_text", "text": "Fix the bug"}]})
    );
    assert_eq!(
        input[1],
        json!({"type": "function_call", "call_id": "toolu_1", "name": "read_file", "arguments": "{\"path\":\"a.rs\"}"})
    );
    assert_eq!(
        input[2],
        json!({"type": "function_call_output", "call_id": "toolu_1", "output": "fn main(){}"})
    );
    assert_eq!(input.len(), 3, "thinking of another vendor is not replayed");
    // The streamed answer.
    assert_eq!(r.message.text(), "Hello");
    let calls = r.message.tool_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].id, "call_9");
    assert_eq!(calls[0].args, json!({"path": "a.txt"}));
    assert_eq!(r.stop, StopReason::ToolUse);
    assert_eq!(
        (
            r.usage.input,
            r.usage.cache_read,
            r.usage.output,
            r.usage.reasoning
        ),
        (20, 100, 30, 12)
    );
    assert_eq!(r.model, "gpt-6-luna");
    let evs = evs.lock().unwrap();
    assert!(evs
        .iter()
        .any(|e| matches!(e, ChatEvent::ThinkingDelta(t) if t == "thinking")));
    assert!(evs
        .iter()
        .any(|e| matches!(e, ChatEvent::ToolCallStarted { name, .. } if name == "read_file")));
}

/// A failed Responses stream is a stream error (retryable when the server says so).
#[tokio::test]
async fn responses_failure_is_a_stream_error() {
    let failed = "event: response.failed\ndata: {\"type\":\"response.failed\",\"response\":{\"error\":{\"code\":\"server_error\",\"message\":\"boom\"}}}\n\n";
    let (base, _) = server(vec![sse(200, failed)]).await;
    let p = responses::Responses::new("x", &format!("{base}/v1"), Some("k"), http(), false);
    match p.chat(&request("gpt-6-luna"), &|_| {}).await {
        Err(LlmError::Stream {
            retryable, message, ..
        }) => {
            assert!(retryable);
            assert_eq!(message, "boom");
        }
        other => panic!("{other:?}"),
    }
}

/// ADR-31: a local Ollama server reports tool and vision support and context length through `/api/show`.
#[tokio::test]
async fn local_ollama_capabilities_are_detected() {
    let (url, seen) = server(vec![
        json_reply(200, json!({"data": [{"id": "qwen3:8b"}, {"id": "llama2"}]}), &[]),
        json_reply(404, json!({"error": "not found"}), &[]),
        json_reply(200, json!({"capabilities": ["completion", "tools"], "model_info": {"qwen3.context_length": 40960}}), &[]),
        json_reply(200, json!({"capabilities": ["completion"], "model_info": {"llama.context_length": 4096}}), &[]),
    ])
    .await;
    let p = openai::OpenAiCompat::new("local", &format!("{url}/v1"), None, http());
    let m = p.list_models().await.unwrap();
    assert_eq!(
        (m[0].supports_tools, m[0].context_window),
        (Some(true), Some(40960))
    );
    assert_eq!(m[0].supports_vision, Some(false));
    assert_eq!(
        (m[1].supports_tools, m[1].context_window),
        (Some(false), Some(4096))
    );
    let s = seen.lock().unwrap();
    assert_eq!(s[1].path, "/api/v0/models");
    assert_eq!(
        (s[2].path.as_str(), &s[2].body),
        ("/api/show", &json!({"model": "qwen3:8b"}))
    );
}

/// ADR-31: LM Studio reports `tool_use`, `vlm` and the context length in `/api/v0/models`.
#[tokio::test]
async fn local_lm_studio_capabilities_are_detected() {
    let (url, _) = server(vec![
        json_reply(200, json!({"data": [{"id": "qwen2-vl"}]}), &[]),
        json_reply(200, json!({"data": [{"id": "qwen2-vl", "type": "vlm", "max_context_length": 32768, "capabilities": ["tool_use"]}]}), &[]),
    ])
    .await;
    let p = openai::OpenAiCompat::new("local", &format!("{url}/v1"), None, http());
    let m = p.list_models().await.unwrap();
    assert_eq!(
        (
            m[0].supports_tools,
            m[0].supports_vision,
            m[0].context_window
        ),
        (Some(true), Some(true), Some(32768))
    );
}
