//! Strict tool arguments (ADR-144): Claude's and OpenAI's APIs decode a tool call against its schema, so the
//! arguments of a tool sent with `strict: true` always parse. Each takes a subset of JSON Schema; a schema is fitted
//! to it here (constraints the decoder does not take are dropped — the tool checks them as before; an object with keys
//! of its own choosing is asked as JSON text), or left as it is when it cannot be (free-form arguments, `$ref`).
//!
//! Claude: every object `additionalProperties: false`; no numeric, length or pattern constraints; per request at most
//! 20 strict tools, 24 optional parameters and 16 parameters with alternatives (`anyOf`, a list of types) — past
//! them the request is refused, so tools are admitted in order while they fit. OpenAI: every property listed in
//! `required`, an optional one taking `null` as well (read back as not given, `coerce`).

use serde_json::{json, Map, Value};

use crate::error::LlmError;
use crate::types::ChatRequest;

/// Claude's per-request limits on strict schemas.
pub const CLAUDE_TOOLS: usize = 20;
pub const CLAUDE_OPTIONAL: usize = 24;
pub const CLAUDE_UNIONS: usize = 16;

/// String formats both decoders take.
const FORMATS: &[&str] = &[
    "date-time",
    "time",
    "date",
    "duration",
    "email",
    "hostname",
    "uri",
    "ipv4",
    "ipv6",
    "uuid",
];

/// Keywords neither decoder takes, dropped (the tool still checks what they said).
const DROPPED: &[&str] = &[
    "minimum",
    "maximum",
    "exclusiveMinimum",
    "exclusiveMaximum",
    "multipleOf",
    "minLength",
    "maxLength",
    "pattern",
    "maxItems",
    "uniqueItems",
    "minProperties",
    "maxProperties",
    "contains",
    "examples",
];

/// What a fitted schema costs against Claude's limits.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Cost {
    pub optional: usize,
    pub unions: usize,
}

/// `schema` fitted to the strict subset, with its cost; `None` when it has no strict form.
pub fn fit(schema: &Value) -> Option<(Value, Cost)> {
    let mut cost = Cost::default();
    let v = node(schema, &mut cost)?;
    v.get("type")
        .and_then(Value::as_str)
        .filter(|t| *t == "object")?;
    Some((v, cost))
}

fn node(s: &Value, cost: &mut Cost) -> Option<Value> {
    let Value::Object(m) = s else {
        // `true` (anything) has no strict form; `false` never occurs in tool schemas.
        return None;
    };
    if m.contains_key("$ref") || m.contains_key("not") || m.contains_key("if") {
        return None;
    }
    // An object with keys of its own choosing (a template's section, a map) has no strict form: it is asked as a
    // JSON object written as text, which the arguments' repair reads back as the object (`coerce`).
    let extra = m.get("additionalProperties");
    let no_props = m
        .get("properties")
        .and_then(Value::as_object)
        .is_none_or(Map::is_empty);
    let free = m.get("type").is_some_and(|t| t == "object")
        && (extra.is_some_and(|x| x != &Value::Bool(false)) || no_props && extra.is_none());
    if free {
        let what = m
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        let text = format!("{what} A JSON object, written as text.")
            .trim()
            .to_owned();
        return Some(json!({"type": "string", "description": text}));
    }
    let mut out = Map::new();
    for (k, v) in m {
        match k.as_str() {
            k if DROPPED.contains(&k) => {}
            "format" if !v.as_str().is_some_and(|f| FORMATS.contains(&f)) => {}
            "minItems" if v.as_u64().is_some_and(|n| n > 1) => {}
            "properties" => {
                let mut props = Map::new();
                for (name, p) in v.as_object()? {
                    props.insert(name.clone(), node(p, cost)?);
                }
                out.insert(k.clone(), Value::Object(props));
            }
            "items" => {
                out.insert(k.clone(), node(v, cost)?);
            }
            "anyOf" | "oneOf" | "allOf" => {
                let alts: Option<Vec<Value>> =
                    v.as_array()?.iter().map(|x| node(x, cost)).collect();
                // One of several shapes: Claude's decoder takes `anyOf` (a value fits one of them).
                let key = if k == "allOf" { "allOf" } else { "anyOf" };
                out.insert(key.into(), Value::Array(alts?));
            }
            _ => {
                out.insert(k.clone(), v.clone());
            }
        }
    }
    let is_object = match &out.get("type") {
        Some(Value::String(t)) => t == "object",
        Some(Value::Array(a)) => a.iter().any(|t| t == "object"),
        _ => out.contains_key("properties"),
    };
    if is_object {
        if !out.contains_key("properties") {
            out.insert("properties".into(), json!({}));
        }
        out.insert("additionalProperties".into(), Value::Bool(false));
        let props = out["properties"].as_object().cloned().unwrap_or_default();
        let required: Vec<&str> = out
            .get("required")
            .and_then(Value::as_array)
            .map(|r| r.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        for (name, p) in &props {
            if !required.contains(&name.as_str()) {
                cost.optional += 1;
            }
            if p.get("anyOf").is_some() || p.get("type").is_some_and(Value::is_array) {
                cost.unions += 1;
            }
        }
    }
    Some(Value::Object(out))
}

/// Which of the tools (their fitted cost; `None`: no strict form or not asked) Claude takes as strict in one request:
/// in order, while they fit its limits.
pub fn claude_admits(costs: &[Option<Cost>]) -> Vec<bool> {
    let (mut tools, mut optional, mut unions) = (0, 0, 0);
    costs
        .iter()
        .map(|c| match c {
            Some(c)
                if tools < CLAUDE_TOOLS
                    && optional + c.optional <= CLAUDE_OPTIONAL
                    && unions + c.unions <= CLAUDE_UNIONS =>
            {
                tools += 1;
                optional += c.optional;
                unions += c.unions;
                true
            }
            _ => false,
        })
        .collect()
}

/// A fitted schema in OpenAI's strict form: every property required, an optional one also taking `null`.
pub fn openai(fitted: &Value) -> Value {
    let mut v = fitted.clone();
    openai_node(&mut v);
    v
}

fn openai_node(v: &mut Value) {
    let Value::Object(m) = v else { return };
    if let Some(Value::Array(alts)) = m.get_mut("anyOf") {
        alts.iter_mut().for_each(openai_node);
    }
    if let Some(items) = m.get_mut("items") {
        openai_node(items);
    }
    if !m.get("properties").is_some_and(Value::is_object) {
        return;
    }
    let required: Vec<String> = m
        .get("required")
        .and_then(Value::as_array)
        .map(|r| {
            r.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let Some(Value::Object(props)) = m.get_mut("properties") else {
        return;
    };
    let names: Vec<String> = props.keys().cloned().collect();
    for (name, p) in props.iter_mut() {
        openai_node(p);
        if !required.contains(name) {
            *p = nullable(std::mem::take(p));
        }
    }
    m.insert("required".into(), json!(names));
}

/// `s` taking `null` too.
fn nullable(mut s: Value) -> Value {
    let accepts = |s: &Value| match &s["type"] {
        Value::String(t) => t == "null",
        Value::Array(a) => a.iter().any(|t| t == "null"),
        _ => false,
    };
    if accepts(&s) || s["anyOf"].as_array().is_some_and(|a| a.iter().any(accepts)) {
        return s;
    }
    if let Some(e) = s.get_mut("enum").and_then(Value::as_array_mut) {
        e.push(Value::Null);
    }
    match s.get("type").cloned() {
        Some(Value::String(t)) => {
            s["type"] = json!([t, "null"]);
            s
        }
        Some(Value::Array(mut a)) => {
            a.push(json!("null"));
            s["type"] = Value::Array(a);
            s
        }
        _ => match s.get_mut("anyOf").and_then(Value::as_array_mut) {
            Some(a) => {
                a.push(json!({"type": "null"}));
                s
            }
            None => json!({"anyOf": [s, {"type": "null"}]}),
        },
    }
}

/// Endpoints and models (`<base url>|<model>`) that refused a strict schema in this process: sent without it since.
fn refusals() -> &'static std::sync::Mutex<std::collections::HashSet<String>> {
    static R: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<String>>> =
        std::sync::OnceLock::new();
    R.get_or_init(Default::default)
}

/// Whether `e` is the provider refusing a strict schema of `req` (a 400 about the schema): the request then goes again
/// without strict tools, and so does every later one to that endpoint and model — a schema the decoder does not take
/// costs one request, never the run.
pub fn refused(e: &LlmError, req: &ChatRequest) -> bool {
    req.tools.iter().any(|t| t.strict)
        && matches!(e, LlmError::Http { status: 400, message, .. }
            if { let m = message.to_ascii_lowercase(); m.contains("strict") || m.contains("schema") })
}

pub fn remember_refused(key: &str) {
    refusals()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(key.to_owned());
}

/// Whether `req` goes without strict tools to `key` (an earlier refusal), and the request so.
pub fn plain_for(key: &str, req: &ChatRequest) -> Option<ChatRequest> {
    let refused = req.tools.iter().any(|t| t.strict)
        && refusals()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(key);
    refused.then(|| plain(req))
}

/// `req` with no tool asked strict.
pub fn plain(req: &ChatRequest) -> ChatRequest {
    let mut r = req.clone();
    r.tools.iter_mut().for_each(|t| t.strict = false);
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report() -> Value {
        json!({
            "type": "object",
            "required": ["summary", "results"],
            "properties": {
                "summary": {"type": "string", "minLength": 1},
                "confidence": {"type": "number", "minimum": 0, "maximum": 1},
                "results": {"type": "array", "minItems": 1, "items": {
                    "type": "object", "required": ["ac_id"],
                    "properties": {
                        "ac_id": {"type": "string", "pattern": "^AC"},
                        "status": {"oneOf": [{"type": "string", "enum": ["met"]}, {"type": "string", "enum": ["unmet"]}]}
                    }
                }},
                "note": {"type": ["string", "null"], "format": "markdown"}
            }
        })
    }

    #[test]
    fn a_schema_is_fitted_to_the_strict_subset_and_costed() {
        let (v, cost) = fit(&report()).unwrap();
        assert_eq!(v["additionalProperties"], false);
        assert_eq!(
            v["properties"]["results"]["items"]["additionalProperties"],
            false
        );
        assert!(v["properties"]["summary"].get("minLength").is_none());
        assert!(v["properties"]["confidence"].get("minimum").is_none());
        assert_eq!(v["properties"]["results"]["minItems"], 1);
        assert!(v["properties"]["results"]["items"]["properties"]["ac_id"]
            .get("pattern")
            .is_none());
        assert!(v["properties"]["results"]["items"]["properties"]["status"]
            .get("anyOf")
            .is_some());
        assert!(v["properties"]["note"].get("format").is_none());
        // confidence, note (top) and status (item) are optional; note and status take alternatives.
        assert_eq!(
            cost,
            Cost {
                optional: 3,
                unions: 2
            }
        );
    }

    #[test]
    fn free_form_objects_are_asked_as_json_text() {
        let (v, _) = fit(&json!({"type": "object", "additionalProperties": false, "properties": {
            "extension": {"type": "object", "additionalProperties": true, "description": "Template section."},
            "env": {"type": "object", "additionalProperties": {"type": "string"}}
        }}))
        .unwrap();
        assert_eq!(
            v["properties"]["extension"],
            json!({"type": "string", "description": "Template section. A JSON object, written as text."})
        );
        assert_eq!(v["properties"]["env"]["type"], "string");
        // The arguments themselves free-form: no strict form.
        assert!(fit(&json!({"type": "object"})).is_none());
        assert!(
            fit(&json!({"type": "object", "properties": {}, "additionalProperties": false}))
                .is_some()
        );
    }

    #[test]
    fn claude_takes_strict_tools_while_they_fit_its_limits() {
        let c = |optional, unions| Some(Cost { optional, unions });
        assert_eq!(
            claude_admits(&[c(20, 0), None, c(5, 0), c(4, 16), c(0, 1)]),
            vec![true, false, false, true, false]
        );
        assert_eq!(
            claude_admits(&vec![c(0, 0); 22])
                .iter()
                .filter(|x| **x)
                .count(),
            CLAUDE_TOOLS
        );
    }

    #[test]
    fn openai_requires_every_property_and_lets_optional_ones_be_null() {
        let (v, _) = fit(&report()).unwrap();
        let o = openai(&v);
        assert_eq!(
            o["required"],
            json!(["confidence", "note", "results", "summary"])
        );
        assert_eq!(
            o["properties"]["confidence"]["type"],
            json!(["number", "null"])
        );
        assert_eq!(o["properties"]["note"]["type"], json!(["string", "null"]));
        assert_eq!(o["properties"]["summary"]["type"], "string");
        let item = &o["properties"]["results"]["items"];
        assert_eq!(item["required"], json!(["ac_id", "status"]));
        assert_eq!(
            item["properties"]["status"]["anyOf"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
    }
}
