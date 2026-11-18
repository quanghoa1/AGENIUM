//! The model call of a learning step: one structured question, its answer parsed.

use super::*;

pub(super) async fn ask(
    provider: &dyn Provider,
    m: &PipelineModel,
    rules: &str,
    user: String,
    report: &mut LearnReport,
) -> Result<Value, MemoryError> {
    let req = ChatRequest {
        model: m.model.clone(),
        system: vec![SystemBlock {
            text: rules.to_owned(),
            cache: false,
        }],
        messages: vec![Message::user_text(user)],
        tools: vec![],
        effort: m.effort.clone(),
        max_tokens: m.max_tokens,
        cache_key: None,
        long_cache: false,
        tools_off: false,
    };
    // An answer that is empty or not JSON is asked for once more: a cheap model sometimes answers nothing at all
    // (DeepSeek, the shared-pool eval of 2026-10-31), and the whole episode was lost to it.
    let mut text = String::new();
    for _ in 0..2 {
        let resp = m
            .retry
            .run(
                |_| provider.chat(&req, &|_| {}),
                tokio::time::sleep,
                |_, _, _| {},
            )
            .await
            .map_err(|e| MemoryError::Model(e.to_string()))?;
        report.tokens_in += resp.usage.total_input();
        report.tokens_out += resp.usage.output;
        report.usage.add(&resp.usage);
        text = resp.message.text();
        if let Some(v) = json_object(&text) {
            return Ok(v);
        }
        // An extraction cut off by the token budget still holds every fact it finished: keep those rather than lose
        // the whole episode (2026-10-25: DeepSeek's answer for a ticket stopped mid-fact and the ticket taught nothing).
        if let Some(facts) = complete_items(&text, "facts") {
            report.rejected.push(format!(
                "the extraction answer was cut off; kept its {} complete fact(s)",
                facts.len()
            ));
            return Ok(serde_json::json!({ "facts": facts }));
        }
    }
    Err(MemoryError::Model(format!(
        "the answer is not JSON: {}",
        text.chars().take(200).collect::<String>()
    )))
}

/// A memory id the model named: a number, or the same written as text (`"2"`, `"id=2"`). Gemini Flash answered
/// `["2"]` for the shared lesson that misled the work, and the blame was lost (the shared-pool eval, 2026-10-31).
pub(super) fn memory_id(x: &Value) -> Option<i64> {
    x.as_i64().or_else(|| {
        let s = x.as_str()?.trim();
        s.strip_prefix("id=").unwrap_or(s).trim().parse().ok()
    })
}
