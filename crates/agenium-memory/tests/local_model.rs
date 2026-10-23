//! Local embedding model (M15-2). Needs the model files: set `AGENIUM_TEST_MODELS` to a models root holding
//! `bge-small-en-v1.5/` (`agenium models download` creates it). Without it the tests are skipped (and say so).

#![cfg(feature = "local")]

use std::path::PathBuf;
use std::time::Instant;

use agenium_memory::local::{self, LocalEmbedder, BGE_SMALL_EN};
use agenium_memory::Embedder;

fn models() -> Option<PathBuf> {
    match std::env::var_os("AGENIUM_TEST_MODELS") {
        Some(p) => Some(PathBuf::from(p)),
        None => {
            eprintln!("SKIPPED: set AGENIUM_TEST_MODELS to a models root with bge-small-en-v1.5");
            None
        }
    }
}

fn cos(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

#[test]
fn model_files_verify_and_embeddings_are_sane() {
    let Some(root) = models() else { return };
    assert!(local::installed(&BGE_SMALL_EN, &root));
    assert!(
        local::verify(&BGE_SMALL_EN, &root).is_empty(),
        "sha256 of every file"
    );
    let t = Instant::now();
    let e = LocalEmbedder::load(BGE_SMALL_EN, &root).unwrap();
    eprintln!("load: {} ms", t.elapsed().as_millis());
    let v = e
        .embed(&[
            "The tests run with cargo test.".into(),
            "Bananas are yellow.".into(),
        ])
        .unwrap();
    assert_eq!(v[0].len(), 384);
    let n: f32 = v[0].iter().map(|x| x * x).sum::<f32>().sqrt();
    assert!((n - 1.0).abs() < 1e-4, "L2-normalised, got {n}");
    let again = e.embed(&["The tests run with cargo test.".into()]).unwrap();
    assert!(cos(&v[0], &again[0]) > 0.9999, "deterministic");
    let q = e.embed_query("how do I run the test suite?").unwrap();
    assert!(
        cos(&q, &v[0]) > cos(&q, &v[1]) + 0.1,
        "semantic, not lexical"
    );
    // Batching (padding) does not change a text's vector.
    let solo = e.embed(&["Bananas are yellow.".into()]).unwrap();
    assert!(cos(&solo[0], &v[1]) > 0.999);
}

#[derive(serde::Deserialize)]
struct Pair {
    fact: String,
    query: String,
    vi: String,
}

#[derive(serde::Deserialize)]
struct Fixture {
    pairs: Vec<Pair>,
    distractors: Vec<String>,
}

#[test]
fn recall_and_throughput_on_the_fixture() {
    let Some(root) = models() else { return };
    let e = LocalEmbedder::load(BGE_SMALL_EN, &root).unwrap();
    let fx: Fixture = serde_json::from_str(include_str!("data/retrieval.json")).unwrap();
    let mut corpus: Vec<String> = fx.pairs.iter().map(|p| p.fact.clone()).collect();
    corpus.extend(fx.distractors.iter().cloned());

    let t = Instant::now();
    let vecs = e.embed(&corpus).unwrap();
    let batch_rate = corpus.len() as f64 / t.elapsed().as_secs_f64();
    let t = Instant::now();
    for c in corpus.iter().take(20) {
        e.embed(std::slice::from_ref(c)).unwrap();
    }
    let single_rate = 20.0 / t.elapsed().as_secs_f64();

    let recall = |queries: Vec<&str>| -> (f64, f64) {
        let (mut at1, mut at5) = (0, 0);
        for (i, q) in queries.iter().enumerate() {
            let qv = e.embed_query(q).unwrap();
            let mut ranked: Vec<(usize, f32)> = vecs
                .iter()
                .enumerate()
                .map(|(j, v)| (j, cos(&qv, v)))
                .collect();
            ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
            let pos = ranked.iter().position(|(j, _)| *j == i).unwrap();
            at1 += (pos == 0) as u32;
            at5 += (pos < 5) as u32;
        }
        let n = queries.len() as f64;
        (at1 as f64 / n, at5 as f64 / n)
    };
    let t = Instant::now();
    let (en1, en5) = recall(fx.pairs.iter().map(|p| p.query.as_str()).collect());
    let query_ms = t.elapsed().as_secs_f64() * 1000.0 / fx.pairs.len() as f64;
    let (vi1, vi5) = recall(fx.pairs.iter().map(|p| p.vi.as_str()).collect());
    eprintln!(
        "bge-small-en-v1.5 on {} facts: EN recall@1 {en1:.3} recall@5 {en5:.3} | VI query -> EN fact recall@1 {vi1:.3} recall@5 {vi5:.3} | embed {batch_rate:.0} facts/s (batch 32), {single_rate:.0} facts/s (single), query {query_ms:.1} ms (incl. ranking)",
        corpus.len()
    );
    // S6 pass threshold for English (memory is English-only, ADR-40).
    assert!(en5 >= 0.85, "EN recall@5 {en5}");
    assert!(
        single_rate >= 20.0,
        "S6 throughput threshold: {single_rate} facts/s"
    );
}
