//! Vendor adapters: Gemini, the limiter, OpenCode, Azure, local servers, OpenRouter caching.

use super::*;

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
            cache_write_1h: 0,
            reasoning: 30
        }
    );
    // Gemini 2.5 uses thinkingBudget.
    let mut req = request("gemini-2.5-flash");
    req.effort = Effort::LOW;
    assert_eq!(
        gemini::build_body(&req)["generationConfig"]["thinkingConfig"]["thinkingBudget"],
        1024
    );
}

/// Review of 2026-10-27: a tool call without an id got a count within its response (`gemini_call_1`, `call_0`),
/// so the next response's call had the same id and the orchestrator replayed the first call's result for it.
#[test]
fn calls_without_an_id_get_ids_unique_across_responses() {
    let ev = |data: &str| agenium_llm::sse::SseEvent {
        event: None,
        data: data.into(),
    };
    let gemini_ids: Vec<String> = (0..2)
        .map(|_| {
            let mut st = gemini::StreamState::default();
            for chunk in GEMINI_STREAM.split("\n\n").filter(|c| !c.is_empty()) {
                st.apply(&ev(chunk.trim_start_matches("data: ")), &|_| {})
                    .unwrap();
            }
            st.finish().unwrap().message.tool_calls()[0].id.clone()
        })
        .collect();
    let openai_ids: Vec<String> = (0..2)
        .map(|_| {
            let mut st = openai::StreamState::default();
            for d in [
                r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"name":"read_file","arguments":"{\"path\":\"a\"}"}}]}}]}"#,
                r#"{"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}"#,
                "[DONE]",
            ] {
                st.apply(&ev(d), &|_| {}).unwrap();
            }
            st.finish().unwrap().message.tool_calls()[0].id.clone()
        })
        .collect();
    let responses_ids: Vec<String> = (0..2)
        .map(|_| {
            let mut st = responses::StreamState::default();
            for d in [
                r#"{"type":"response.output_item.done","output_index":0,"item":{"type":"function_call","name":"read_file","arguments":"{}"}}"#,
                r#"{"type":"response.completed","response":{"status":"completed","usage":{}}}"#,
            ] {
                st.apply(&ev(d), &|_| {}).unwrap();
            }
            st.finish().unwrap().message.tool_calls()[0].id.clone()
        })
        .collect();
    for ids in [&gemini_ids, &openai_ids, &responses_ids] {
        assert!(!ids[0].is_empty() && ids[0] != ids[1], "{ids:?}");
        assert_eq!(history::wire_id(&ids[0]), ids[0], "safe on every wire");
    }
}

/// Review of 2026-10-27: an MCP schema whose non-null `anyOf` branch is not an object made the Gemini schema pass
/// panic (`inner["nullable"] = ...` on a boolean).
#[test]
fn gemini_schema_pass_takes_any_anyof_branch() {
    for branch in [json!(true), json!("string"), json!(3)] {
        let s =
            json!({"type": "object", "properties": {"x": {"anyOf": [branch, {"type": "null"}]}}});
        let _ = gemini::sanitize_schema(&s);
    }
    let s = json!({"anyOf": [{"type": "string"}, {"type": "null"}], "description": "d"});
    assert_eq!(
        gemini::sanitize_schema(&s),
        json!({"type": "string", "nullable": true, "description": "d"})
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
    req.effort = Effort::MEDIUM;
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

/// ADR-111: Azure OpenAI's v1 API gets the key in Azure's `api-key` header too, and OpenAI's effort levels.
#[tokio::test]
async fn azure_openai_sends_the_api_key_header() {
    let (base, seen) = server(vec![sse(200, OPENAI_STREAM)]).await;
    let p = openai::OpenAiCompat::new(
        "azure-openai",
        &format!("{base}/contoso.openai.azure.com/openai/v1"),
        Some("az-key"),
        http(),
    );
    let mut req = request("gpt-6-luna");
    req.effort = Effort::MAX;
    p.chat(&req, &|_| {}).await.unwrap();
    let s = seen.lock().unwrap().clone();
    assert_eq!(s[0].header("api-key"), Some("az-key"));
    assert_eq!(s[0].header("authorization"), Some("Bearer az-key"));
    assert_eq!(s[0].body["reasoning_effort"], "max", "GPT-6 Luna has max");
}

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

/// Local servers are found by asking `<base>/models`; a port nobody listens on is skipped.
#[tokio::test]
async fn local_servers_are_discovered() {
    let (url, seen) = server(vec![json_reply(
        200,
        json!({"object": "list", "data": [{"id": "qwen3:8b"}, {"id": "llama3.2"}]}),
        &[],
    )])
    .await;
    let closed = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        format!("http://{}/v1", l.local_addr().unwrap())
    };
    let base = format!("{url}/v1");
    let http = agenium_llm::http::Http::default_client().unwrap();
    let found =
        agenium_llm::local::discover_at(&http, &[("lmstudio", &closed), ("ollama", &base)]).await;
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].kind, "ollama");
    assert_eq!(found[0].base_url, base);
    assert_eq!(found[0].models, vec!["qwen3:8b", "llama3.2"]);
    assert_eq!(seen.lock().unwrap()[0].path, "/v1/models");
}

/// Learned from pi: OpenRouter reports cache writes inside `prompt_tokens`; they are not plain input. The invariant
/// `input + cache_read + cache_write = prompt` holds and output includes reasoning.
#[tokio::test]
async fn openrouter_cache_writes_are_not_input() {
    let body = concat!(
        "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"ok\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":5000,\"completion_tokens\":60,\"prompt_tokens_details\":{\"cached_tokens\":3000,\"cache_write_tokens\":1500},\"completion_tokens_details\":{\"reasoning_tokens\":20}}}\n\n",
        "data: [DONE]\n\n"
    );
    let (base, _) = server(vec![sse(200, body)]).await;
    let p = openai::OpenAiCompat::new("openrouter", &format!("{base}/api/v1"), Some("k"), http());
    let r = p
        .chat(&request("anthropic/claude-sonnet"), &|_| {})
        .await
        .unwrap();
    assert_eq!(
        (
            r.usage.input,
            r.usage.cache_read,
            r.usage.cache_write,
            r.usage.output,
            r.usage.reasoning
        ),
        (500, 3000, 1500, 60, 20)
    );
    assert_eq!(r.usage.total_input(), 5000);
}
