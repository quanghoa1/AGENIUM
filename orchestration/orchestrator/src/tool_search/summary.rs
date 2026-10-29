//! What a server's tools found by search are about, in a few words for the run's capabilities line (ADR-139): the first
//! word of their names (`github 20, slack 11, …`), or — when the names say nothing (`tool_000`, `tool_001`, … or one
//! prefix on every tool) — the words their descriptions share most (`issue 14, message 9, …`).

use std::collections::{BTreeMap, BTreeSet};

/// Groups named at most.
const GROUPS: usize = 25;
/// Description words named at most.
const TOPICS: usize = 15;

/// Words of a description that say nothing of what a tool is about: common words and the actions (what the tools act
/// on is the point).
const PLAIN: &[&str] = &[
    "about", "after", "also", "back", "been", "before", "both", "each", "from", "given", "have",
    "into", "just", "more", "most", "none", "only", "other", "over", "same", "some", "such",
    "than", "that", "their", "them", "then", "there", "these", "they", "this", "those", "tool",
    "under", "when", "where", "which", "while", "with", "without", "your", "add", "change",
    "check", "close", "create", "delete", "edit", "fetch", "find", "list", "make", "open", "read",
    "remove", "return", "returns", "run", "search", "send", "show", "start", "stop", "update",
    "write",
];

/// `names`' groups, then `descriptions`' words when the names make fewer than two groups.
pub(super) fn summary(names: &[&str], descriptions: &[&str]) -> String {
    let mut count: BTreeMap<String, usize> = Default::default();
    for n in names {
        let first = n.split(['_', '-', '.']).next().unwrap_or(n).to_lowercase();
        *count.entry(first).or_default() += 1;
    }
    if count.values().filter(|c| **c > 1).count() < 2 {
        if let Some(t) = topics(descriptions) {
            return format!("about: {t}");
        }
    }
    let mut other = 0;
    let mut parts: Vec<String> = Vec::new();
    for (g, c) in &count {
        if *c > 1 && parts.len() < GROUPS {
            parts.push(format!("{g} {c}"));
        } else {
            other += c;
        }
    }
    if other > 0 {
        parts.push(format!("other {other}"));
    }
    parts.join(", ")
}

/// The words most descriptions share (a plural counted with its singular), each with how many tools it is in; words
/// in more than half of them (`list`, `repository` on a one-subject server) say nothing and are left out.
fn topics(descriptions: &[&str]) -> Option<String> {
    let mut df: BTreeMap<String, usize> = Default::default();
    for d in descriptions {
        let words: BTreeSet<String> = d
            .split(|c: char| !c.is_alphanumeric())
            .map(str::to_lowercase)
            .filter(|w| {
                w.len() >= 4 && w.chars().all(|c| c.is_alphabetic()) && !PLAIN.contains(&w.as_str())
            })
            .collect();
        for w in words {
            *df.entry(w).or_default() += 1;
        }
    }
    let plurals: Vec<String> = df
        .keys()
        .filter(|w| w.ends_with('s') && df.contains_key(&w[..w.len() - 1]))
        .cloned()
        .collect();
    for p in plurals {
        let c = df.remove(&p).unwrap_or(0);
        *df.entry(p[..p.len() - 1].to_owned()).or_default() += c;
    }
    let mut ranked: Vec<(String, usize)> = df
        .into_iter()
        .filter(|(_, c)| *c > 1 && *c * 2 <= descriptions.len())
        .collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    if ranked.is_empty() {
        return None;
    }
    Some(
        ranked
            .into_iter()
            .take(TOPICS)
            .map(|(w, c)| format!("{w} {c}"))
            .collect::<Vec<_>>()
            .join(", "),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_that_say_something_are_grouped() {
        let names = [
            "github_create_issue",
            "github_list_issues",
            "slack_post",
            "slack_read",
            "sentry_get",
        ];
        assert_eq!(summary(&names, &[""; 5]), "github 2, slack 2, other 1");
    }

    #[test]
    fn names_that_say_nothing_are_summed_up_by_their_descriptions() {
        let names = ["tool_000", "tool_001", "tool_002", "tool_003", "tool_004"];
        let descriptions = [
            "Create an issue in a repository.",
            "List issues of a repository.",
            "Post a message to a Slack channel.",
            "Read the messages of a channel.",
            "Get one error event with its stack trace.",
        ];
        assert_eq!(
            summary(&names, &descriptions),
            "about: channel 2, issue 2, message 2, repository 2"
        );
        // Nothing shared: the names' one group.
        assert_eq!(
            summary(&names[..2], &["Alpha thing.", "Beta stuff."]),
            "tool 2"
        );
    }
}
