//! Harmless repairs of a model's tool arguments against the tool's JSON Schema (ADR-105). Models send a number as
//! a string, one object where a list is expected, a list or an object as a JSON string, or `"true"` for a boolean;
//! a strict parser turns each into an `invalid_arguments` error that costs a step and counts towards the stop after
//! three. Only shapes whose meaning is not in doubt are changed: an unknown field, a missing field or a value of
//! another kind is left as it is, for the tool to refuse — except an argument under the name another harness gives
//! it (ADR-129: a vendor CLI working through Agenium's tools sends `file_path`, `new_str`, `query`...), renamed to
//! the schema's own name when that one is absent and the other is not a field of the schema.

use serde_json::{Map, Value};

/// `args` repaired against `schema` (a tool's `parameters`).
pub fn coerce(schema: &Value, args: Value) -> Value {
    // The whole argument object sent as a JSON string.
    let args = match args {
        Value::String(s) if s.trim_start().starts_with('{') => match serde_json::from_str(&s) {
            Ok(v @ Value::Object(_)) => v,
            _ => Value::String(s),
        },
        v => v,
    };
    fix(schema, schema, args)
}

fn types(s: &Value) -> Vec<&str> {
    match &s["type"] {
        Value::String(t) => vec![t.as_str()],
        Value::Array(a) => a.iter().filter_map(Value::as_str).collect(),
        _ => vec![],
    }
}

/// Follows a local `$ref` (`#/$defs/X` or `#/definitions/X`).
fn resolve<'a>(root: &'a Value, s: &'a Value) -> &'a Value {
    match s["$ref"].as_str() {
        Some(r) => r
            .strip_prefix("#/")
            .map(|path| path.split('/').fold(root, |v, k| &v[k]))
            .filter(|v| !v.is_null())
            .unwrap_or(s),
        None => s,
    }
}

fn fix(root: &Value, schema: &Value, v: Value) -> Value {
    let s = resolve(root, schema);
    let t = types(s);
    if t.is_empty() || v.is_null() && t.contains(&"null") {
        return v;
    }
    let want = |k: &str| t.contains(&k);
    match v {
        Value::String(text) => {
            let trimmed = text.trim();
            if want("string") {
                return Value::String(text);
            }
            if want("integer") {
                if let Ok(n) = trimmed.parse::<i64>() {
                    return Value::from(n);
                }
            }
            if want("number") {
                if let Ok(n) = trimmed.parse::<f64>() {
                    if let Some(n) = serde_json::Number::from_f64(n) {
                        return Value::Number(n);
                    }
                }
            }
            if want("boolean") {
                match trimmed {
                    "true" => return Value::Bool(true),
                    "false" => return Value::Bool(false),
                    _ => {}
                }
            }
            if want("array") || want("object") {
                if let Ok(parsed) = serde_json::from_str::<Value>(trimmed) {
                    let fits = (want("array") && parsed.is_array())
                        || (want("object") && parsed.is_object());
                    if fits {
                        return fix(root, schema, parsed);
                    }
                }
            }
            // One text where a list of texts is expected.
            if want("array") && types(resolve(root, &s["items"])).contains(&"string") {
                return Value::Array(vec![Value::String(text)]);
            }
            Value::String(text)
        }
        Value::Number(n) if want("string") && !want("number") && !want("integer") => {
            Value::String(n.to_string())
        }
        Value::Bool(b) if want("string") && !want("boolean") => Value::String(b.to_string()),
        Value::Array(items) if want("array") => {
            let item = &s["items"];
            Value::Array(
                items
                    .into_iter()
                    .map(|x| {
                        if item.is_object() {
                            fix(root, item, x)
                        } else {
                            x
                        }
                    })
                    .collect(),
            )
        }
        // One item where a list is expected.
        v @ Value::Object(_) if want("array") && !want("object") => {
            fix(root, schema, Value::Array(vec![v]))
        }
        Value::Object(map) if want("object") => {
            let props = s["properties"].as_object();
            let map = props.map(|p| rename_aliases(p, map.clone())).unwrap_or(map);
            let out: Map<String, Value> = map
                .into_iter()
                .map(|(k, x)| {
                    let x = match props.and_then(|p| p.get(&k)) {
                        Some(ps) => fix(root, ps, x),
                        None => x,
                    };
                    (k, x)
                })
                .collect();
            Value::Object(out)
        }
        v => v,
    }
}

/// Names other harnesses give an argument of Agenium's tools (Claude Code, Codex, Gemini/agy, Grok, OpenCode), by the
/// schema's name.
const ALIASES: &[(&str, &[&str])] = &[
    (
        "path",
        &[
            "file_path",
            "filePath",
            "filepath",
            "Path",
            "FilePath",
            "file",
            "absolute_path",
            "AbsolutePath",
            "target_file",
            "TargetFile",
            "DirectoryPath",
            "directory",
            "dir",
            "notebook_path",
        ],
    ),
    (
        "old_string",
        &["old_str", "oldString", "old_text", "TargetContent"],
    ),
    (
        "new_string",
        &[
            "new_str",
            "newString",
            "new_text",
            "replacement",
            "ReplacementContent",
        ],
    ),
    (
        "content",
        &["contents", "file_text", "Content", "CodeContent", "text"],
    ),
    ("pattern", &["query", "Query", "regex", "SearchPattern"]),
    ("command", &["cmd", "CommandLine"]),
    ("workdir", &["cwd", "Cwd", "working_directory"]),
];

/// Renames an argument sent under another harness's name to the schema's, when the schema's is absent and the other
/// name is not a field of the schema.
fn rename_aliases(props: &Map<String, Value>, mut map: Map<String, Value>) -> Map<String, Value> {
    for (name, aliases) in ALIASES {
        if !props.contains_key(*name) || map.contains_key(*name) {
            continue;
        }
        if let Some(a) = aliases
            .iter()
            .find(|a| map.contains_key(**a) && !props.contains_key(**a))
        {
            if let Some(v) = map.remove(*a) {
                map.insert((*name).to_owned(), v);
            }
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string"},
                "offset": {"type": ["integer", "null"]},
                "ratio": {"type": "number"},
                "replace_all": {"type": "boolean"},
                "label": {"type": "string"},
                "edits": {"type": "array", "items": {"$ref": "#/$defs/Edit"}},
                "tags": {"type": "array", "items": {"type": "string"}}
            },
            "$defs": {"Edit": {"type": "object", "properties": {"line": {"type": "integer"}}}}
        })
    }

    #[test]
    fn harmless_shapes_are_repaired() {
        let got = coerce(
            &schema(),
            json!({"path": "a.rs", "offset": "40", "ratio": " 0.5", "replace_all": "true", "label": 7,
                   "edits": "[{\"line\": \"3\"}]", "tags": "x"}),
        );
        assert_eq!(
            got,
            json!({"path": "a.rs", "offset": 40, "ratio": 0.5, "replace_all": true, "label": "7",
                   "edits": [{"line": 3}], "tags": ["x"]})
        );
        // One object where a list is expected; the whole object as a JSON string.
        let got = coerce(&schema(), json!("{\"edits\": {\"line\": 1}}"));
        assert_eq!(got, json!({"edits": [{"line": 1}]}));
    }

    #[test]
    fn doubtful_shapes_are_left_for_the_tool_to_refuse() {
        let v = json!({"offset": "forty", "replace_all": "yes", "unknown": "1", "path": null});
        assert_eq!(coerce(&schema(), v.clone()), v);
        assert_eq!(coerce(&schema(), json!(null)), json!(null));
        assert_eq!(coerce(&json!({}), json!({"a": "1"})), json!({"a": "1"}));
    }

    /// ADR-129: a CLI's own argument names reach the schema's; a name the schema has is never moved.
    #[test]
    fn another_harness_argument_names_are_renamed() {
        let read = json!({"type": "object", "properties": {"path": {"type": "string"}, "offset": {"type": "integer"}}});
        assert_eq!(
            coerce(&read, json!({"file_path": "src/a.rs", "offset": "3"})),
            json!({"path": "src/a.rs", "offset": 3})
        );
        // The schema's own name wins; the other stays for the tool to refuse.
        assert_eq!(
            coerce(&read, json!({"path": "a", "file_path": "b"})),
            json!({"path": "a", "file_path": "b"})
        );
        let edit = json!({"type": "object", "properties": {"path": {"type": "string"},
            "old_string": {"type": "string"}, "new_string": {"type": "string"}}});
        assert_eq!(
            coerce(
                &edit,
                json!({"TargetFile": "x", "old_str": "a", "replacement": "b"})
            ),
            json!({"path": "x", "old_string": "a", "new_string": "b"})
        );
        // `glob` is grep's own field: never taken for `pattern`.
        let grep = json!({"type": "object", "properties": {"pattern": {"type": "string"}, "glob": {"type": "string"}}});
        assert_eq!(
            coerce(&grep, json!({"query": "fn main", "glob": "*.rs"})),
            json!({"pattern": "fn main", "glob": "*.rs"})
        );
    }
}
