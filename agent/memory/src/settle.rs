//! Settling the memory (ADR-145): live facts that drifted into saying nearly the same thing are found by their
//! vectors (the local embedding model, no tokens), folded when they say the same, and put to the user when they
//! contradict each other. Writing a fact already reconciles it with the memories it was shown next to (the UPDATE /
//! DELETE of the pipeline); this catches what slipped between episodes — a paraphrase, a figure that changed.
//!
//! Per pair of facts of the same kind, scope, project and trust, by cosine similarity:
//! - at least [`SAME`] with the same figures (numbers, versions) and the same negation: the same fact, folded;
//! - at least [`SAME`] with other figures or a negation on one side only ("port 8080" / "port 8081"): a conflict — the
//!   judge says which when it is on, else the user decides;
//! - between [`RELATED`] and [`SAME`]: the judge, when on, says same / contradicting / both hold; without it, left.
//!
//! The judge is asked at most `max_questions` times a pass, each pair once (`fact_pair`). Nothing is learned here: a
//! fact is only folded into one that says the same, and a contradiction is never settled without the user.

use std::collections::HashMap;

use agenium_store::agent::{Fact, PairVerdict};
use agenium_store::AgentStore;
use async_trait::async_trait;
use serde::Serialize;

use crate::MemoryError;

/// Cosine similarity from which two facts with the same figures and names read as the same (bge-small-en-v1.5).
/// Measured on 70 labelled pairs (ADR-145): no wrong fold down to 0.90; at 0.93, 10 of 20 folded without the judge.
pub const SAME: f64 = 0.93;
/// Cosine similarity from which two facts are about the same thing and may be the same or contradict. Measured on 70
/// labelled pairs (ADR-145): at 0.85 the judge found 13 of 20 contradictions, at 0.75 18 of 20 for 42 questions
/// (unrelated pairs read at most 0.59).
pub const RELATED: f64 = 0.75;
/// Facts of one group compared at most (the newest): the pass stays quadratic in a bounded number.
const GROUP_MAX: usize = 3000;

/// What the judge says of two facts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Verdict {
    Same,
    Conflict,
    Distinct,
}

/// Reads two facts and says whether they are the same, contradict each other or both hold; `None` when unsure, off
/// or failing (the pair is then left for a later pass).
#[async_trait]
pub trait PairJudge: Send + Sync {
    async fn judge(&self, a: &Fact, b: &Fact) -> Option<Verdict>;
}

#[derive(Debug, Clone, Copy)]
pub struct SettleOptions {
    /// Judge questions in one pass at most.
    pub max_questions: usize,
    /// Report what would be done without changing the memory.
    pub dry_run: bool,
    /// [`SAME`] and [`RELATED`] (other values to measure them).
    pub same: f64,
    pub related: f64,
}

impl Default for SettleOptions {
    fn default() -> Self {
        Self {
            max_questions: 20,
            dry_run: false,
            same: SAME,
            related: RELATED,
        }
    }
}

/// One pair the pass acted on.
#[derive(Debug, Clone, Serialize)]
pub struct Settled {
    /// The fact kept (a fold) or the first of a conflict.
    pub a: i64,
    /// The fact folded into `a`, or the second of a conflict.
    pub b: i64,
    pub similarity: f64,
    pub verdict: PairVerdict,
    /// `figures` (same words, other numbers), `vectors` (same words and figures), `judge`.
    pub by: &'static str,
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct SettleReport {
    /// Live facts compared.
    pub facts: usize,
    /// Pairs at least [`RELATED`] alike.
    pub related: usize,
    /// Pairs judged in an earlier pass, left.
    pub known: usize,
    /// Judge questions asked.
    pub asked: usize,
    /// Related pairs left for want of a judge or of questions.
    pub left: usize,
    pub done: Vec<Settled>,
}

impl SettleReport {
    pub fn folded(&self) -> usize {
        self.done
            .iter()
            .filter(|d| d.verdict == PairVerdict::Same)
            .count()
    }

    pub fn conflicts(&self) -> usize {
        self.done
            .iter()
            .filter(|d| d.verdict == PairVerdict::Conflict)
            .count()
    }
}

/// The figures of a text (digit groups: `8080`, `3`, `12` of `3.12`) and whether it says no.
fn figures(text: &str) -> (Vec<String>, bool) {
    let lower = text.to_lowercase();
    let mut groups = Vec::new();
    let mut negated = false;
    for w in lower.split(|c: char| !c.is_alphanumeric() && c != '\'') {
        negated |= NEGATIONS.contains(&w);
        if let Some(n) = NUMBER_WORDS.iter().position(|x| *x == w) {
            groups.push(n.to_string());
            continue;
        }
        if w == "once" || w == "twice" {
            groups.push(if w == "once" { "1" } else { "2" }.to_owned());
            continue;
        }
        // The digit runs of a word ("8080", "v1" → "1", "py312" → "312").
        let mut cur = String::new();
        for c in w.chars() {
            if c.is_ascii_digit() {
                cur.push(c);
            } else if !cur.is_empty() {
                groups.push(std::mem::take(&mut cur));
            }
        }
        if !cur.is_empty() {
            groups.push(cur);
        }
    }
    // The same figures in another order are the same figures ("port 80, version 2" / "version 2 on port 80").
    groups.sort();
    (groups, negated)
}

/// The words of a text, without the punctuation around them (`staging.shop.example.com` stays whole).
fn words(text: &str) -> impl Iterator<Item = &str> {
    text.split_whitespace()
        .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()))
        .filter(|w| !w.is_empty())
}

/// Whether every name of one text — a word holding `.`, `/`, `_` or `-` (a host, a path, an identifier) or a
/// capitalised word past the first (a product, a language: GitHub, GitLab, English) — is a word of the other too.
/// Measured (ADR-145): "staging at staging.shop.example.com" / "staging at stage.shop.example.net" read 0.943 alike.
fn names_shared(a: &str, b: &str) -> bool {
    let names = |t: &str| -> Vec<String> {
        words(t)
            .enumerate()
            .filter(|(i, w)| {
                w.contains(['.', '/', '_', '-'])
                    || *i > 0 && w.chars().next().is_some_and(char::is_uppercase)
            })
            .map(|(_, w)| w.to_lowercase())
            .collect()
    };
    let has = |t: &str, n: &str| words(t).any(|w| w.eq_ignore_ascii_case(n));
    names(a).iter().all(|n| has(b, n)) && names(b).iter().all(|n| has(a, n))
}

/// Whether two texts use nearly the same words, figures aside (three in four words shared): "The dev server listens
/// on port 8080." / "The dev server listens on port 9090.".
fn same_words(a: &str, b: &str) -> bool {
    let set = |t: &str| -> std::collections::BTreeSet<String> {
        words(t)
            .map(|w| w.to_lowercase())
            // Figures and negations are compared on their own (`figures`).
            .filter(|w| {
                !w.chars().any(|c| c.is_ascii_digit())
                    && !NUMBER_WORDS.contains(&w.as_str())
                    && !NEGATIONS.contains(&w.as_str())
            })
            .collect()
    };
    let (x, y) = (set(a), set(b));
    let union = x.union(&y).count();
    union > 0 && x.intersection(&y).count() * 4 >= union * 3
}

/// Numbers written as words, by their value.
const NUMBER_WORDS: &[&str] = &[
    "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten",
    "eleven", "twelve",
];

const NEGATIONS: &[&str] = &[
    "not",
    "no",
    "never",
    "don't",
    "doesn't",
    "isn't",
    "aren't",
    "wasn't",
    "won't",
    "can't",
    "cannot",
    "shouldn't",
    "mustn't",
    "without",
    "nor",
    "neither",
];

fn cosine(a: &[f32], b: &[f32]) -> f64 {
    let (mut dot, mut na, mut nb) = (0.0f64, 0.0f64, 0.0f64);
    for (x, y) in a.iter().zip(b) {
        dot += *x as f64 * *y as f64;
        na += (*x as f64).powi(2);
        nb += (*y as f64).powi(2);
    }
    if na > 0.0 && nb > 0.0 {
        dot / (na.sqrt() * nb.sqrt())
    } else {
        0.0
    }
}

/// Which of two facts that say the same stays: the more confident, then the one validated in more contexts, then the
/// newer.
fn keeper<'a>(a: &'a Fact, b: &'a Fact) -> (&'a Fact, &'a Fact) {
    let rank = |f: &Fact| (f.confidence, f.validated.len(), f.id);
    if rank(a).partial_cmp(&rank(b)) == Some(std::cmp::Ordering::Less) {
        (b, a)
    } else {
        (a, b)
    }
}

/// One settling pass over `store`'s live facts.
pub async fn settle(
    store: &AgentStore,
    judge: Option<&dyn PairJudge>,
    opts: SettleOptions,
) -> Result<SettleReport, MemoryError> {
    let mut report = SettleReport::default();
    let facts = store.live(usize::MAX >> 1)?;
    report.facts = facts.len();
    let ids: Vec<i64> = facts.iter().map(|f| f.id).collect();
    let vectors = store.vectors_of(&ids)?;
    // Same kind, scope, project and trust (newest first, as `live` returns them).
    let mut groups: HashMap<(String, String, Option<String>, bool), Vec<&Fact>> = HashMap::new();
    for f in facts.iter().filter(|f| vectors.contains_key(&f.id)) {
        let g = groups
            .entry((
                f.kind.as_str().to_owned(),
                f.scope.to_string(),
                f.project.clone(),
                f.untrusted,
            ))
            .or_default();
        if g.len() < GROUP_MAX {
            g.push(f);
        }
    }
    let mut pairs: Vec<(f64, &Fact, &Fact)> = Vec::new();
    for g in groups.values() {
        for (i, a) in g.iter().enumerate() {
            for b in &g[i + 1..] {
                let s = cosine(&vectors[&a.id], &vectors[&b.id]);
                if s >= opts.related {
                    pairs.push((s, a, b));
                }
            }
        }
    }
    pairs.sort_by(|x, y| y.0.total_cmp(&x.0).then(x.1.id.cmp(&y.1.id)));
    report.related = pairs.len();
    let mut gone: std::collections::HashSet<i64> = Default::default();
    for (s, a, b) in pairs {
        if gone.contains(&a.id) || gone.contains(&b.id) {
            continue;
        }
        if store.pair_verdict(a.id, b.id)?.is_some() {
            report.known += 1;
            continue;
        }
        let same_figures = figures(&a.text) == figures(&b.text) && names_shared(&a.text, &b.text);
        let verdict = if s >= opts.same && same_figures {
            Some((PairVerdict::Same, "vectors"))
        } else if let Some(j) = judge.filter(|_| report.asked < opts.max_questions) {
            report.asked += 1;
            match j.judge(a, b).await {
                Some(Verdict::Same) => Some((PairVerdict::Same, "judge")),
                Some(Verdict::Conflict) => Some((PairVerdict::Conflict, "judge")),
                Some(Verdict::Distinct) => Some((PairVerdict::Distinct, "judge")),
                None => None,
            }
        } else if !same_figures && (s >= opts.same || same_words(&a.text, &b.text)) {
            // The same words with other figures, other names or one side saying no: for the user. The vectors read
            // "port 8080" and "port 9090" only 0.89 alike, so the words are compared too.
            Some((PairVerdict::Conflict, "figures"))
        } else {
            None
        };
        let Some((verdict, by)) = verdict else {
            report.left += 1;
            continue;
        };
        let (keep, other) = keeper(a, b);
        let (first, second) = if verdict == PairVerdict::Same {
            (keep.id, other.id)
        } else {
            (a.id, b.id)
        };
        if !opts.dry_run {
            if verdict == PairVerdict::Same {
                store.fold(first, second)?;
            }
            store.set_pair(first, second, verdict, s, by)?;
        }
        if verdict == PairVerdict::Same {
            gone.insert(second);
        }
        report.done.push(Settled {
            a: first,
            b: second,
            similarity: s,
            verdict,
            by,
        });
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn figures_and_negations_tell_facts_apart() {
        assert_ne!(
            figures("The API listens on port 8080."),
            figures("The API listens on port 8081.")
        );
        assert_eq!(
            figures("Python 3.12 is used."),
            figures("The project uses Python 3.12")
        );
        assert_ne!(
            figures("Run the tests with cargo nextest."),
            figures("Do not run the tests with cargo nextest.")
        );
        assert_ne!(
            figures("Tests can run in parallel."),
            figures("Tests can't run in parallel.")
        );
        assert_eq!(
            figures("Notify the team."),
            figures("Notify the team on release.")
        );
        assert_eq!(
            figures("Retries a failed webhook three times."),
            figures("A failed webhook is retried 3 times.")
        );
        assert_eq!(
            figures("Port 80, version 2."),
            figures("Version 2 on port 80.")
        );
        assert_ne!(figures("Retried once."), figures("Retried twice."));
    }

    #[test]
    fn the_same_words_are_told_figures_aside() {
        assert!(same_words(
            "The dev server listens on port 8080.",
            "The dev server listens on port 9090."
        ));
        assert!(same_words(
            "Run the tests in parallel.",
            "Do not run the tests in parallel."
        ));
        assert!(!same_words(
            "Use pnpm to install dependencies.",
            "Dependencies are installed with pnpm."
        ));
        assert!(!same_words(
            "The logs are kept for 30 days.",
            "The audit trail is kept for 7 years."
        ));
    }

    #[test]
    fn names_on_one_side_only_keep_facts_apart() {
        assert!(!names_shared(
            "The staging environment is at staging.shop.example.com.",
            "The staging environment is at stage.shop.example.net."
        ));
        assert!(!names_shared(
            "The CI runs on GitHub Actions.",
            "The CI runs on GitLab CI."
        ));
        assert!(!names_shared(
            "Commit messages are in English.",
            "Commit messages are in Vietnamese."
        ));
        assert!(names_shared(
            "The frontend is built with Vite.",
            "Vite builds the frontend."
        ));
        assert!(names_shared(
            "Images are in the bucket shop-media.",
            "The shop-media bucket holds the images."
        ));
        assert!(names_shared("Run the tests.", "Tests are run with care."));
    }
}
