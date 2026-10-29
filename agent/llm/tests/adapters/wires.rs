//! The Anthropic and OpenAI-compatible wires: request contract, streams, errors, quota.

use super::*;

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
            cache_write_1h: 0,
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
        model: None,
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
            cache_write_1h: 0,
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
    let mut req = request("deepseek-v4-pro");
    req.effort = Effort::MEDIUM;
    req.messages[1].parts[0] = Part::Thinking {
        text: "earlier".into(),
        signature: None,
        provider: "openai_compat".into(),
        redacted: false,
        model: None,
    };
    let r = p.chat(&req, &|_| {}).await.unwrap();
    let b = seen.lock().unwrap()[0].body.clone();
    assert_eq!(
        b["reasoning_effort"], "high",
        "DeepSeek has no medium: the nearest, the higher on a tie"
    );
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

/// A refusal's `stop_details` reach the response (category and explanation, shown to the user), and a full context
/// window stops an answer like the output limit.
#[tokio::test]
async fn anthropic_refusal_details_and_a_full_context_window() {
    let refused = ANTHROPIC_STREAM.replace(
        r#"{"stop_reason":"tool_use","stop_sequence":null}"#,
        r#"{"stop_reason":"refusal","stop_sequence":null,"stop_details":{"type":"refusal","category":"cyber","explanation":"This request was declined because it could enable cyber harm."}}"#,
    );
    let full = ANTHROPIC_STREAM.replace(
        r#""stop_reason":"tool_use""#,
        r#""stop_reason":"model_context_window_exceeded""#,
    );
    let (base, _) = server(vec![sse(200, &refused), sse(200, &full)]).await;
    let p = anthropic::Anthropic::new("anthropic", Some(&base), "sk-test", http());
    let (_, emit) = collect();
    let r = p.chat(&request("claude-opus-5-5"), &emit).await.unwrap();
    assert_eq!(r.stop, StopReason::Refusal);
    let why = r.refusal.unwrap();
    assert_eq!(why.category.as_deref(), Some("cyber"));
    assert!(why.explanation.unwrap().contains("cyber harm"));
    let r = p.chat(&request("claude-opus-5-5"), &emit).await.unwrap();
    assert_eq!(r.stop, StopReason::MaxTokens);
    assert!(r.refusal.is_none());
}
