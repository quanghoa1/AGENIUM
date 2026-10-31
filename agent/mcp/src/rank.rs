//! Ranking tools against a request (ADR-139): BM25 over each tool's name, description and arguments, and reciprocal
//! rank fusion to join it with other rankings (vectors, the judge). As the tool search of Anthropic's API, a tool is
//! found by its name, its description, its argument names and their descriptions; unlike a plain BM25, common words
//! are left out (a "the" or an "on" pulled a database tool above Slack's for "notify the team on slack") and words
//! are cut to a stem ("errors" finds "error"), measured in the ADR's examples.

use std::collections::HashMap;

use serde_json::Value;

/// English words that say nothing about what a tool does.
const STOP: &[&str] = &[
    "a", "about", "after", "all", "an", "and", "any", "are", "as", "at", "be", "by", "can", "do",
    "does", "for", "from", "get", "has", "have", "how", "i", "if", "in", "into", "is", "it", "its",
    "me", "my", "need", "of", "on", "or", "our", "should", "so", "some", "that", "the", "their",
    "them", "then", "there", "this", "to", "tool", "use", "want", "was", "we", "what", "when",
    "which", "who", "why", "will", "with", "you", "your",
];

/// A word cut to a rough stem: plural and verb endings off ("issues" → "issu", "issue" → "issu").
fn stem(w: &str) -> String {
    let w = w.to_lowercase();
    for (suffix, min) in [
        ("ing", 5),
        ("ies", 4),
        ("es", 4),
        ("ed", 4),
        ("s", 3),
        ("e", 4),
    ] {
        if w.len() >= min + suffix.len() - 1 && w.ends_with(suffix) && !w.ends_with("ss") {
            let base = &w[..w.len() - suffix.len()];
            return if suffix == "ies" {
                format!("{base}y")
            } else {
                base.to_owned()
            };
        }
    }
    w
}

/// The words of a text: split at anything not a letter or digit and at camelCase, common words left out, stemmed.
pub fn words(s: &str) -> Vec<String> {
    let mut spaced = String::with_capacity(s.len() + 8);
    let mut prev_lower = false;
    for c in s.chars() {
        if c.is_uppercase() && prev_lower {
            spaced.push(' ');
        }
        prev_lower = c.is_lowercase() || c.is_ascii_digit();
        spaced.push(if c.is_alphanumeric() { c } else { ' ' });
    }
    spaced
        .split_whitespace()
        .map(str::to_lowercase)
        .filter(|w| !STOP.contains(&w.as_str()))
        .map(|w| stem(&w))
        .collect()
}

/// What a tool is found by: its name (twice: a word of the name says more than one of the description), its
/// description, and the names and descriptions of its arguments (one level of nesting).
pub fn tool_text(name: &str, description: &str, schema: &Value) -> String {
    let mut s = format!("{name} {name} {description}");
    let mut args = |props: &Value| {
        for (k, v) in props.as_object().into_iter().flatten() {
            s.push(' ');
            s.push_str(k);
            if let Some(d) = v["description"].as_str() {
                s.push(' ');
                s.push_str(d);
            }
            for e in v["enum"].as_array().into_iter().flatten() {
                if let Some(e) = e.as_str() {
                    s.push(' ');
                    s.push_str(e);
                }
            }
        }
    };
    args(&schema["properties"]);
    for v in schema["properties"].as_object().into_iter().flatten() {
        args(&v.1["properties"]);
        args(&v.1["items"]["properties"]);
    }
    s
}

/// Characters of a description its meaning is taken from.
const MEANING_CHARS: usize = 300;

/// What a tool's meaning (its embedding) is taken from: its name and the start of its description, cut at a word.
/// Real servers' descriptions run long (Sentry's hold examples and "use this tool when" lists) and dilute a single
/// vector; the arguments, which the BM25 text keeps, did the same. Measured on 330 real tools (ADR-139).
pub fn meaning_text(name: &str, description: &str) -> String {
    let d = description.split_whitespace().collect::<Vec<_>>().join(" ");
    let d = match d.char_indices().nth(MEANING_CHARS) {
        None => d,
        Some((at, _)) => d[..at]
            .rsplit_once(' ')
            .map_or(&d[..at], |(a, _)| a)
            .to_owned(),
    };
    format!("{name}: {d}")
}

/// Okapi BM25 over a fixed set of texts.
pub struct Bm25 {
    docs: Vec<HashMap<String, u32>>,
    lens: Vec<f64>,
    df: HashMap<String, u32>,
    avg: f64,
}

const K1: f64 = 1.2;
const B: f64 = 0.75;

impl Bm25 {
    pub fn new(texts: &[String]) -> Bm25 {
        let mut docs = Vec::with_capacity(texts.len());
        let mut lens = Vec::with_capacity(texts.len());
        let mut df: HashMap<String, u32> = HashMap::new();
        for t in texts {
            let w = words(t);
            lens.push(w.len() as f64);
            let mut tf: HashMap<String, u32> = HashMap::new();
            for x in w {
                *tf.entry(x).or_default() += 1;
            }
            for k in tf.keys() {
                *df.entry(k.clone()).or_default() += 1;
            }
            docs.push(tf);
        }
        let avg = (lens.iter().sum::<f64>() / lens.len().max(1) as f64).max(1.0);
        Bm25 {
            docs,
            lens,
            df,
            avg,
        }
    }

    /// Each text's score for `query` (0 when no word of the query is in it).
    pub fn scores(&self, query: &str) -> Vec<f64> {
        let mut q = words(query);
        q.sort();
        q.dedup();
        let n = self.docs.len() as f64;
        self.docs
            .iter()
            .zip(&self.lens)
            .map(|(tf, len)| {
                q.iter()
                    .filter_map(|t| {
                        let f = *tf.get(t)? as f64;
                        let df = *self.df.get(t)? as f64;
                        let idf = (1.0 + (n - df + 0.5) / (df + 0.5)).ln();
                        Some(idf * f * (K1 + 1.0) / (f + K1 * (1.0 - B + B * len / self.avg)))
                    })
                    .sum()
            })
            .collect()
    }
}

/// The indexes of `scores` above zero, best first.
pub fn order(scores: &[f64]) -> Vec<usize> {
    let mut v: Vec<usize> = (0..scores.len()).filter(|i| scores[*i] > 0.0).collect();
    v.sort_by(|a, b| scores[*b].total_cmp(&scores[*a]).then(a.cmp(b)));
    v
}

/// Reciprocal rank fusion of weighted rankings (each best first): an item's score is the sum of
/// `weight / (k + rank)`.
pub fn fuse(rankings: &[(&[usize], f64)], k: f64) -> Vec<(usize, f64)> {
    let mut score: HashMap<usize, f64> = HashMap::new();
    for (r, weight) in rankings {
        for (rank, i) in r.iter().enumerate() {
            *score.entry(*i).or_default() += weight / (k + rank as f64 + 1.0);
        }
    }
    let mut v: Vec<(usize, f64)> = score.into_iter().collect();
    v.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn catalogue() -> Vec<(&'static str, &'static str, Value)> {
        vec![
            (
                "github_create_issue",
                "Create a new issue in a GitHub repository",
                json!({"properties": {"title": {"description": "Issue title"}}}),
            ),
            (
                "github_list_issues",
                "List issues of a repository",
                json!({}),
            ),
            (
                "slack_send_message",
                "Post a message to a Slack channel",
                json!({"properties": {"channel": {}, "text": {"description": "message text"}}}),
            ),
            (
                "slack_list_channels",
                "List the Slack channels the bot can see",
                json!({}),
            ),
            (
                "postgres_query",
                "Run a read-only SQL query on the database",
                json!({}),
            ),
            (
                "sentry_get_event",
                "Get the stack trace and details of one error event",
                json!({}),
            ),
            (
                "jira_create_ticket",
                "Create a Jira ticket in a project",
                json!({}),
            ),
        ]
    }

    fn top(q: &str) -> Vec<&'static str> {
        let c = catalogue();
        let texts: Vec<String> = c.iter().map(|(n, d, s)| tool_text(n, d, s)).collect();
        order(&Bm25::new(&texts).scores(q))
            .into_iter()
            .map(|i| c[i].0)
            .collect()
    }

    #[test]
    fn common_words_and_endings_do_not_decide_a_search() {
        assert_eq!(top("create an issue on github")[0], "github_create_issue");
        // "on" and "the" no longer bring the database tool in.
        assert!(!top("notify the team on slack").contains(&"postgres_query"));
        assert_eq!(top("notify the team on slack").len(), 2);
        // "errors" finds the tool that says "error".
        assert_eq!(
            top("why is production throwing errors"),
            ["sentry_get_event"]
        );
        // camelCase and snake_case names are words.
        assert_eq!(
            words("createIssue list_channels"),
            ["creat", "issu", "list", "channel"]
        );
        assert!(top("the a of").is_empty());
    }

    #[test]
    fn meaning_is_taken_from_the_name_and_the_start_of_the_description() {
        assert_eq!(
            meaning_text("github create_issue", "Create  an\nissue."),
            "github create_issue: Create an issue."
        );
        let long = "word ".repeat(100);
        let t = meaning_text("n", &long);
        assert!(
            t.len() <= "n: ".len() + MEANING_CHARS && t.ends_with("word"),
            "{t}"
        );
    }

    #[test]
    fn rankings_fuse_by_reciprocal_rank() {
        // 1 is second and third: ahead of 0, second in one ranking only.
        let f = fuse(&[(&[2, 0, 1], 1.0), (&[2, 1], 1.0)], 60.0);
        assert_eq!(f.iter().map(|x| x.0).collect::<Vec<_>>(), [2, 1, 0]);
        // A heavier ranking decides between two items each first in one ranking.
        let f = fuse(&[(&[0], 1.0), (&[1], 1.5)], 10.0);
        assert_eq!(f.iter().map(|x| x.0).collect::<Vec<_>>(), [1, 0]);
    }
}
