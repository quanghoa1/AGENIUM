//! Reading the model's answers: JSON in prose, cut-off lists, episode details that are not memories.

use super::*;

/// An obvious episode detail rather than durable knowledge: an id (agent id or ULID), a commit hash, or the exact byte
/// size of a file. `EXTRACT_RULES` forbids them, yet models still write them (2026-10-25 memory-reuse runs: 3 of 13
/// extracted facts), and recall then spends every later work order on them. Returns what it found.
pub fn ephemeral(text: &str) -> Option<&'static str> {
    let tokens: Vec<&str> = text
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .filter(|t| !t.is_empty())
        .collect();
    let ulid = |t: &str| {
        let t = t.strip_prefix("agent_").unwrap_or(t);
        t.len() == 26
            && t.chars().any(|c| c.is_ascii_digit())
            && t.chars()
                .all(|c| c.is_ascii_digit() || (c.is_ascii_uppercase() && !"ILOU".contains(c)))
    };
    if tokens.iter().any(|t| ulid(t)) {
        return Some("an id");
    }
    let hex = |t: &str| {
        (7..=40).contains(&t.len())
            && t.chars()
                .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
            && t.chars().any(|c| c.is_ascii_digit())
            && t.chars().any(|c| c.is_ascii_alphabetic())
    };
    let lower = text.to_lowercase();
    if tokens.iter().any(|t| t.len() == 40 && hex(t))
        || (lower.contains("commit") && tokens.iter().any(|t| hex(t)))
    {
        return Some("a commit hash");
    }
    let words: Vec<&str> = lower.split_whitespace().collect();
    if words.windows(2).any(|w| {
        w[0].trim_matches(|c: char| !c.is_ascii_digit())
            .parse::<u64>()
            .is_ok()
            && w[1].trim_matches(|c: char| !c.is_alphabetic()) == "bytes"
    }) {
        return Some("an exact file size");
    }
    None
}

/// First JSON object in a model answer (tolerates code fences and prose around it).
pub fn json_object(text: &str) -> Option<Value> {
    let start = text.find('{')?;
    let mut depth = 0i32;
    let mut in_str = false;
    let mut esc = false;
    for (i, c) in text[start..].char_indices() {
        if in_str {
            match (esc, c) {
                (true, _) => esc = false,
                (false, '\\') => esc = true,
                (false, '"') => in_str = false,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => in_str = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return serde_json::from_str(&text[start..start + i + 1]).ok();
                }
            }
            _ => {}
        }
    }
    None
}

/// The complete objects of the array under `key` in a JSON answer that was cut off before it closed.
pub fn complete_items(text: &str, key: &str) -> Option<Vec<Value>> {
    let at = text.find(&format!("\"{key}\""))?;
    let open = at + text[at..].find('[')?;
    let mut items = Vec::new();
    let (mut depth, mut in_str, mut esc, mut start) = (0i32, false, false, None);
    for (i, c) in text[open + 1..].char_indices() {
        if in_str {
            match (esc, c) {
                (true, _) => esc = false,
                (false, '\\') => esc = true,
                (false, '"') => in_str = false,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => in_str = true,
            '{' => {
                if depth == 0 {
                    start = Some(open + 1 + i);
                }
                depth += 1;
            }
            '}' => {
                depth -= 1;
                if depth == 0 {
                    let s = start.take()?;
                    items.push(serde_json::from_str(&text[s..open + 1 + i + 1]).ok()?);
                }
            }
            ']' if depth == 0 => break,
            _ => {}
        }
    }
    (!items.is_empty()).then_some(items)
}

pub(super) fn kind_of(k: Option<&str>) -> FactKind {
    match k {
        Some("preference") => FactKind::Preference,
        Some("lesson") => FactKind::Lesson,
        _ => FactKind::Fact,
    }
}

pub(super) fn is_empty_scope(v: &Value) -> bool {
    v.as_object().is_none_or(|o| o.is_empty())
}
