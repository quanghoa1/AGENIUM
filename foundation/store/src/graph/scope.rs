//! Names and scopes: canonical names, normalized scopes, whether an item's scope fits a context.

use super::*;

/// Normalised name: lowercase, every run of non-alphanumeric characters becomes one space, trimmed. `+` and `#`
/// directly after a letter or digit stay part of the word: "C#", "C++" and "C" are three languages, and "DS920+" is
/// not "DS920" (they all became "c" / "ds920" before ADR-94; `AgentStore::open` re-keys older databases).
pub fn canonical(s: &str) -> String {
    let mut out = String::new();
    let mut gap = false;
    for c in s.trim().chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            if gap && !out.is_empty() {
                out.push(' ');
            }
            out.push(c);
            gap = false;
        } else if matches!(c, '+' | '#') && !gap && !out.is_empty() {
            out.push(c);
        } else {
            gap = true;
        }
    }
    out
}

/// Canonical scope object: keys and string values normalised, empty values dropped.
pub fn normalize_scope(scope: &Value) -> Value {
    let mut m = serde_json::Map::new();
    if let Some(o) = scope.as_object() {
        let mut keys: Vec<&String> = o.keys().collect();
        keys.sort();
        for k in keys {
            let v = match &o[k] {
                Value::String(s) => canonical(s),
                Value::Number(n) => n.to_string(),
                Value::Bool(b) => b.to_string(),
                _ => continue,
            };
            let key = canonical(k).replace(' ', "_");
            if !v.is_empty() && !key.is_empty() {
                m.insert(key, Value::String(v));
            }
        }
    }
    Value::Object(m)
}

/// `k=v;k=v` (sorted) of a normalised scope; the identity part of unique indexes.
pub fn scope_key(scope: &Value) -> String {
    normalize_scope(scope)
        .as_object()
        .map(|o| {
            o.iter()
                .map(|(k, v)| format!("{k}={}", v.as_str().unwrap_or("")))
                .collect::<Vec<_>>()
                .join(";")
        })
        .unwrap_or_default()
}

/// Scope test of ADR-51 §5 between an item's scope and the context scope (both normalised).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ScopeMatch {
    /// A key present on both sides differs: the item must not be used here.
    Conflict,
    /// Factor for the re-ranker: 1.0 per matching key, `0.6` per key the context does not know.
    Factor(f64),
}

pub fn scope_match(item: &Value, ctx: &Value) -> ScopeMatch {
    let (Some(i), c) = (item.as_object(), ctx.as_object()) else {
        return ScopeMatch::Factor(1.0);
    };
    let mut f = 1.0;
    for (k, v) in i {
        match c.and_then(|c| c.get(k)) {
            Some(cv) if cv == v => f *= 1.05,
            Some(_) => return ScopeMatch::Conflict,
            None => f *= 0.6,
        }
    }
    ScopeMatch::Factor(f)
}
