//! Secret detection (decision 34: gitleaks-style scan before auto-commit and push) and redaction of known
//! secret values and secret-shaped tokens in text that leaves the process (logs, tool output, run log).

use std::sync::{Arc, RwLock};

use regex::Regex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub rule: &'static str,
    pub line: usize,
    /// The match with its middle masked, safe to show.
    pub preview: String,
}

struct Rule {
    id: &'static str,
    re: Regex,
}

fn rules() -> &'static [Rule] {
    static RULES: std::sync::OnceLock<Vec<Rule>> = std::sync::OnceLock::new();
    RULES.get_or_init(|| {
        let r = |id, p: &str| Rule { id, re: Regex::new(p).expect("valid secret rule") };
        vec![
            r("private-key", r"-----BEGIN (?:[A-Z0-9]+ )*PRIVATE KEY(?: BLOCK)?-----"),
            r("aws-access-key-id", r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b"),
            r("github-token", r"\b(?:ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9]{30,}\b|\bgithub_pat_[A-Za-z0-9_]{40,}\b"),
            r("gitlab-token", r"\bglpat-[A-Za-z0-9_\-]{20,}\b"),
            r("slack-token", r"\bxox[abprs]-[A-Za-z0-9-]{10,}\b"),
            r("anthropic-key", r"\bsk-ant-[A-Za-z0-9_\-]{20,}\b"),
            r("openai-key", r"\bsk-(?:proj-|svcacct-)?[A-Za-z0-9_\-]{20,}\b"),
            r("google-api-key", r"\bAIza[0-9A-Za-z_\-]{35}\b"),
            r("stripe-key", r"\b(?:sk|rk)_(?:live|test)_[0-9A-Za-z]{20,}\b"),
            r("jwt", r"\beyJ[A-Za-z0-9_-]{10,}\.eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\b"),
            r(
                "generic-assignment",
                r#"(?i)\b(?:api[_-]?key|secret|token|passw(?:or)?d|client[_-]?secret|access[_-]?key)\b["']?\s*[:=]\s*["']([A-Za-z0-9/+_\-.]{16,})["']"#,
            ),
        ]
    })
}

fn mask(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= 8 {
        return "*".repeat(chars.len());
    }
    let head: String = chars[..4].iter().collect();
    let tail: String = chars[chars.len() - 2..].iter().collect();
    format!("{head}…{tail}")
}

/// Scans text content; returns one finding per match.
pub fn scan_text(content: &str) -> Vec<Finding> {
    let mut out = Vec::new();
    for (i, line) in content.lines().enumerate() {
        // Rules are ordered specific-first; a span already claimed by an earlier rule is not reported twice
        // (e.g. `sk-ant-...` is an Anthropic key, not also an OpenAI key).
        let mut claimed: Vec<(usize, usize)> = Vec::new();
        for rule in rules() {
            for m in rule.re.find_iter(line) {
                // Placeholders in examples are not secrets.
                let s = m.as_str();
                if s.contains("EXAMPLE")
                    || s.contains("xxxxxxxx")
                    || s.contains("XXXXXXXX")
                    || s.contains("${")
                {
                    continue;
                }
                if claimed.iter().any(|&(a, b)| m.start() < b && a < m.end()) {
                    continue;
                }
                claimed.push((m.start(), m.end()));
                out.push(Finding {
                    rule: rule.id,
                    line: i + 1,
                    preview: mask(s),
                });
            }
        }
    }
    out
}

/// Replaces known secret values and secret-shaped tokens. Known values are registered from the secret store
/// at the moment they are read, so anything Agenium ever handed to a provider/MCP is masked in its own logs.
#[derive(Clone, Default)]
pub struct Redactor {
    known: Arc<RwLock<Vec<(String, String)>>>,
}

impl Redactor {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a secret value (values shorter than 8 characters are ignored: too likely to hit normal text).
    pub fn register(&self, name: &str, value: &str) {
        if value.chars().count() < 8 {
            return;
        }
        let mut k = self.known.write().unwrap_or_else(|p| p.into_inner());
        if !k.iter().any(|(_, v)| v == value) {
            k.push((name.to_owned(), value.to_owned()));
            // Longest first so a secret that contains another is masked whole.
            k.sort_by_key(|e| std::cmp::Reverse(e.1.len()));
        }
    }

    pub fn redact(&self, text: &str) -> String {
        let mut s = text.to_owned();
        for (name, v) in self.known.read().unwrap_or_else(|p| p.into_inner()).iter() {
            if s.contains(v.as_str()) {
                s = s.replace(v.as_str(), &format!("[REDACTED:{name}]"));
            }
        }
        for rule in rules() {
            if rule.id == "generic-assignment" {
                s = rule
                    .re
                    .replace_all(&s, |c: &regex::Captures<'_>| {
                        c[0].replace(&c[1], "[REDACTED]")
                    })
                    .into_owned();
            } else {
                s = rule
                    .re
                    .replace_all(&s, format!("[REDACTED:{}]", rule.id).as_str())
                    .into_owned();
            }
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_common_secret_shapes_and_skips_placeholders() {
        let text = "\
let k = \"sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123\";
AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE
token: ghp_abcdefghijklmnopqrstuvwxyz0123456789
-----BEGIN OPENSSH PRIVATE KEY-----
password = \"correcthorsebatterystaple\"
nothing here";
        let f = scan_text(text);
        let rules: Vec<_> = f.iter().map(|x| (x.rule, x.line)).collect();
        assert_eq!(
            rules,
            [
                ("anthropic-key", 1),
                ("github-token", 3),
                ("private-key", 4),
                ("generic-assignment", 5)
            ]
        );
        assert!(f.iter().all(|x| !x.preview.contains("abcdefghijklmnop")));
    }

    #[test]
    fn redacts_known_values_and_patterns() {
        let r = Redactor::new();
        r.register("provider/x", "my-very-secret-value");
        r.register("short", "abc");
        let out =
            r.redact("key=my-very-secret-value tok=ghp_abcdefghijklmnopqrstuvwxyz0123456789 abc");
        assert_eq!(
            out,
            "key=[REDACTED:provider/x] tok=[REDACTED:github-token] abc"
        );
        assert_eq!(
            r.redact("api_key = \"0123456789abcdef0123\""),
            "api_key = \"[REDACTED]\""
        );
    }
}
