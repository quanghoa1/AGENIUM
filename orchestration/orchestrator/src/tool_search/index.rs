//! What a search ranks: the catalogue's tools with their words (BM25) and their meaning (the memory's embedding model,
//! ADR-47), the vectors kept for the process so a tool is embedded once.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock};

use agenium_mcp::rank::{self, Bm25};
use agenium_mcp::McpTool;
use agenium_memory::Embedder;
use agenium_tools::Tool;

/// Tool vectors by (model, text): a catalogue listed again embeds only its new tools.
type Vectors = Mutex<HashMap<(String, u64), Arc<Vec<f32>>>>;
fn vectors() -> &'static Vectors {
    static V: OnceLock<Vectors> = OnceLock::new();
    V.get_or_init(Default::default)
}
/// Kept vectors at most (about 1.5 KB each).
const KEEP_VECTORS: usize = 20_000;

fn hash(s: &str) -> u64 {
    let mut h = DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

pub(super) struct Index {
    pub tools: Vec<Arc<McpTool>>,
    /// What identifies this list of tools (a listed-again catalogue with other tools builds a new index).
    pub print: u64,
    bm25: Bm25,
    texts: Vec<String>,
}

/// The fingerprint of a list of tools.
pub(super) fn print(tools: &[Arc<McpTool>]) -> u64 {
    let mut h = DefaultHasher::new();
    for t in tools {
        t.name().hash(&mut h);
        t.description().hash(&mut h);
        t.parameters().to_string().hash(&mut h);
    }
    h.finish()
}

impl Index {
    pub fn new(tools: Vec<Arc<McpTool>>) -> Index {
        let texts: Vec<String> = tools
            .iter()
            .map(|t| {
                rank::tool_text(
                    &format!("{} {}", t.server(), t.mcp_name()),
                    t.plain_description(),
                    &t.parameters(),
                )
            })
            .collect();
        Index {
            print: print(&tools),
            bm25: Bm25::new(&texts),
            tools,
            texts,
        }
    }

    /// Tools by words, best first.
    pub fn by_words(&self, query: &str) -> Vec<usize> {
        rank::order(&self.bm25.scores(query))
    }

    /// Tools by meaning, best first (`None` when the model cannot embed). Runs the model off the async threads.
    pub async fn by_meaning(&self, embedder: Arc<dyn Embedder>, query: &str) -> Option<Vec<usize>> {
        let texts = self.texts.clone();
        let query = query.to_owned();
        tokio::task::spawn_blocking(move || {
            let model = embedder.model_id().to_owned();
            let mut have: Vec<Option<Arc<Vec<f32>>>> = {
                let v = vectors().lock().unwrap_or_else(|e| e.into_inner());
                texts
                    .iter()
                    .map(|t| v.get(&(model.clone(), hash(t))).cloned())
                    .collect()
            };
            let missing: Vec<usize> = (0..texts.len()).filter(|i| have[*i].is_none()).collect();
            if !missing.is_empty() {
                let batch: Vec<String> = missing.iter().map(|i| texts[*i].clone()).collect();
                let new = embedder.embed(&batch).ok()?;
                let mut v = vectors().lock().unwrap_or_else(|e| e.into_inner());
                if v.len() + new.len() > KEEP_VECTORS {
                    v.clear();
                }
                for (i, e) in missing.into_iter().zip(new) {
                    let e = Arc::new(e);
                    v.insert((model.clone(), hash(&texts[i])), e.clone());
                    have[i] = Some(e);
                }
            }
            let q = embedder.embed_query(&query).ok()?;
            let scores: Vec<f64> = have
                .iter()
                .map(|e| match e {
                    Some(e) if e.len() == q.len() => e
                        .iter()
                        .zip(&q)
                        .map(|(a, b)| (*a as f64) * (*b as f64))
                        .sum(),
                    _ => 0.0,
                })
                .collect();
            let mut order = rank::order(&scores);
            order.truncate(30);
            Some(order)
        })
        .await
        .ok()
        .flatten()
    }
}
