//! Tools on every wire, cache breakpoints, effort levels per model, custom endpoints.

use super::*;

/// ADR-113: a notes request lists the tools but forbids calling them, in each wire's own words.
#[test]
fn tools_listed_but_not_callable_in_every_wire() {
    let mut req = request("m");
    req.tools_off = true;
    assert_eq!(
        anthropic::build_body(&req)["tool_choice"],
        json!({"type": "none"})
    );
    assert_eq!(
        openai::build_body(openai::Flavor::OpenAi, &req)["tool_choice"],
        "none"
    );
    assert_eq!(responses::build_body(&req)["tool_choice"], "none");
    assert_eq!(
        gemini::build_body(&req)["toolConfig"]["functionCallingConfig"]["mode"],
        "NONE"
    );
    for b in [
        anthropic::build_body(&req),
        openai::build_body(openai::Flavor::DeepSeek, &req),
    ] {
        assert!(b["tools"].as_array().is_some_and(|t| !t.is_empty()));
    }
    req.tools_off = false;
    assert!(openai::build_body(openai::Flavor::OpenAi, &req)
        .get("tool_choice")
        .is_none());
}

/// ADR-113: through OpenRouter, Anthropic models cache only at explicit breakpoints: after the stable system block
/// and on the last message; DeepSeek-style models need none.
#[test]
fn openrouter_marks_cache_breakpoints_for_anthropic_models() {
    let req = request("anthropic/claude-sonnet-5.5");
    let b = openai::build_body(openai::Flavor::OpenRouter, &req);
    let sys = b["messages"][0]["content"].as_array().unwrap();
    assert_eq!(sys.len(), 3);
    assert!(sys[0].get("cache_control").is_none());
    assert_eq!(sys[1]["cache_control"]["ttl"], "1h");
    let last = b["messages"].as_array().unwrap().last().unwrap().clone();
    assert_eq!(
        last["content"].as_array().unwrap().last().unwrap()["cache_control"]["type"],
        "ephemeral"
    );
    let plain = openai::build_body(openai::Flavor::OpenRouter, &request("deepseek/deepseek-v4"));
    assert!(plain["messages"][0]["content"].is_string());
}

/// ADR-113: the cache route goes to OpenRouter as its session id and to xAI as the conversation id.
#[tokio::test]
async fn cache_routes_reach_openrouter_and_xai() {
    for (host, header) in [
        ("openrouter.ai/api/v1", "x-session-id"),
        ("api.x.ai/v1", "x-grok-conv-id"),
    ] {
        let (base, seen) = server(vec![sse(200, OPENAI_STREAM)]).await;
        let p = openai::OpenAiCompat::new("v", &format!("{base}/{host}"), Some("k"), http());
        p.chat(&request("m"), &|_| {}).await.unwrap();
        let s = seen.lock().unwrap().clone();
        assert_eq!(s[0].header(header), Some("agent_x:AGN-1"), "{host}");
    }
}

/// ADR-113: on Anthropic's own API a replayed thinking block whose earlier history changed is dropped, not a 400
/// (accounts from 2026-08-31 enforce the check), and the response says how many were dropped.
#[tokio::test]
async fn claude_drops_thinking_after_an_edit_instead_of_failing() {
    let req = request("claude-haiku-5-5");
    let b = anthropic::build_body_with(&req, true);
    assert_eq!(
        b["thinking"]["block_binding"]["prefix_mismatch_behavior"],
        "drop_block"
    );
    assert!(anthropic::build_body(&req)["thinking"]
        .get("block_binding")
        .is_none());
    let stream = ANTHROPIC_STREAM.replacen(
        "\"content\":[],",
        "\"content\":[],\"input_transformations\":[{\"type\":\"thinking_dropped\",\"path\":\"messages.1.content.0\",\"reason\":\"prefix_binding_mismatch\"},{\"type\":\"something_new\"}],",
        1,
    );
    let (base, _) = server(vec![sse(200, &stream)]).await;
    let p = anthropic::Anthropic::new("a", Some(&base), "k", http());
    let r = p.chat(&req, &|_| {}).await.unwrap();
    assert_eq!(r.dropped_thinking, 1, "unknown entries are ignored");
}

/// Each model gets its own level under the vendor's name (ADR-114): a level the model lacks becomes its nearest (the
/// higher on a tie), and a model whose levels are unknown gets none (its vendor's default).
#[test]
fn each_model_gets_its_own_effort_level_on_the_wire() {
    let lv = |s: &str| s.parse::<Effort>().unwrap();
    let claude = |model: &str, e: Effort| {
        let mut r = request(model);
        r.effort = e;
        anthropic::build_body(&r)["output_config"]["effort"].clone()
    };
    assert_eq!(claude("claude-haiku-5-5", Effort::XHIGH), "xhigh");
    assert_eq!(claude("claude-sonnet-4-6", Effort::XHIGH), "max");
    assert_eq!(claude("claude-opus-4-5-20251101", Effort::MAX), "high");
    use agenium_llm::openai::{effort_for, Flavor};
    let level = |f: Flavor, model: &str, e: &str| {
        effort_for(f, model, &lv(e)).map(|l| l.as_str().to_owned())
    };
    assert_eq!(
        level(Flavor::DeepSeek, "deepseek-v4-pro", "medium").as_deref(),
        Some("high")
    );
    assert_eq!(
        level(Flavor::OpenCode, "deepseek-v4.1-flash", "low").as_deref(),
        Some("low")
    );
    assert_eq!(
        level(Flavor::OpenAi, "gpt-5.5-pro", "low").as_deref(),
        Some("medium")
    );
    assert_eq!(
        level(Flavor::OpenAi, "gpt-6.1-sol", "max").as_deref(),
        Some("max")
    );
    assert_eq!(
        level(Flavor::Xai, "grok-4.7", "max").as_deref(),
        Some("xhigh")
    );
    assert_eq!(
        level(Flavor::OpenRouter, "openai/gpt-5.5", "minimal").as_deref(),
        Some("minimal")
    );
    assert_eq!(
        level(Flavor::OpenAi, "gpt-4o", "high"),
        None,
        "not a reasoning model we know"
    );
    assert_eq!(level(Flavor::Generic, "llama-4-scout", "high"), None);
    let mut g = request("gemini-3-pro-preview");
    g.effort = Effort::MEDIUM;
    let tc = gemini::build_body(&g)["generationConfig"]["thinkingConfig"].clone();
    assert_eq!(tc["thinkingLevel"], "high", "Gemini 3 Pro has low and high");
    g.model = "gemini-9-ultra".into();
    let tc = gemini::build_body(&g)["generationConfig"]["thinkingConfig"].clone();
    assert!(tc.get("thinkingLevel").is_none(), "{tc}");
    let mut r = request("gpt-6-astra");
    r.effort = lv("none");
    assert_eq!(
        responses::build_body(&r)["reasoning"]["effort"],
        "low",
        "Astra cannot turn reasoning off"
    );
}

/// Anthropic's Models API says which effort levels each model takes (`capabilities.effort`); the list keeps them,
/// and the table fills a model it does not describe.
#[tokio::test]
async fn claude_models_carry_their_effort_levels() {
    let lvl = |l: &[&str]| {
        let mut e = json!({"supported": true});
        for x in ["low", "medium", "high", "xhigh", "max"] {
            e[x] = json!({"supported": l.contains(&x)});
        }
        e
    };
    let body = json!({"data": [
        {"id": "claude-haiku-5-5", "capabilities": {"effort": lvl(&["low", "medium", "high", "xhigh", "max"])}},
        {"id": "claude-sonnet-4-6", "capabilities": {"effort": lvl(&["low", "medium", "high", "max"])}},
        {"id": "claude-haiku-4-5-20251001", "capabilities": {"effort": {"supported": false}}},
        {"id": "claude-next"}
    ]});
    let (base, _) = server(vec![json_reply(200, body, &[])]).await;
    let p = anthropic::Anthropic::new("a", Some(&base), "k", http());
    let m = p.list_models().await.unwrap();
    let names = |i: usize| {
        m[i].efforts
            .as_ref()
            .map(|l| l.iter().map(|e| e.as_str()).collect::<Vec<_>>().join(","))
    };
    assert_eq!(names(0).as_deref(), Some("low,medium,high,xhigh,max"));
    assert_eq!(m[0].default_effort, Some(Effort::MEDIUM));
    assert_eq!(names(1).as_deref(), Some("low,medium,high,max"));
    assert_eq!(names(2).as_deref(), Some(""), "no effort control");
    assert_eq!(
        names(3).as_deref(),
        Some("low,medium,high,xhigh,max"),
        "from the table"
    );
}

/// OpenRouter's model list names each model's levels (`reasoning.supported_efforts`, highest first) and its default.
#[tokio::test]
async fn openrouter_models_carry_their_effort_levels() {
    let body = json!({"data": [
        {"id": "openai/gpt-5.5", "reasoning": {"supported_efforts": ["xhigh", "high", "medium", "low", "none"],
            "default_effort": "medium"}},
        {"id": "meta-llama/llama-4-scout", "reasoning": null}
    ]});
    let (base, _) = server(vec![json_reply(200, body, &[])]).await;
    let p = openai::OpenAiCompat::new(
        "or",
        &format!("{base}/openrouter.ai/api/v1"),
        Some("k"),
        http(),
    );
    let m = p.list_models().await.unwrap();
    let names: Vec<&str> = m[0]
        .efforts
        .as_ref()
        .unwrap()
        .iter()
        .map(|e| e.as_str())
        .collect();
    assert_eq!(names, ["none", "low", "medium", "high", "xhigh"]);
    assert_eq!(m[0].default_effort, Some(Effort::MEDIUM));
    assert_eq!(m[1].efforts, None);
}

/// A custom endpoint gets the user's headers on every request, and a model the user gave effort levels gets the
/// chosen level as `reasoning_effort` (a model Agenium knows nothing of gets none otherwise).
#[tokio::test]
async fn a_custom_endpoint_sends_the_users_headers_and_effort() {
    let (base, seen) = server(vec![sse(200, OPENAI_STREAM), sse(200, OPENAI_STREAM)]).await;
    let mut efforts = std::collections::BTreeMap::new();
    efforts.insert(
        "house-model".to_owned(),
        vec!["low".parse().unwrap(), "high".parse().unwrap()],
    );
    let p = openai::OpenAiCompat::new("gw", &format!("{base}/v1"), Some("sk-gw"), http())
        .customized(agenium_llm::custom::Custom {
            headers: vec![("X-Tenant-Id".into(), "team-7".into())],
            efforts,
            body: vec![agenium_llm::custom::check_field("temperature", "0.2", None).unwrap()],
            ..Default::default()
        });
    p.chat(&request("house-model"), &|_| {}).await.unwrap();
    p.chat(&request("other-model"), &|_| {}).await.unwrap();
    let s = seen.lock().unwrap().clone();
    assert_eq!(s[0].header("x-tenant-id"), Some("team-7"));
    assert_eq!(s[0].header("authorization"), Some("Bearer sk-gw"));
    assert_eq!(s[0].body["reasoning_effort"], "high");
    assert_eq!(s[0].body["temperature"], 0.2);
    assert!(s[1].body.get("reasoning_effort").is_none(), "{}", s[1].body);
}

/// ADR-144: a tool asked strict is sent strict to Anthropic's own API (its schema fitted, no input streamed ahead)
/// and to OpenAI's (every property required, the optional ones nullable); relays and other vendors get it as before.
#[test]
fn strict_tools_go_strict_only_to_claude_and_openai() {
    let mut req = request("m");
    req.tools[0].strict = true;
    let claude = anthropic::build_body_with(&req, true);
    let t = &claude["tools"][0];
    assert_eq!(t["strict"], true);
    assert!(t.get("eager_input_streaming").is_none());
    assert_eq!(t["input_schema"]["additionalProperties"], false);
    let relay = anthropic::build_body(&req);
    assert!(relay["tools"][0].get("strict").is_none());
    assert_eq!(relay["tools"][0]["eager_input_streaming"], true);

    let o = openai::build_body(openai::Flavor::OpenAi, &req);
    let f = &o["tools"][0]["function"];
    assert_eq!(f["strict"], true);
    assert_eq!(f["parameters"]["required"], json!(["limit", "path"]));
    assert!(f["parameters"]["properties"]["limit"]["anyOf"]
        .as_array()
        .unwrap()
        .contains(&json!({"type": "null"})));
    for flavor in [
        openai::Flavor::DeepSeek,
        openai::Flavor::OpenCode,
        openai::Flavor::Generic,
    ] {
        assert!(openai::build_body(flavor, &req)["tools"][0]["function"]
            .get("strict")
            .is_none());
    }
    // Not asked strict: as before everywhere.
    req.tools[0].strict = false;
    assert!(anthropic::build_body_with(&req, true)["tools"][0]
        .get("strict")
        .is_none());
}

/// ADR-144: a strict schema the API refuses costs one request, not the run: the request goes again without strict
/// tools, and so do the next ones to that endpoint and model.
#[tokio::test]
async fn a_refused_strict_schema_is_sent_again_plain() {
    let refusal = json!({"type": "error", "error": {"type": "invalid_request_error",
        "message": "tools.0.input_schema: Schema is too complex for compilation"}});
    let (base, seen) = server(vec![
        json_reply(400, refusal.clone(), &[]),
        sse(200, ANTHROPIC_STREAM),
        sse(200, ANTHROPIC_STREAM),
    ])
    .await;
    let p = anthropic::Anthropic::new("a", Some(&format!("{base}/api.anthropic.com")), "k", http());
    let mut req = request("claude-haiku-5-5");
    req.tools[0].strict = true;
    p.chat(&req, &|_| {}).await.unwrap();
    p.chat(&req, &|_| {}).await.unwrap();
    let s = seen.lock().unwrap().clone();
    assert_eq!(s.len(), 3);
    assert_eq!(s[0].body["tools"][0]["strict"], true);
    assert!(s[1].body["tools"][0].get("strict").is_none());
    assert!(s[2].body["tools"][0].get("strict").is_none());

    let (base, seen) = server(vec![json_reply(400, refusal, &[]), sse(200, OPENAI_STREAM)]).await;
    let p = openai::OpenAiCompat::new("o", &format!("{base}/api.openai.com/v1"), Some("k"), http());
    p.chat(&req, &|_| {}).await.unwrap();
    let s = seen.lock().unwrap().clone();
    assert_eq!(s[0].body["tools"][0]["function"]["strict"], true);
    assert!(s[1].body["tools"][0]["function"].get("strict").is_none());
}
