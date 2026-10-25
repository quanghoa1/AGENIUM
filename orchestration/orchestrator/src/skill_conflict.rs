//! Keeping skills from contradicting each other (ADR-92). Contradictions mostly come from two skills about the same
//! subject that drift apart, so the main defence is structural — **one subject, one skill**: a new skill whose subject
//! (description + Job) is too close to an existing skill's must improve that skill instead (`propose_skill_update`,
//! an explicit, reviewable diff of one skill) rather than live beside it. A second, narrow layer catches **explicit
//! opposite directives** between different skills ("always X" against "never X" / "do not X" / "avoid X"). Both are
//! deterministic and run with or without the judge; with the judge on, a semantic contradiction read is added on top
//! (`App::skill_contradiction`, ADR-92).

use std::collections::BTreeSet;

use agenium_memory::{Embedder, HashEmbedder};

/// Same subject by content words (hashing embedder over [`subject_words`], always computed). Calibrated with
/// `agenium-eval --example skill_similarity` on the 12 library skills against 6 paraphrased and 7 different candidates:
/// different subjects scored at most 0.211; 0.40 catches 2/6 paraphrases (0.456, 0.673) with no false positive. A
/// precise, low-recall backstop — the meaning score and the other layers do most of the work.
pub const SAME_SUBJECT_WORDS: f32 = 0.40;

/// Same subject by meaning (the local bge-small model, when installed). Same calibration: different subjects scored at
/// most 0.714 (a changelog skill against commit-and-pr); 0.77 catches 4/6 paraphrases (0.799–0.859) with no false
/// positive and a 0.056 margin. It misses close paraphrases that score 0.69–0.74 (stated in ADR-92).
pub const SAME_SUBJECT_MEANING: f32 = 0.77;

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let d: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    d / (na * nb).max(1e-9)
}

/// The subject's content words for the lexical signal: generic verbs and function words removed, so two short
/// subjects do not look alike just because both "write" something for a "change".
pub fn subject_words(subject: &str) -> String {
    const GENERIC: [&str; 52] = [
        "the", "a", "an", "and", "or", "of", "to", "in", "on", "for", "by", "with", "from", "as",
        "at", "it", "its", "is", "be", "so", "that", "this", "when", "into", "one", "every",
        "each", "your", "you", "can", "would", "write", "make", "add", "change", "check", "update",
        "use", "fix", "run", "find", "read", "show", "give", "keep", "take", "get", "set", "work",
        "new", "before", "after",
    ];
    subject
        .split(|c: char| !c.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|w| w.len() > 1 && !GENERIC.contains(&w.as_str()))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Per existing skill: (name, similarity by words, similarity by meaning when `semantic` is a real model). The scores
/// the one-subject gate uses — also what `agenium-eval --example skill_similarity` prints for calibration.
pub fn subject_scores(
    subject: &str,
    others: &[(String, String)],
    semantic: Option<&dyn Embedder>,
) -> Vec<(String, f32, Option<f32>)> {
    if others.is_empty() {
        return Vec::new();
    }
    let embed_all = |e: &dyn Embedder, f: &dyn Fn(&str) -> String| -> Option<Vec<f32>> {
        let texts: Vec<String> = std::iter::once(f(subject))
            .chain(others.iter().map(|(_, s)| f(s)))
            .collect();
        let vs = e.embed(&texts).ok()?;
        let (me, rest) = vs.split_first()?;
        Some(rest.iter().map(|v| cosine(me, v)).collect())
    };
    let words = embed_all(&HashEmbedder::new(384), &subject_words).unwrap_or_default();
    let meaning = semantic
        .filter(|e| !e.model_id().starts_with("hash"))
        .and_then(|e| embed_all(e, &|s: &str| s.to_owned()));
    others
        .iter()
        .enumerate()
        .map(|(i, (name, _))| {
            (
                name.clone(),
                words.get(i).copied().unwrap_or(0.0),
                meaning.as_ref().and_then(|m| m.get(i).copied()),
            )
        })
        .collect()
}

/// The existing skill whose subject is close enough to `subject` to be the same subject: by words (always) or by
/// meaning (when `semantic` is a real model). Returns the closest such (name, score, "words" | "meaning").
pub fn same_subject(
    subject: &str,
    others: &[(String, String)],
    semantic: Option<&dyn Embedder>,
) -> Option<(String, f32, &'static str)> {
    let mut best: Option<(String, f32, &'static str)> = None;
    for (name, w, m) in subject_scores(subject, others, semantic) {
        for (score, threshold, how) in [
            (Some(w), SAME_SUBJECT_WORDS, "words"),
            (m, SAME_SUBJECT_MEANING, "meaning"),
        ] {
            if let Some(s) = score.filter(|s| *s >= threshold) {
                if best.as_ref().is_none_or(|b| s > b.1) {
                    best = Some((name.clone(), s, how));
                }
            }
        }
    }
    best
}

/// A skill's guidance for the conflict check: its body plus its `references/*.md`, where many of its rules live.
pub fn skill_guidance(s: &agenium_skills::Skill) -> String {
    let mut text = s.body.clone();
    for f in s
        .files
        .iter()
        .filter(|f| f.starts_with("references/") && f.ends_with(".md"))
    {
        if let Ok(t) = std::fs::read_to_string(s.meta.dir.join(f)) {
            text.push('\n');
            text.push_str(&t);
        }
    }
    text
}

/// The sentence-level directives of a skill body: (positive?, content stems of the clause, the sentence).
pub fn directives(body: &str) -> Vec<(bool, BTreeSet<String>, String)> {
    const NEG: [&str; 7] = [
        "must not ",
        "should not ",
        "do not ",
        "don't ",
        "never ",
        "avoid ",
        "no need to ",
    ];
    const POS: [&str; 5] = ["always ", "must ", "make sure ", "ensure ", "be sure to "];
    let mut out = Vec::new();
    for line in body.lines() {
        for sentence in line.split(['.', ';']) {
            let s = sentence.trim();
            let low = format!(" {} ", s.to_lowercase());
            // A negative marker wins over a positive one in the same sentence ("must not" contains "must").
            let find = |markers: &[&str], positive: bool| {
                markers
                    .iter()
                    .filter_map(|m| {
                        low.find(&format!(" {m}"))
                            .map(|i| (positive, i + m.len() + 1))
                    })
                    .min_by_key(|(_, i)| *i)
            };
            let hit = find(&NEG, false).or_else(|| find(&POS, true));
            if let Some((pos, at)) = hit {
                let stems = stems(&low[at.min(low.len())..]);
                if stems.len() >= 2 {
                    out.push((pos, stems, s.to_owned()));
                }
            }
        }
    }
    out
}

/// Content words of a clause, lightly stemmed so "disable" and "disabling" meet.
fn stems(text: &str) -> BTreeSet<String> {
    const STOP: [&str; 30] = [
        "that", "this", "with", "from", "into", "your", "their", "them", "they", "then", "than",
        "when", "what", "which", "will", "would", "should", "could", "have", "been", "were",
        "there", "here", "only", "also", "just", "very", "each", "every", "those",
    ];
    text.split(|c: char| !c.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|w| w.len() >= 4 && !STOP.contains(&w.as_str()))
        .map(|w| {
            let mut w = w;
            for suf in ["ing", "ed", "es", "e", "s"] {
                if w.len() > suf.len() + 3 && w.ends_with(suf) {
                    w.truncate(w.len() - suf.len());
                    break;
                }
            }
            w
        })
        .collect()
}

/// Explicit opposite directives between `body` and the other skills (name, guidance): a positive directive in one and
/// a negative one in the other about the same thing (at least two shared content stems covering a third of the shorter
/// clause). Returns one human-readable note per conflict. High precision by design (the 12 library skills, references
/// included, produce none): it misses paraphrased contradictions, which the subject gate, the judge (when on) and the
/// anti-drift flag (ADR-91) are for.
pub fn directive_conflicts(name: &str, body: &str, others: &[(String, String)]) -> Vec<String> {
    let mine = directives(body);
    let mut out = Vec::new();
    for (other, theirs) in others {
        if other == name {
            continue;
        }
        for (p2, s2, line2) in directives(theirs) {
            for (p1, s1, line1) in &mine {
                if *p1 == p2 {
                    continue;
                }
                let shared = s1.intersection(&s2).count();
                let small = s1.len().min(s2.len()).max(1);
                if shared >= 2 && shared * 3 >= small {
                    out.push(format!(
                        "contradicts skill {other}: \"{line1}\" vs \"{line2}\""
                    ));
                }
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// The subject of a skill for the one-subject gate: its description and its Job section.
pub fn subject(description: &str, body: &str) -> String {
    let mut on = false;
    let mut job = String::new();
    for l in body.lines() {
        if let Some(h) = l.strip_prefix("## ") {
            on = h.trim().eq_ignore_ascii_case("job");
            continue;
        }
        if on {
            job.push_str(l);
            job.push(' ');
        }
    }
    format!("{} {}", description.trim(), job.trim())
}

/// An existing skill as the conflict checks see it.
pub struct Known {
    pub name: String,
    /// Description + Job (for the one-subject gate).
    pub subject: String,
    /// Body + references (for the directive check and the judge).
    pub guidance: String,
}

/// The `n` known skills whose subject is closest to `subject` by words, closest first (what the judge is shown).
pub fn closest<'a>(subject: &str, known: &'a [Known], n: usize) -> Vec<&'a Known> {
    let texts: Vec<String> = std::iter::once(subject.to_owned())
        .chain(known.iter().map(|k| k.subject.clone()))
        .collect();
    let Ok(vs) = HashEmbedder::new(384).embed(&texts) else {
        return Vec::new();
    };
    let Some((me, rest)) = vs.split_first() else {
        return Vec::new();
    };
    let mut scored: Vec<(f32, &Known)> = known
        .iter()
        .zip(rest)
        .map(|(k, v)| (cosine(me, v), k))
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    scored.into_iter().take(n).map(|(_, k)| k).collect()
}

impl crate::app::App {
    /// What a new or changed skill is checked against: the skills `agent` uses (own and overlaid, role, enabled
    /// library) and the whole library, except `skip` (the skill being changed).
    pub fn known_skills(
        &self,
        agent: Option<&agenium_store::core::agents::Agent>,
        skip: &str,
    ) -> Vec<Known> {
        let reg = self.skill_registry();
        let mut out: Vec<Known> = Vec::new();
        let mut push = |s: &agenium_skills::Skill| {
            if s.meta.name == skip || out.iter().any(|k| k.name == s.meta.name) {
                return;
            }
            out.push(Known {
                name: s.meta.name.clone(),
                subject: subject(&s.meta.description, &s.body),
                guidance: skill_guidance(s),
            });
        };
        if let Some(a) = agent {
            for s in &reg.for_agent(&a.id.to_string(), &a.role, &a.skills).skills {
                push(s);
            }
        }
        for s in &reg.library().skills {
            push(s);
        }
        out
    }

    /// With the judge on (`[judge].skill_quality`): the 3 skills closest to `subject` that it confidently reads as
    /// contradicting `text` (ADR-92). Empty when the judge is off or fails, so callers proceed on the deterministic
    /// gates alone.
    pub async fn judged_contradictions(
        &self,
        agent: agenium_core::AgentId,
        skip: &str,
        subject: &str,
        text: &str,
    ) -> Vec<String> {
        if !self.config().judge.skill_quality {
            return Vec::new();
        }
        let Ok(a) = self.agent(agent) else {
            return Vec::new();
        };
        let known = self.known_skills(Some(&a), skip);
        let related: Vec<(String, String)> = closest(subject, &known, 3)
            .into_iter()
            .map(|k| (k.name.clone(), k.guidance.clone()))
            .collect();
        self.skill_contradiction(text, &related)
            .await
            .unwrap_or_default()
    }

    /// Advisory for the user after importing a library skill: explicit opposite directives against the rest of the
    /// library (never blocks the import — the user decides).
    pub fn library_skill_conflicts(&self, name: &str) -> Vec<String> {
        let lib = self.skill_registry().library();
        let Some(s) = lib.get(name) else {
            return Vec::new();
        };
        let known = self.known_skills(None, name);
        let others: Vec<(String, String)> =
            known.into_iter().map(|k| (k.name, k.guidance)).collect();
        directive_conflicts(name, &skill_guidance(s), &others)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The library's guidance: each body plus its reference documents.
    fn library() -> Vec<(String, String)> {
        crate::skill_shape::LIBRARY
            .iter()
            .map(|b| {
                let mut text = crate::skill_shape::strip_frontmatter(b.skill_md);
                for (path, content) in b.files {
                    if path.starts_with("references/") {
                        text.push('\n');
                        text.push_str(content);
                    }
                }
                (b.name.to_owned(), text)
            })
            .collect()
    }

    #[test]
    fn a_paraphrased_subject_is_the_same_subject_by_words() {
        let lib: Vec<(String, String)> = crate::skill_shape::LIBRARY
            .iter()
            .map(|b| {
                let body = crate::skill_shape::strip_frontmatter(b.skill_md);
                let desc = b
                    .skill_md
                    .lines()
                    .find_map(|l| l.strip_prefix("description: "))
                    .unwrap_or("");
                (b.name.to_owned(), subject(desc, &body))
            })
            .collect();
        let same = subject(
            "Write tests that would catch real regressions in the code under change.",
            "## Job\nDesign tests that fail when the behaviour breaks.\n",
        );
        let hit = same_subject(&same, &lib, None).expect("same subject");
        assert_eq!(
            (hit.0.as_str(), hit.2),
            ("writing-tests", "words"),
            "{hit:?}"
        );
        let diff = subject(
            "Add translated UI strings for every supported language.",
            "## Job\nMake new UI text available in every language.\n",
        );
        assert!(same_subject(&diff, &lib, None).is_none());
    }

    #[test]
    fn the_built_in_library_does_not_contradict_itself() {
        let lib = library();
        for (name, body) in &lib {
            let c = directive_conflicts(name, body, &lib);
            assert!(c.is_empty(), "{name}: {c:?}");
        }
    }

    #[test]
    fn an_opposite_directive_is_caught() {
        let lib = library();
        let flaky = "## Job\nKeep the suite green.\n\n## Steps\n1. Always disable a flaky test so the suite stays green.\n";
        let c = directive_conflicts("green-suite", flaky, &lib);
        assert!(
            c.iter().any(|n| n.contains("root-cause-debugging")),
            "{c:?}"
        );
        let secrets =
            "## Steps\n1. Always log the full connection string so failures are easy to debug.\n";
        let c = directive_conflicts("debug-logging", secrets, &lib);
        assert!(c.iter().any(|n| n.contains("secure-coding")), "{c:?}");
    }

    #[test]
    fn unrelated_directives_are_not_conflicts() {
        let lib = library();
        let ok = "## Steps\n1. Always write the changelog entry under Added, Changed or Fixed.\n2. Never reorder past entries.\n";
        assert!(directive_conflicts("changelog-entry", ok, &lib).is_empty());
    }
}
