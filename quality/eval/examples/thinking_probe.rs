//! Probes the history-editing check of Claude's thinking blocks (preserved thinking): an agent loop that edits an
//! earlier tool result (clearing old results, a cut for notes) while replaying later thinking blocks. With Agenium's
//! adapter on Anthropic's own API such a block is dropped (not a 400) and counted; the probe prints the count for the
//! unchanged history, the edit with the thinking replayed, and the edit with the later thinking removed (what the loop
//! now does).
//!
//! cargo run -p agenium-eval --example thinking_probe -- --key-env <VAR> --model <claude model> [--effort max]
//!
//! Adaptive thinking may skip thinking on these easy lookups (Sonnet and Opus at high did); a higher effort makes it think.

use agenium_core::Effort;
use agenium_llm::*;
use anyhow::{anyhow, Result};
use serde_json::json;

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let model = arg(&args, "--model").ok_or_else(|| anyhow!("--model"))?;
    let var = arg(&args, "--key-env").ok_or_else(|| anyhow!("--key-env"))?;
    let effort: Effort = arg(&args, "--effort")
        .unwrap_or_else(|| "high".into())
        .parse()
        .map_err(|e: String| anyhow!(e))?;
    let key = std::env::var(&var).map_err(|_| anyhow!("{var} is not set"))?;
    let p = build_provider(&ProviderSpec {
        custom: Default::default(),
        id: "anthropic".into(),
        kind: "anthropic".into(),
        base_url: None,
        api_key: Some(key),
        max_concurrency: 1,
    })?;
    let tool = ToolSpec {
        name: "lookup".into(),
        description: "Looks up a record by key and returns its fields.".into(),
        parameters: json!({"type": "object", "properties": {"key": {"type": "string"}}, "required": ["key"]}),
    };
    let base = |messages: Vec<Message>| {
        ChatRequest {
        model: model.clone(),
        system: vec![SystemBlock {
            text: "You are a careful agent. Look up records one call at a time. Before each call, reason about what \
                   the previous result told you and which key it points to next."
                .into(),
            cache: true,
        }],
        messages,
        tools: vec![tool.clone()],
        effort: effort.clone(),
        max_tokens: 6000,
        cache_key: None,
        long_cache: false,
        tools_off: false,
    }
    };
    let result = |id: &str, text: &str| Message {
        role: Role::User,
        parts: vec![Part::ToolResult {
            call_id: id.into(),
            name: "lookup".into(),
            content: text.into(),
            is_error: false,
        }],
    };
    let call_id = |m: &Message| {
        m.parts.iter().find_map(|p| match p {
            Part::ToolCall { id, .. } => Some(id.clone()),
            _ => None,
        })
    };
    let thinking = |m: &Message| {
        m.parts
            .iter()
            .filter(|p| matches!(p, Part::Thinking { .. }))
            .count()
    };
    let show = |label: &str, r: std::result::Result<ChatResponse, LlmError>| match r {
        Ok(r) => println!(
            "{label}: accepted, {} replayed thinking block(s) dropped or flagged",
            r.dropped_thinking
        ),
        Err(e) => println!("{label}: REJECTED: {e}"),
    };

    // A chain: each record names the next key, so the model must think after reading each result.
    let mut msgs = vec![Message::user_text(
        "Start at record A-1 and follow the 'next' key of each record until a record has none. Then tell me the \
         owner of the last record.",
    )];
    let records = [
        format!(
            "key A-1; owner: Lan; next: B-2; notes: {}",
            "archived ledger line. ".repeat(80)
        ),
        "key B-2; owner: Minh; next: C-3".to_owned(),
        "key C-3; owner: Hoa; next: none".to_owned(),
    ];
    let mut first_result_at = None;
    let mut thinking_after_edit = 0;
    for (n, rec) in records.iter().enumerate() {
        let r = p
            .chat(&base(msgs.clone()), &|_| {})
            .await
            .map_err(|e| anyhow!("{e}"))?;
        let Some(id) = call_id(&r.message) else {
            println!("turn {}: no tool call; stopping the chain", n + 1);
            break;
        };
        println!("turn {}: {} thinking block(s)", n + 1, thinking(&r.message));
        if first_result_at.is_some() {
            thinking_after_edit += thinking(&r.message);
        }
        msgs.push(r.message.clone());
        msgs.push(result(&id, rec));
        first_result_at.get_or_insert(msgs.len() - 1);
    }
    let Some(edit_at) = first_result_at else {
        return Ok(());
    };
    println!("thinking blocks after the first result: {thinking_after_edit}");

    show(
        "history unchanged",
        p.chat(&base(msgs.clone()), &|_| {}).await,
    );
    let mut edited = msgs.clone();
    if let Part::ToolResult { content, .. } = &mut edited[edit_at].parts[0] {
        *content =
            "[cleared to save context: 1700 characters; call the tool again if you need it]".into();
    }
    show(
        "first result cleared, later thinking replayed",
        p.chat(&base(edited.clone()), &|_| {}).await,
    );
    for m in edited.iter_mut().skip(edit_at + 1) {
        m.parts.retain(|p| !matches!(p, Part::Thinking { .. }));
    }
    show(
        "first result cleared, later thinking removed",
        p.chat(&base(edited), &|_| {}).await,
    );
    Ok(())
}
