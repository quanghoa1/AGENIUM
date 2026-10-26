//! Provider errors and their classification (decision 15 / 3j): retry, fatal, or quota.

use std::time::Duration;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum LlmError {
    #[error("network error: {0}")]
    Network(String),
    #[error("request timed out")]
    Timeout,
    #[error("stream dropped before completion: {0}")]
    StreamDropped(String),
    #[error("HTTP {status} ({kind}): {message}")]
    Http {
        status: u16,
        /// Vendor error type/code (`rate_limit_error`, `insufficient_quota`, `RESOURCE_EXHAUSTED`, ...).
        kind: String,
        message: String,
        /// The provider says credit/quota is exhausted (not a temporary rate limit).
        quota: bool,
        retry_after: Option<Duration>,
    },
    /// The provider streamed an error event mid-response (e.g. `overloaded_error`).
    #[error("provider error ({kind}): {message}")]
    Stream {
        kind: String,
        message: String,
        retryable: bool,
    },
    #[error("cannot decode provider response: {0}")]
    Decode(String),
    #[error("provider not configured: {0}")]
    Config(String),
    #[error("model does not support {0}")]
    Unsupported(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Classified {
    /// Retry with backoff; `Some` = the provider asked for this delay (`retry-after`).
    Retry(Option<Duration>),
    /// 400/401/403/404...: report, do not retry.
    Fatal,
    /// 402 / quota-coded 429: the agent becomes QuotaExhausted (decision 3j).
    Quota,
}

pub fn classify(e: &LlmError) -> Classified {
    match e {
        LlmError::Network(_) | LlmError::Timeout | LlmError::StreamDropped(_) => {
            Classified::Retry(None)
        }
        LlmError::Stream {
            retryable: true, ..
        } => Classified::Retry(None),
        LlmError::Http { quota: true, .. } | LlmError::Http { status: 402, .. } => {
            Classified::Quota
        }
        LlmError::Http {
            status: 429,
            retry_after,
            ..
        } => Classified::Retry(*retry_after),
        LlmError::Http {
            status: 408 | 409,
            retry_after,
            ..
        } => Classified::Retry(*retry_after),
        // 5xx includes Anthropic 529 overloaded.
        LlmError::Http {
            status: 500..=599,
            retry_after,
            ..
        } => Classified::Retry(*retry_after),
        _ => Classified::Fatal,
    }
}

/// The request did not fit the model's context. Every vendor words it differently (texts seen from Anthropic, OpenAI,
/// Gemini, DeepSeek, OpenRouter, LiteLLM, Ollama, LM Studio, llama.cpp, vLLM); a rate limit or a throttle is never
/// this, and neither is a bare 400 or 413 without a matching text (ADR-103).
pub fn is_context_overflow(e: &LlmError) -> bool {
    let (status, kind, message) = match e {
        LlmError::Http {
            status,
            kind,
            message,
            ..
        } => (Some(*status), kind.as_str(), message.as_str()),
        LlmError::Stream { kind, message, .. } => (None, kind.as_str(), message.as_str()),
        _ => return false,
    };
    if status == Some(429) {
        return false;
    }
    let text = format!("{kind} {message}").to_ascii_lowercase();
    if [
        "rate limit",
        "rate_limit",
        "throttl",
        "quota",
        "too many requests",
    ]
    .iter()
    .any(|w| text.contains(w))
    {
        return false;
    }
    [
        "context_length_exceeded",
        "context length",
        "context window",
        "context_window_exceeded",
        "maximum context",
        "prompt is too long",
        "prompt too long",
        "prompt exceeds max length",
        "input is too long",
        "input too long",
        "too many tokens",
        "too many input tokens",
        "request_too_large",
        "request too large",
        "exceeds the context",
        "available context size",
        "exceed context",
        "exceeds the maximum number of tokens",
        "input token count",
        "reduce the length of the messages",
        "string_above_max_length",
    ]
    .iter()
    .any(|w| text.contains(w))
}

/// Parses `retry-after` (seconds or HTTP date; dates are treated as "unknown").
pub fn parse_retry_after(v: Option<&str>) -> Option<Duration> {
    let v = v?.trim();
    v.parse::<f64>()
        .ok()
        .filter(|s| s.is_finite() && *s >= 0.0)
        .map(|s| Duration::from_millis((s * 1000.0) as u64))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn http(status: u16, quota: bool) -> LlmError {
        LlmError::Http {
            status,
            kind: String::new(),
            message: String::new(),
            quota,
            retry_after: None,
        }
    }

    #[test]
    fn classification_table() {
        assert_eq!(
            classify(&LlmError::Network("x".into())),
            Classified::Retry(None)
        );
        assert_eq!(
            classify(&LlmError::StreamDropped("eof".into())),
            Classified::Retry(None)
        );
        assert_eq!(classify(&http(429, false)), Classified::Retry(None));
        assert_eq!(classify(&http(429, true)), Classified::Quota);
        assert_eq!(classify(&http(402, false)), Classified::Quota);
        assert_eq!(classify(&http(529, false)), Classified::Retry(None));
        assert_eq!(classify(&http(503, false)), Classified::Retry(None));
        for s in [400, 401, 403, 404, 413] {
            assert_eq!(classify(&http(s, false)), Classified::Fatal, "{s}");
        }
        let ra = LlmError::Http {
            status: 429,
            kind: "rate_limit_error".into(),
            message: String::new(),
            quota: false,
            retry_after: Some(Duration::from_secs(7)),
        };
        assert_eq!(
            classify(&ra),
            Classified::Retry(Some(Duration::from_secs(7)))
        );
        assert_eq!(
            parse_retry_after(Some("2.5")),
            Some(Duration::from_millis(2500))
        );
        assert_eq!(
            parse_retry_after(Some("Wed, 21 Oct 2026 07:28:00 GMT")),
            None
        );
    }
}

#[cfg(test)]
mod overflow_tests {
    use super::*;

    fn http(status: u16, kind: &str, message: &str) -> LlmError {
        LlmError::Http {
            status,
            kind: kind.into(),
            message: message.into(),
            quota: false,
            retry_after: None,
        }
    }

    #[test]
    fn context_overflow_in_each_vendor_s_words() {
        for (status, kind, message) in [
            (400, "invalid_request_error", "prompt is too long: 210345 tokens > 200000 maximum"),
            (413, "request_too_large", "Request exceeds the maximum allowed number of bytes."),
            (400, "context_length_exceeded", "This model's maximum context length is 128000 tokens."),
            (400, "INVALID_ARGUMENT", "The input token count (1200000) exceeds the maximum number of tokens allowed (1048576)."),
            (400, "", "This endpoint's maximum context length is 65536 tokens. Please reduce the length of the messages."),
            (400, "", "the request exceeds the available context size, try increasing it"),
            (500, "", "prompt too long; exceeded max context length by 1200 tokens"),
            (400, "model_context_window_exceeded", ""),
        ] {
            assert!(is_context_overflow(&http(status, kind, message)), "{kind} {message}");
        }
    }

    #[test]
    fn rate_limits_and_bare_errors_are_not_overflow() {
        assert!(!is_context_overflow(&http(
            429,
            "rate_limit_error",
            "too many tokens per minute"
        )));
        assert!(!is_context_overflow(&http(
            400,
            "ThrottlingException",
            "Too many tokens, please wait"
        )));
        assert!(!is_context_overflow(&http(400, "", "")));
        assert!(!is_context_overflow(&http(413, "", "")));
        assert!(!is_context_overflow(&LlmError::Timeout));
    }
}
