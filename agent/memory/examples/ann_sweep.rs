//! ANN parameter sweep on a kept database (ADR-125 follow-up, `docs/reference/STORE-SCALE.md` §14): recall@10 against
//! exact search and KNN latency for each build (connectivity, parallel or serial inserts) and search setting
//! (expansion_search, rerank), all on the same facts.
//!
//! `cargo run --release -p agenium-memory --example ann_sweep -- --corpus <file> --db <dir> [--models <dir>] [n]`
//!
//! The corpus has one sentence per line: the first `n` (default 100 000) are the facts, the next 500 give the queries
//! (their first eight words, embedded as queries). The database in `--db` is kept and topped up, so the slow part —
//! embedding the facts — runs once; each build replaces the sidecar.
//!
//! `--agent M,EF` skips the sweep and measures what an agent gets instead: the top 8 of `hybrid` and of the full
//! `retrieve::recall` pipeline with the sidecar (connectivity M, expansion_search EF) against the same with exact search.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use agenium_memory::retrieve::{recall, RecallContext};
use agenium_memory::{default_embedder, Embedder, HashEmbedder};
use agenium_store::{AgentOptions, AgentStore, AnnOptions, IndexMode, NewFact};

fn pct(xs: &[f64], p: f64) -> f64 {
    let mut xs = xs.to_vec();
    xs.sort_by(f64::total_cmp);
    xs[((xs.len() as f64 - 1.0) * p).round() as usize]
}

fn open(path: &Path, e: &dyn Embedder, mode: IndexMode, ann: AnnOptions) -> AgentStore {
    let opts = AgentOptions {
        index_mode: mode,
        ann,
        ..AgentOptions::default()
    };
    AgentStore::open_with(
        path,
        e.dim(),
        e.model_id(),
        &path.with_extension("pre"),
        opts,
    )
    .unwrap()
}

/// Embeds `texts` on a few threads (the model is the slow part of ingest), keeping the order.
fn embed_all(e: &Arc<dyn Embedder>, texts: &[String]) -> Vec<Vec<f32>> {
    let workers = 3;
    let per = texts.len().div_ceil(workers).max(1);
    std::thread::scope(|s| {
        let hs: Vec<_> = texts
            .chunks(per)
            .map(|part| s.spawn(move || e.embed(part).unwrap()))
            .collect();
        hs.into_iter().flat_map(|h| h.join().unwrap()).collect()
    })
}

fn main() {
    let (mut n, mut corpus, mut db, mut models, mut agent) = (100_000usize, None, None, None, None);
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--corpus" => corpus = args.next(),
            "--db" => db = args.next(),
            "--models" => models = args.next(),
            "--agent" => agent = args.next(),
            _ => n = a.parse().expect("size"),
        }
    }
    let corpus = std::fs::read_to_string(corpus.expect("--corpus <file>")).unwrap();
    let lines: Vec<&str> = corpus.lines().filter(|l| !l.trim().is_empty()).collect();
    assert!(
        lines.len() >= n + 500,
        "the corpus has {} sentences",
        lines.len()
    );
    let e: Arc<dyn Embedder> = match &models {
        Some(m) => default_embedder(Path::new(m)),
        None => Arc::new(HashEmbedder::new(384)),
    };
    let dir = PathBuf::from(db.expect("--db <dir>"));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("agent-{n}.db"));
    println!(
        "embedder: {} ({}-d), {n} facts, db {}",
        e.model_id(),
        e.dim(),
        path.display()
    );

    // Ingest (resumable): facts in corpus order, one per transaction.
    {
        let store = open(&path, e.as_ref(), IndexMode::Exact, AnnOptions::default());
        let have = store.count_live().unwrap() as usize;
        let t0 = Instant::now();
        let mut done = have;
        while done < n {
            let texts: Vec<String> = lines[done..(done + 768).min(n)]
                .iter()
                .map(|s| s.to_string())
                .collect();
            let vs = embed_all(&e, &texts);
            for (t, v) in texts.iter().zip(&vs) {
                store
                    .add(&NewFact::fact(t.as_str(), "corpus"), Some(v))
                    .unwrap();
            }
            done += texts.len();
            eprintln!(
                "  ingested {done}/{n} ({:.0}/s)",
                (done - have) as f64 / t0.elapsed().as_secs_f64()
            );
        }
    }

    let queries: Vec<String> = lines[n..n + 500]
        .iter()
        .map(|s| s.split_whitespace().take(8).collect::<Vec<_>>().join(" "))
        .collect();
    let qv: Vec<Vec<f32>> = queries.iter().map(|q| e.embed_query(q).unwrap()).collect();
    if let Some(spec) = agent {
        let (m, ef) = spec.split_once(',').expect("--agent M,EF");
        agent_view(
            &path,
            &e,
            &queries,
            &qv,
            m.parse().unwrap(),
            ef.parse().unwrap(),
        );
        return;
    }
    let (truth, exact_ms) = {
        let store = open(&path, e.as_ref(), IndexMode::Exact, AnnOptions::default());
        let mut t = vec![];
        let mut ms = vec![];
        for v in &qv {
            let s = Instant::now();
            t.push(store.knn_exact(v, 10).unwrap());
            ms.push(s.elapsed().as_secs_f64() * 1000.0);
        }
        (t, ms)
    };
    println!(
        "exact KNN p50/p95: {:.2} / {:.2} ms",
        pct(&exact_ms, 0.5),
        pct(&exact_ms, 0.95)
    );
    println!("| build (M, inserts) | build s | ef_search | rerank | recall@10 | top-1 = exact | KNN p50/p95 ms |");
    println!("|---|---|---|---|---|---|---|");

    let builds: [(usize, usize, &str); 3] =
        [(16, 0, "parallel"), (16, 1, "serial"), (32, 0, "parallel")];
    for (m, threads, how) in builds {
        let base = AnnOptions {
            connectivity: m,
            build_threads: threads,
            scale_ef: false,
            ..AnnOptions::default()
        };
        for f in agenium_store::ann::sidecar_files(&path) {
            let _ = std::fs::remove_file(f);
        }
        let t0 = Instant::now();
        drop(open(&path, e.as_ref(), IndexMode::Ann, base.clone()));
        let build_s = t0.elapsed().as_secs_f64();
        for ef in [64, 128, 256, 512] {
            for rerank in [false, true] {
                let store = open(
                    &path,
                    e.as_ref(),
                    IndexMode::Ann,
                    AnnOptions {
                        expansion_search: ef,
                        rerank,
                        ..base.clone()
                    },
                );
                assert_eq!(store.index_status().builds, 0, "the sidecar was rebuilt");
                let (mut found, mut top1, mut ms) = (0, 0, vec![]);
                for (v, t) in qv.iter().zip(&truth) {
                    let s = Instant::now();
                    let got = store.knn(v, 10).unwrap();
                    ms.push(s.elapsed().as_secs_f64() * 1000.0);
                    found += t.iter().filter(|h| got.iter().any(|g| g.0 == h.0)).count();
                    top1 += usize::from(got.first().map(|g| g.0) == t.first().map(|h| h.0));
                }
                println!(
                    "| M{m}, {how} | {build_s:.1} | {ef} | {} | {:.4} | {:.3} | {:.3} / {:.3} |",
                    if rerank { "x4" } else { "off" },
                    found as f64 / (qv.len() * 10) as f64,
                    top1 as f64 / qv.len() as f64,
                    pct(&ms, 0.5),
                    pct(&ms, 0.95)
                );
            }
        }
    }
}

/// Top-8 agreement of `hybrid` and `recall` between the sidecar and exact search, and their latency.
fn agent_view(
    path: &Path,
    e: &Arc<dyn Embedder>,
    queries: &[String],
    qv: &[Vec<f32>],
    m: usize,
    ef: usize,
) {
    let ann = AnnOptions {
        connectivity: m,
        expansion_search: ef,
        scale_ef: false,
        ..AnnOptions::default()
    };
    let ctx = RecallContext {
        project: None,
        scope: serde_json::json!({}),
    };
    type Run = (Vec<Vec<i64>>, Vec<Vec<i64>>, Vec<f64>, Vec<f64>);
    let run = |store: &AgentStore| -> Run {
        let (mut hy, mut rc, mut hy_ms, mut rc_ms) = (vec![], vec![], vec![], vec![]);
        for (q, v) in queries.iter().zip(qv) {
            let t = Instant::now();
            hy.push(
                store
                    .hybrid(q, Some(v), 8)
                    .unwrap()
                    .into_iter()
                    .map(|h| h.fact.id)
                    .collect(),
            );
            hy_ms.push(t.elapsed().as_secs_f64() * 1000.0);
            let t = Instant::now();
            rc.push(
                recall(store, e.as_ref(), q, &ctx, 8)
                    .unwrap()
                    .into_iter()
                    .map(|r| r.id)
                    .collect(),
            );
            rc_ms.push(t.elapsed().as_secs_f64() * 1000.0);
        }
        (hy, rc, hy_ms, rc_ms)
    };
    let exact = run(&open(path, e.as_ref(), IndexMode::Exact, ann.clone()));
    let with_ann = {
        let store = open(path, e.as_ref(), IndexMode::Ann, ann);
        println!("sidecar builds on open: {}", store.index_status().builds);
        run(&store)
    };
    let agree = |a: &[Vec<i64>], b: &[Vec<i64>]| {
        let (mut same, mut total, mut top1) = (0, 0, 0);
        for (x, y) in a.iter().zip(b) {
            same += x.iter().filter(|id| y.contains(id)).count();
            total += x.len().max(y.len());
            top1 += usize::from(x.first() == y.first());
        }
        (
            same as f64 / total.max(1) as f64,
            top1 as f64 / a.len() as f64,
        )
    };
    let (hs, h1) = agree(&exact.0, &with_ann.0);
    let (rs, r1) = agree(&exact.1, &with_ann.1);
    println!(
        "| M{m}, ef {ef} | top-8 same as exact | top-1 same | exact p50/p95 ms | ANN p50/p95 ms |"
    );
    println!("|---|---|---|---|---|");
    println!(
        "| hybrid | {hs:.4} | {h1:.3} | {:.2} / {:.2} | {:.2} / {:.2} |",
        pct(&exact.2, 0.5),
        pct(&exact.2, 0.95),
        pct(&with_ann.2, 0.5),
        pct(&with_ann.2, 0.95)
    );
    println!(
        "| recall (vector + BM25 + graph) | {rs:.4} | {r1:.3} | {:.2} / {:.2} | {:.2} / {:.2} |",
        pct(&exact.3, 0.5),
        pct(&exact.3, 0.95),
        pct(&with_ann.3, 0.5),
        pct(&with_ann.3, 0.95)
    );
}
