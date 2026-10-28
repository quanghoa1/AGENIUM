//! Finding facts: nearest vectors (exact or the ANN sidecar), BM25 keywords, and both fused by reciprocal rank.

use super::*;

/// Turns free text into a safe FTS5 query: every word token quoted, OR-ed (BM25 ranks by overlap).
pub fn fts_query(text: &str) -> Option<String> {
    let words: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() >= 2)
        .take(32)
        .map(|w| format!("\"{}\"", w.to_lowercase()))
        .collect();
    (!words.is_empty()).then(|| words.join(" OR "))
}

/// English words so common that BM25 gives them almost no weight, yet matching them makes a search touch most of the
/// index (re-measured 2026-10-25: queries keeping them cost several times more at 100 k facts).
pub(super) const STOPWORDS: &[&str] = &[
    "a", "about", "after", "all", "also", "an", "and", "any", "are", "as", "at", "be", "been",
    "before", "but", "by", "can", "could", "did", "do", "does", "for", "from", "had", "has",
    "have", "he", "her", "his", "how", "i", "if", "in", "into", "is", "it", "its", "may", "me",
    "more", "most", "my", "no", "not", "of", "on", "one", "only", "or", "other", "our", "out",
    "over", "she", "should", "so", "some", "such", "than", "that", "the", "their", "them", "then",
    "there", "these", "they", "this", "those", "through", "to", "up", "us", "very", "was", "we",
    "were", "what", "when", "where", "which", "while", "who", "will", "with", "would", "you",
    "your",
];

/// The FTS5 query for a fact search: like [`fts_query`] without stopwords. `None` when only stopwords remain (the
/// vector channel still answers).
pub fn fts_search_query(text: &str) -> Option<String> {
    let words: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|w| w.chars().count() >= 2 && !STOPWORDS.contains(&w.as_str()))
        .take(32)
        .map(|w| format!("\"{w}\""))
        .collect();
    (!words.is_empty()).then(|| words.join(" OR "))
}

impl AgentStore {
    /// Which index serves KNN now, and the sidecar's state.
    pub fn index_status(&self) -> AnnStatus {
        self.vectors.status()
    }

    /// Waits for a sidecar build to finish (benchmarks, tests). `false` on timeout.
    pub fn wait_for_index(&self, timeout: Duration) -> bool {
        self.vectors.wait_ready(timeout)
    }

    /// Exact KNN whatever the active index (recall measurement, tests).
    pub fn knn_exact(&self, query: &[f32], k: usize) -> Result<Vec<(i64, f64)>, StoreError> {
        self.check_vec(query)?;
        if k == 0 || !usable(query) {
            return Ok(vec![]);
        }
        Exact(&self.db).knn(query, k)
    }

    /// Is there any live fact? (Stops at the first one; `count_live` walks them all, ~20 ms at 1 M facts.)
    pub fn has_live(&self) -> Result<bool, StoreError> {
        self.db.read(|c| {
            Ok(c.query_row(
                "SELECT EXISTS (SELECT 1 FROM fact WHERE superseded_by IS NULL)",
                [],
                |r| r.get(0),
            )?)
        })
    }

    pub fn count_live(&self) -> Result<u64, StoreError> {
        self.db.read(|c| {
            Ok(c.query_row(
                "SELECT count(*) FROM fact WHERE superseded_by IS NULL",
                [],
                |r| r.get::<_, i64>(0),
            )? as u64)
        })
    }

    /// Cosine similarity of the query to each given fact that has a vector (facts without one are absent).
    pub fn similarity_to(
        &self,
        query: &[f32],
        ids: &[i64],
    ) -> Result<HashMap<i64, f64>, StoreError> {
        self.check_vec(query)?;
        let mut out = HashMap::new();
        if ids.is_empty() || !usable(query) {
            return Ok(out);
        }
        let qn = query
            .iter()
            .map(|x| (*x as f64).powi(2))
            .sum::<f64>()
            .sqrt();
        self.db.read(|c| {
            let mut st = c.prepare_cached("SELECT embedding FROM fact_vec WHERE rowid = ?1")?;
            for id in ids {
                let blob: Option<Vec<u8>> = st.query_row([id], |r| r.get(0)).optional()?;
                let Some(blob) = blob else { continue };
                let v = blob
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|b| f32::from_le_bytes(*b) as f64);
                let (mut dot, mut vn) = (0.0, 0.0);
                for (x, q) in v.zip(query) {
                    dot += x * *q as f64;
                    vn += x * x;
                }
                if vn > 0.0 && qn > 0.0 {
                    out.insert(*id, dot / (vn.sqrt() * qn));
                }
            }
            Ok(out)
        })
    }

    /// Cosine KNN over live facts: `(fact id, distance)` ascending, through the active index (the ANN sidecar from
    /// `ann_threshold` facts, else exact). A query without a direction finds nothing.
    pub fn knn(&self, query: &[f32], k: usize) -> Result<Vec<(i64, f64)>, StoreError> {
        self.check_vec(query)?;
        if k == 0 || !usable(query) {
            return Ok(vec![]);
        }
        let rerank = self.vectors.options().ann.rerank;
        match self.vectors.knn(query, if rerank { k * 4 } else { k }) {
            Some(hits) if rerank => {
                let ids: Vec<i64> = hits.iter().map(|h| h.0).collect();
                let sim = self.similarity_to(query, &ids)?;
                let mut out: Vec<(i64, f64)> =
                    sim.into_iter().map(|(id, s)| (id, 1.0 - s)).collect();
                out.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
                out.truncate(k);
                Ok(out)
            }
            Some(hits) => Ok(hits),
            None => Exact(&self.db).knn(query, k),
        }
    }

    /// The index serving KNN now.
    pub fn index_kind(&self) -> IndexKind {
        self.index_status().active
    }

    /// BM25 keyword search over live facts: `(fact id, bm25)` best first.
    pub fn keyword(&self, text: &str, k: usize) -> Result<Vec<(i64, f64)>, StoreError> {
        let Some(q) = fts_search_query(text) else {
            return Ok(vec![]);
        };
        self.db.read(|c| {
            let mut st = c.prepare_cached(
                "SELECT f.id, bm25(fact_fts) AS s FROM fact_fts JOIN fact f ON f.id = fact_fts.rowid
                 WHERE fact_fts MATCH ?1 AND f.superseded_by IS NULL ORDER BY s LIMIT ?2",
            )?;
            let rows = st.query_map(params![q, k as i64], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
        })
    }

    /// Hybrid retrieval: vector KNN + BM25, fused by reciprocal rank (k = 60), hidden facts filtered.
    /// `project` boosts facts of the current project (decision 25) by ordering ties.
    pub fn hybrid(
        &self,
        text: &str,
        query: Option<&[f32]>,
        k: usize,
    ) -> Result<Vec<Hit>, StoreError> {
        let pool = (k * 4).max(20);
        let vec_hits = match query {
            Some(q) => self.knn(q, pool)?,
            None => vec![],
        };
        let kw_hits = self.keyword(text, pool)?;
        let mut score: HashMap<i64, (f64, bool, bool)> = HashMap::new();
        for (rank, (id, _)) in vec_hits.iter().enumerate() {
            let e = score.entry(*id).or_default();
            e.0 += 1.0 / (RRF_K + rank as f64 + 1.0);
            e.1 = true;
        }
        for (rank, (id, _)) in kw_hits.iter().enumerate() {
            let e = score.entry(*id).or_default();
            e.0 += 1.0 / (RRF_K + rank as f64 + 1.0);
            e.2 = true;
        }
        let ids: Vec<i64> = score.keys().copied().collect();
        let facts = self.db.read(|c| Self::facts_by_ids(c, &ids))?;
        let mut hits: Vec<Hit> = score
            .into_iter()
            .filter_map(|(id, (s, v, kw))| {
                let f = facts.get(&id)?.clone();
                (f.is_live() && f.weight >= HIDE_BELOW_WEIGHT).then_some(Hit {
                    score: s * f.weight,
                    via: match (v, kw) {
                        (true, true) => Via::Hybrid,
                        (true, false) => Via::Vector,
                        _ => Via::Keyword,
                    },
                    fact: f,
                })
            })
            .collect();
        hits.sort_by(|a, b| b.score.total_cmp(&a.score).then(b.fact.id.cmp(&a.fact.id)));
        hits.truncate(k);
        Ok(hits)
    }
}
