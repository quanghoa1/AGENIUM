//! Answers kept by question (metajev's idea, `docs/reference/JEV-OPPORTUNITIES.md` §1): the key is a hash of the
//! endpoint and model, the question and the state entries it names, so the same question about the same text is answered once. It is
//! what makes a prefetch work: the done check asks the review's questions with its own (one request at submit), and the
//! review run then finds them here. Kept in `<root>/judge-answers.jsonl`, newest last, bounded.

use std::collections::{BTreeMap, HashMap};

use agenium_llm::systemone::{Answer, Judging, Question};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::app::App;

/// Answers kept: the newest this many.
const KEEP: usize = 2000;

#[derive(Serialize, Deserialize)]
struct Kept {
    key: String,
    answer: Answer,
    at: String,
}

/// The state entries a question names (`` `key` `` in its text); all of them when it names none.
fn named<'a>(
    q: &Question,
    state: &'a BTreeMap<String, serde_json::Value>,
) -> Vec<(&'a String, &'a serde_json::Value)> {
    let text = q.to_json().to_string();
    let hits: Vec<_> = state
        .iter()
        .filter(|(k, _)| text.contains(&format!("`{k}`")))
        .collect();
    if hits.is_empty() {
        state.iter().collect()
    } else {
        hits
    }
}

fn key(model: &str, q: &Question, state: &BTreeMap<String, serde_json::Value>) -> String {
    let mut h = Sha256::new();
    h.update(model.as_bytes());
    h.update([0]);
    h.update(q.to_json().to_string().as_bytes());
    for (k, v) in named(q, state) {
        h.update([0]);
        h.update(k.as_bytes());
        h.update([0]);
        h.update(v.to_string().as_bytes());
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

impl App {
    fn answers_path(&self) -> std::path::PathBuf {
        self.layout()
            .judge_activity()
            .with_file_name("judge-answers.jsonl")
    }

    /// The questions of `j` answered before (by question id). Requests with images are never cached.
    pub(super) fn cached_answers(&self, model: &str, j: &Judging) -> BTreeMap<String, Answer> {
        if !j.images.is_empty() {
            return BTreeMap::new();
        }
        let Ok(text) = std::fs::read_to_string(self.answers_path()) else {
            return BTreeMap::new();
        };
        let kept: HashMap<String, Answer> = text
            .lines()
            .filter_map(|l| serde_json::from_str::<Kept>(l).ok())
            .map(|k| (k.key, k.answer))
            .collect();
        j.questions
            .iter()
            .filter_map(|(id, q)| {
                kept.get(&key(model, q, &j.state))
                    .map(|a| (id.clone(), a.clone()))
            })
            .collect()
    }

    /// Keeps the answers `j` got (best effort).
    pub(super) fn keep_answers(
        &self,
        model: &str,
        j: &Judging,
        answers: &BTreeMap<String, Answer>,
    ) {
        use std::io::Write;
        if !j.images.is_empty() || answers.is_empty() {
            return;
        }
        let path = self.answers_path();
        let at = agenium_core::now_iso();
        let lines: Vec<String> = answers
            .iter()
            .filter_map(|(id, a)| {
                let q = j.questions.get(id)?;
                serde_json::to_string(&Kept {
                    key: key(model, q, &j.state),
                    answer: a.clone(),
                    at: at.clone(),
                })
                .ok()
            })
            .collect();
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            for l in &lines {
                let _ = writeln!(f, "{l}");
            }
        }
        if let Ok(text) = std::fs::read_to_string(&path) {
            let all: Vec<&str> = text.lines().collect();
            if all.len() > 2 * KEEP {
                let _ = std::fs::write(&path, all[all.len() - KEEP..].join("\n") + "\n");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_key_covers_only_the_state_a_question_names() {
        let q = Question::Noul {
            instructions: "Is `report` done?".into(),
        };
        let mut a = BTreeMap::new();
        a.insert("report".to_owned(), serde_json::json!("done"));
        let mut b = a.clone();
        b.insert("run_facts".to_owned(), serde_json::json!("cargo test"));
        assert_eq!(
            key("m", &q, &a),
            key("m", &q, &b),
            "an unnamed entry does not matter"
        );
        b.insert("report".to_owned(), serde_json::json!("not done"));
        assert_ne!(key("m", &q, &a), key("m", &q, &b), "the named entry does");
        assert_ne!(
            key("m", &q, &a),
            key("n", &q, &a),
            "another model is another key"
        );
    }
}
