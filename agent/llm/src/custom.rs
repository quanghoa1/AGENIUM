//! What a user may add to a custom (OpenAI-compatible) endpoint's requests: extra headers, extra body fields and
//! where the effort level goes. Nothing set means Agenium's own request; a value set is sent as given. The rules
//! here are checked when the user saves and again when a request is built (the configuration file can be edited
//! by hand): Agenium's own fields (model, messages, tools, streaming, token limits) cannot be replaced, names are
//! plain, values are bounded, and no value may carry a `${...}` placeholder (secrets go in secret headers only).

use serde_json::{Map, Value};

/// Body fields Agenium sets itself: replacing them would change what the agent sends or reads back.
pub const RESERVED_FIELDS: &[&str] = &[
    "model",
    "messages",
    "input",
    "instructions",
    "prompt",
    "tools",
    "tool_choice",
    "functions",
    "function_call",
    "stream",
    "stream_options",
    "n",
    "response_format",
    "max_tokens",
    "max_completion_tokens",
    "max_output_tokens",
    "prompt_cache_key",
];

/// Headers the HTTP layer owns; a user value would break the request or let one request smuggle another.
pub const RESERVED_HEADERS: &[&str] = &[
    "host",
    "content-length",
    "content-type",
    "content-encoding",
    "connection",
    "transfer-encoding",
    "te",
    "trailer",
    "upgrade",
    "keep-alive",
    "expect",
    "accept-encoding",
];

pub const MAX_FIELDS: usize = 32;
pub const MAX_VALUE_BYTES: usize = 2048;
pub const MAX_PATH_DEPTH: usize = 4;
pub const MAX_HEADER_BYTES: usize = 4096;

/// One extra body field: where it goes (`reasoning.effort` = `{"reasoning": {"effort": ...}}`), its value, and the
/// one model it is for (`None` = every model of the endpoint).
#[derive(Debug, Clone, PartialEq)]
pub struct BodyField {
    pub path: Vec<String>,
    pub value: Value,
    pub model: Option<String>,
}

/// The user's additions to an endpoint's requests.
#[derive(Debug, Clone, Default)]
pub struct Custom {
    pub headers: Vec<(String, String)>,
    /// Effort levels the user gave a model: a chosen level is sent for it.
    pub efforts: std::collections::BTreeMap<String, Vec<agenium_core::Effort>>,
    pub body: Vec<BodyField>,
    /// Where the effort level goes instead of `reasoning_effort`.
    pub effort_field: Option<Vec<String>>,
}

/// A field path as segments, checked: plain names, not too deep, not one of Agenium's own fields.
pub fn check_path(path: &str) -> Result<Vec<String>, String> {
    let segs: Vec<String> = path.trim().split('.').map(str::to_owned).collect();
    if segs.is_empty() || segs.len() > MAX_PATH_DEPTH {
        return Err(format!(
            "field {path:?}: 1 to {MAX_PATH_DEPTH} names separated by dots"
        ));
    }
    for s in &segs {
        if s.is_empty()
            || s.len() > 64
            || !s
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Err(format!(
                "field {path:?}: names are letters, digits, '_' and '-'"
            ));
        }
    }
    if RESERVED_FIELDS.contains(&segs[0].to_ascii_lowercase().as_str()) {
        return Err(format!("field {path:?}: Agenium sets {} itself", segs[0]));
    }
    Ok(segs)
}

/// A field's value as typed: JSON when it parses (`0.2`, `true`, `{"a": 1}`), else the text itself.
pub fn parse_value(text: &str) -> Result<Value, String> {
    let t = text.trim();
    if t.len() > MAX_VALUE_BYTES {
        return Err(format!("a value is at most {MAX_VALUE_BYTES} bytes"));
    }
    let v = serde_json::from_str::<Value>(t).unwrap_or_else(|_| Value::String(t.to_owned()));
    check_value(&v)?;
    Ok(v)
}

fn check_value(v: &Value) -> Result<(), String> {
    match v {
        Value::String(s) => {
            if s.contains("${") {
                return Err(
                    "a value may not hold a ${...} placeholder: put secrets in a secret header"
                        .into(),
                );
            }
            if s.chars().any(|c| c.is_control() && c != '\n' && c != '\t') {
                return Err("a value may not hold control characters".into());
            }
            Ok(())
        }
        Value::Array(a) => a.iter().try_for_each(check_value),
        Value::Object(o) => o.values().try_for_each(check_value),
        _ => Ok(()),
    }
}

/// A body field as stored (`path`, value text, model) checked into a [`BodyField`].
pub fn check_field(path: &str, value: &str, model: Option<&str>) -> Result<BodyField, String> {
    Ok(BodyField {
        path: check_path(path)?,
        value: parse_value(value)?,
        model: model
            .map(str::trim)
            .filter(|m| !m.is_empty())
            .map(str::to_owned),
    })
}

/// A user header checked: a plain name the HTTP layer does not own, a value on one line.
pub fn check_header(name: &str, value: &str) -> Result<(), String> {
    let n = name.trim();
    if n.is_empty()
        || n.len() > 64
        || !n
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(format!("header {n:?}: letters, digits, '-' and '_'"));
    }
    let lower = n.to_ascii_lowercase();
    if RESERVED_HEADERS.contains(&lower.as_str()) || lower.starts_with("proxy-") {
        return Err(format!("header {n}: set by the HTTP layer, not by hand"));
    }
    if value.len() > MAX_HEADER_BYTES || value.chars().any(|c| c == '\r' || c == '\n' || c == '\0')
    {
        return Err(format!(
            "header {n}: one line, at most {MAX_HEADER_BYTES} bytes"
        ));
    }
    Ok(())
}

/// Puts `v` at `path` in `body`, making the objects on the way (a non-object on the way is replaced).
fn set_path(body: &mut Value, path: &[String], v: Value) {
    let mut at = body;
    for (i, seg) in path.iter().enumerate() {
        if !at.is_object() {
            *at = Value::Object(Map::new());
        }
        let o = at.as_object_mut().expect("an object");
        if i + 1 == path.len() {
            o.insert(seg.clone(), v);
            return;
        }
        at = o
            .entry(seg.clone())
            .or_insert_with(|| Value::Object(Map::new()));
    }
}

/// The user's additions on a request body built by Agenium: the effort level moved to its own field when one is
/// set, then the extra fields (those for every model first, then the model's own, so the model's win). A field that
/// fails the rules is left out (the configuration may have been edited by hand).
pub fn apply(body: &mut Value, c: &Custom, model: &str) {
    if let Some(path) = &c.effort_field {
        let sent = body
            .as_object_mut()
            .and_then(|o| o.remove("reasoning_effort"))
            .or_else(|| {
                let r = body.get_mut("reasoning")?.as_object_mut()?;
                let e = r.remove("effort");
                if r.is_empty() {
                    body.as_object_mut()?.remove("reasoning");
                }
                e
            });
        if let Some(e) = sent {
            if check_path(&path.join(".")).is_ok() {
                set_path(body, path, e);
            }
        }
    }
    let fields = c
        .body
        .iter()
        .filter(|f| f.model.is_none())
        .chain(c.body.iter().filter(|f| f.model.as_deref() == Some(model)));
    for f in fields.take(MAX_FIELDS) {
        let ok = check_path(&f.path.join(".")).is_ok()
            && check_value(&f.value).is_ok()
            && f.value.to_string().len() <= MAX_VALUE_BYTES;
        // A field that breaks the rules is left out; saving it was refused with the reason.
        if ok {
            set_path(body, &f.path, f.value.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn agenium_s_own_fields_and_odd_names_are_refused() {
        for bad in [
            "model",
            "messages",
            "Tools",
            "stream_options.include_usage",
            "max_tokens",
            "a..b",
            "a b",
            "a.b.c.d.e",
            "x$y",
        ] {
            assert!(check_path(bad).is_err(), "{bad}");
        }
        assert_eq!(
            check_path("reasoning.effort").unwrap(),
            vec!["reasoning", "effort"]
        );
        assert!(parse_value("${secret:provider/x}").is_err());
        assert!(parse_value(r#"{"a": "${cred:K}"}"#).is_err());
        assert!(parse_value(&"x".repeat(MAX_VALUE_BYTES + 1)).is_err());
        assert_eq!(parse_value("0.2").unwrap(), json!(0.2));
        assert_eq!(parse_value("high").unwrap(), json!("high"));
        for (n, v) in [
            ("Host", "x"),
            ("Content-Length", "1"),
            ("Proxy-Authorization", "x"),
            ("X-A", "a\r\nX-B: b"),
            ("X A", "v"),
        ] {
            assert!(check_header(n, v).is_err(), "{n}");
        }
        assert!(check_header("X-Tenant-Id", "team-7").is_ok());
    }

    #[test]
    fn fields_land_where_named_and_the_effort_moves() {
        let mut body = json!({"model": "m", "messages": [], "reasoning_effort": "high"});
        let c = Custom {
            body: vec![
                check_field("temperature", "0.2", None).unwrap(),
                check_field("thinking.budget", "2048", Some("m")).unwrap(),
                check_field("temperature", "0.7", Some("m")).unwrap(),
                check_field("top_p", "0.9", Some("other")).unwrap(),
                // Edited into the file by hand: never applied.
                BodyField {
                    path: vec!["model".into()],
                    value: json!("evil"),
                    model: None,
                },
            ],
            effort_field: Some(vec!["reasoning".into(), "effort".into()]),
            ..Default::default()
        };
        apply(&mut body, &c, "m");
        assert_eq!(body["model"], "m");
        assert_eq!(body["temperature"], 0.7, "the model's own value wins");
        assert_eq!(body["thinking"]["budget"], 2048);
        assert!(body.get("top_p").is_none());
        assert!(body.get("reasoning_effort").is_none());
        assert_eq!(body["reasoning"]["effort"], "high");
    }
}
