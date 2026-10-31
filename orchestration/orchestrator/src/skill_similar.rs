//! Library skills close to a skill being written or proposed (ADR-152), so the skill writer and the user can merge
//! instead of adding a second skill on the same job. The measures of ADR-92 (subject words, and meaning with the local
//! embedding model: no model call), with a lower bar for "near": the user is asked, not refused. Each library skill's
//! subject vector is kept by the subject's text, so a check embeds only what is new.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use agenium_memory::{Embedder, HashEmbedder};
use serde::Serialize;

use crate::app::App;
use crate::skill_conflict::{subject, subject_words, SAME_SUBJECT_MEANING, SAME_SUBJECT_WORDS};

/// Near the same subject from these scores (below them nothing is shown).
pub const NEAR_WORDS: f32 = 0.30;
pub const NEAR_MEANING: f32 = 0.70;
/// For a request (before anything is written): a short request shares few words with a skill even on its subject (a
/// review request scored refactoring highest by words, 0.32), so words count only as high as a duplicate's; meaning,
/// with the local model, is the measure, a little lower than for a skill.
const REQUEST_WORDS: f32 = SAME_SUBJECT_WORDS;
const REQUEST_MEANING: f32 = 0.65;

/// A library skill close to the text checked.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SimilarSkill {
    pub name: String,
    /// Subject words in common (0..1).
    pub words: f32,
    /// Closeness of meaning (0..1), when the local embedding model is installed.
    pub meaning: Option<f32>,
    /// `duplicate` (the bars ADR-92 refuses an agent's skill at) or `near`.
    pub level: &'static str,
}

/// Vectors by (model, text).
type Vectors = HashMap<(String, String), Vec<f32>>;

/// Subject vectors: a library skill is embedded once until its subject changes.
static CACHE: LazyLock<Mutex<Vectors>> = LazyLock::new(Default::default);

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let d: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    d / (na * nb).max(1e-9)
}

/// Vectors of `texts` with `e`, from the cache where it has them.
fn embed_cached(e: &dyn Embedder, texts: &[String]) -> Option<Vec<Vec<f32>>> {
    let id = e.model_id().to_owned();
    let mut cache = CACHE.lock().unwrap_or_else(|p| p.into_inner());
    let missing: Vec<String> = texts
        .iter()
        .filter(|t| !cache.contains_key(&(id.clone(), (*t).clone())))
        .cloned()
        .collect();
    if !missing.is_empty() {
        let vs = e.embed(&missing).ok()?;
        for (t, v) in missing.into_iter().zip(vs) {
            cache.insert((id.clone(), t), v);
        }
    }
    texts
        .iter()
        .map(|t| cache.get(&(id.clone(), t.clone())).cloned())
        .collect()
}

/// The skills of `library` (name, subject) close to `me`, closest first, at most `k`. `request`: `me` is a request,
/// not a skill (lower bars).
pub fn similar(
    me: &str,
    library: &[(String, String)],
    semantic: Option<&dyn Embedder>,
    k: usize,
    request: bool,
) -> Vec<SimilarSkill> {
    if library.is_empty() || me.trim().is_empty() {
        return Vec::new();
    }
    let words_of = |s: &str| subject_words(s);
    let hash = HashEmbedder::new(384);
    let mut texts: Vec<String> = vec![words_of(me)];
    texts.extend(library.iter().map(|(_, s)| words_of(s)));
    let words = hash.embed(&texts).unwrap_or_default();
    let meaning = semantic
        .filter(|e| !e.model_id().starts_with("hash"))
        .and_then(|e| {
            let mut t: Vec<String> = vec![me.to_owned()];
            t.extend(library.iter().map(|(_, s)| s.clone()));
            embed_cached(e, &t)
        });
    let (near_w, near_m) = if request {
        (REQUEST_WORDS, REQUEST_MEANING)
    } else {
        (NEAR_WORDS, NEAR_MEANING)
    };
    let mut out: Vec<(f32, SimilarSkill)> = Vec::new();
    for (i, (name, _)) in library.iter().enumerate() {
        let w = match (words.first(), words.get(i + 1)) {
            (Some(a), Some(b)) => cosine(a, b),
            _ => 0.0,
        };
        let m = meaning
            .as_ref()
            .and_then(|v| Some(cosine(v.first()?, v.get(i + 1)?)));
        let dup = w >= SAME_SUBJECT_WORDS || m.is_some_and(|m| m >= SAME_SUBJECT_MEANING);
        let near = w >= near_w || m.is_some_and(|m| m >= near_m);
        if !near {
            continue;
        }
        // Ranked by how far past its own bar each measure is.
        let rank = (w / near_w).max(m.map_or(0.0, |m| m / near_m));
        out.push((
            rank,
            SimilarSkill {
                name: name.clone(),
                words: w,
                meaning: m,
                level: if dup { "duplicate" } else { "near" },
            },
        ));
    }
    out.sort_by(|a, b| b.0.total_cmp(&a.0));
    out.into_iter().take(k).map(|(_, s)| s).collect()
}

impl App {
    /// The library's skills (name, subject), except `skip`.
    pub fn library_subjects(&self, skip: Option<&str>) -> Vec<(String, String)> {
        self.skill_registry()
            .library()
            .skills
            .iter()
            .filter(|s| Some(s.meta.name.as_str()) != skip)
            .map(|s| (s.meta.name.clone(), subject(&s.meta.description, &s.body)))
            .collect()
    }

    /// Library skills close to a skill (its description and body), except `skip` (the skill itself), at most `k`.
    pub fn similar_skills(
        &self,
        description: &str,
        body: &str,
        skip: Option<&str>,
        k: usize,
    ) -> Vec<SimilarSkill> {
        similar(
            &subject(description, body),
            &self.library_subjects(skip),
            Some(self.embedder().as_ref()),
            k,
            false,
        )
    }

    /// Library skills close to a request for a skill (what the user asked), at most `k`.
    pub fn skills_for_request(&self, request: &str, k: usize) -> Vec<SimilarSkill> {
        similar(
            request,
            &self.library_subjects(None),
            Some(self.embedder().as_ref()),
            k,
            true,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_skills_by_words_ranked_and_leveled() {
        let lib = vec![
            (
                "changelog".to_owned(),
                "write the changelog entry for a merged change under its section".to_owned(),
            ),
            (
                "seo-audit".to_owned(),
                "audit a page for search engines titles meta descriptions".to_owned(),
            ),
            (
                "db-backup".to_owned(),
                "back up the postgres database nightly".to_owned(),
            ),
        ];
        let s = similar(
            "write a changelog entry for each merged change",
            &lib,
            None,
            3,
            false,
        );
        assert_eq!(s[0].name, "changelog");
        assert!(s.iter().all(|x| x.name != "db-backup"), "{s:?}");
        assert_eq!(s[0].meaning, None, "no model: words only");
        assert!(matches!(s[0].level, "duplicate" | "near"));
        assert!(similar("", &lib, None, 3, false).is_empty());
        assert!(similar("anything", &[], None, 3, false).is_empty());
    }
}
