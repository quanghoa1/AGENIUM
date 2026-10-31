//! The paths a CLI's own tool call names in its arguments, for the permission hook to check (ADR-129).

use serde_json::Value;

/// What the arguments of a CLI's own tool call name on the disk (ADR-129; path review of 2026-10-29): every string of
/// any argument, nested values and lists included, that a path key holds or that looks like a path (absolute, a drive
/// or a share, a separator, a `file:` URI of any case, percent-decoded). A search pattern that would start outside the
/// folder it searches is an `Err`. The text a call carries (`content` and the like) is not read. Only a few known keys
/// were: a path under another name, in a list, or as `FILE:///` or `file:///…%2F..` went unchecked.
pub(crate) fn named_paths(args: &Value) -> Result<Vec<String>, String> {
    fn walk(key: &str, v: &Value, out: &mut Vec<String>) -> Result<(), String> {
        match v {
            Value::Object(m) => m
                .iter()
                .try_for_each(|(k, x)| walk(&k.to_ascii_lowercase(), x, out)),
            Value::Array(a) => a.iter().try_for_each(|x| walk(key, x, out)),
            Value::String(_) if CONTENT_KEYS.iter().any(|c| key.ends_with(c)) => Ok(()),
            Value::String(s) if PATTERN_KEYS.iter().any(|p| key.ends_with(p)) => {
                if pattern_escapes(s) {
                    Err(format!(
                        "the pattern {s} reaches outside the folder searched: give one relative to it"
                    ))
                } else {
                    Ok(())
                }
            }
            Value::String(s) => {
                if let Some(p) = file_uri(s) {
                    out.push(p);
                } else if !s.contains("://") && (is_path_key(key) || looks_like_path(s)) {
                    out.push(s.clone());
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
    let mut out = Vec::new();
    walk("", args, &mut out)?;
    Ok(out)
}

/// Argument names (lowercase endings) that hold the text a call carries, not a path.
const CONTENT_KEYS: [&str; 13] = [
    "content",
    "contents",
    "text",
    "string",
    "str",
    "instruction",
    "description",
    "query",
    "message",
    "summary",
    "explanation",
    "prompt",
    "title",
];
/// Argument names (lowercase endings) that hold a search pattern (a glob, or a regex for some tools).
const PATTERN_KEYS: [&str; 5] = ["pattern", "includes", "excludes", "glob", "regex"];

/// Argument names (lowercase endings) that hold a path whatever it looks like (a bare file name too).
fn is_path_key(k: &str) -> bool {
    [
        "path",
        "paths",
        "file",
        "files",
        "directory",
        "directories",
        "dir",
        "dirs",
        "cwd",
        "uri",
        "folder",
        "folders",
        "root",
        // A move's or a copy's ends (`Source` was read as text before: the file a move took went unchecked, audit
        // M14, 2026-10-31).
        "source",
        "destination",
    ]
    .iter()
    .any(|s| k.ends_with(s))
}

fn looks_like_path(s: &str) -> bool {
    // Text over several lines (code, a message) is never a path, whatever key holds it.
    if s.contains('\n') {
        return false;
    }
    let b = s.as_bytes();
    s.starts_with(['/', '\\', '~'])
        || (b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic())
        || s.contains(['/', '\\'])
}

/// A search pattern that starts outside the folder searched: absolute, from a drive, a share or `~`, or with a `..`
/// step.
pub(crate) fn pattern_escapes(s: &str) -> bool {
    let b = s.as_bytes();
    s.starts_with(['/', '\\', '~'])
        || (b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic())
        || s.split(['/', '\\']).any(|seg| seg == "..")
}

/// The path of a `file:` URI, any case, percent-decoded (`file:///C:/x`, `FILE://localhost/x`).
fn file_uri(s: &str) -> Option<String> {
    s.get(..5).filter(|h| h.eq_ignore_ascii_case("file:"))?;
    let mut p = &s[5..];
    if let Some(r) = p.strip_prefix("//") {
        p = r.find('/').map_or(r, |i| &r[i..]);
    }
    // `/C:/x` names `C:/x`.
    let b = p.as_bytes();
    if b.len() >= 3 && b[0] == b'/' && b[2] == b':' && b[1].is_ascii_alphabetic() {
        p = &p[1..];
    }
    let mut bytes = Vec::with_capacity(p.len());
    let raw = p.as_bytes();
    let mut i = 0;
    while i < raw.len() {
        let hex = |c: u8| (c as char).to_digit(16);
        match (
            raw[i],
            raw.get(i + 1).and_then(|c| hex(*c)),
            raw.get(i + 2).and_then(|c| hex(*c)),
        ) {
            (b'%', Some(h), Some(l)) => {
                bytes.push((h * 16 + l) as u8);
                i += 3;
            }
            (c, ..) => {
                bytes.push(c);
                i += 1;
            }
        }
    }
    Some(String::from_utf8_lossy(&bytes).into_owned())
}
