//! Calibration for the "one subject, one skill" gate (ADR-92). It calls the gate's own scorer
//! (`agenium_orchestrator::skill_conflict::subject_scores`) so the printed numbers are exactly what the gate compares to
//! its thresholds: similarity by words (content words, hashing embedder) and by meaning (the local bge-small model).
//! Candidates are labelled `same` (paraphrases an existing library skill's subject) or `diff`.
//!   cargo run --release -p agenium-eval --example skill_similarity -- [<models dir with bge-small-en-v1.5>]

use std::path::PathBuf;

use agenium_orchestrator::skill_conflict::{
    subject, subject_scores, SAME_SUBJECT_MEANING, SAME_SUBJECT_WORDS,
};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let lib: Vec<(String, String)> = agenium_orchestrator::skill_shape::LIBRARY
        .iter()
        .map(|b| {
            let desc = b
                .skill_md
                .lines()
                .find_map(|l| l.strip_prefix("description: "))
                .unwrap_or("");
            let body = b.skill_md.split("\n---\n").nth(1).unwrap_or("");
            (b.name.to_owned(), subject(desc, body))
        })
        .collect();
    let job = |j: &str| format!("## Job\n{j}\n");
    let cands: &[(&str, &str, &str, &str)] = &[
        (
            "same",
            "code-review",
            "Check a pull request for bugs and risky edges before approving it.",
            "Look over a pull request and decide if it is safe to merge.",
        ),
        (
            "same",
            "root-cause-debugging",
            "Track a defect to its real cause and fix it there with a test that fails first.",
            "Repair a defect at its source instead of patching the symptom.",
        ),
        (
            "same",
            "perf-audit",
            "Make slow code faster by profiling first and comparing timings before and after.",
            "Speed up a slow path without changing its results.",
        ),
        (
            "same",
            "migration-safety",
            "Change database tables so the change can be undone and does not lock production.",
            "Alter a database schema safely and reversibly.",
        ),
        (
            "same",
            "writing-tests",
            "Write tests that would catch real regressions in the code under change.",
            "Design tests that fail when the behaviour breaks.",
        ),
        (
            "same",
            "commit-and-pr",
            "Write a clear commit message and pull request description.",
            "Describe a change so a reviewer can follow it.",
        ),
        (
            "diff",
            "changelog",
            "Write the changelog entry for a merged change.",
            "Write the changelog entry for a merged change.",
        ),
        (
            "diff",
            "changelog-2",
            "Write the changelog entry for a merged change.",
            "Add one CHANGELOG entry under Added, Changed or Fixed.",
        ),
        (
            "diff",
            "dependency",
            "Upgrade a third-party dependency and check nothing breaks.",
            "Move a dependency to a newer version safely.",
        ),
        (
            "diff",
            "incident",
            "Triage a production incident: contain it, find the scope, keep people informed.",
            "Handle a live outage from alert to all-clear.",
        ),
        (
            "diff",
            "i18n",
            "Add translated UI strings for every supported language.",
            "Make new UI text available in every language.",
        ),
        (
            "diff",
            "logging",
            "Emit structured logs with request ids so incidents can be traced.",
            "Make log lines machine-readable and traceable.",
        ),
        (
            "diff",
            "backfill",
            "Backfill a large table in chunks without long locks.",
            "Fill a new column on a big table without holding a long lock.",
        ),
    ];
    let bge = args
        .get(1)
        .map(|m| agenium_memory::default_embedder(&PathBuf::from(m)));
    println!(
        "thresholds: words >= {SAME_SUBJECT_WORDS}, meaning >= {SAME_SUBJECT_MEANING}  (bge loaded: {})",
        bge.as_ref().is_some_and(|e| !e.model_id().starts_with("hash"))
    );
    for (kind, target, desc, jb) in cands {
        let mut s = subject_scores(&subject(desc, &job(jb)), &lib, bge.as_deref());
        let top_w = {
            s.sort_by(|a, b| b.1.total_cmp(&a.1));
            format!("{} {:.3}", s[0].0, s[0].1)
        };
        let top_m = {
            s.sort_by(|a, b| b.2.unwrap_or(0.0).total_cmp(&a.2.unwrap_or(0.0)));
            s[0].2
                .map(|m| format!("{} {:.3}", s[0].0, m))
                .unwrap_or("-".into())
        };
        println!("{kind:4} {target:22} words: {top_w:32} meaning: {top_m}");
    }
}
