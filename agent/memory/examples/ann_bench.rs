//! ANN sidecar benchmark (ADR-50 §2–3, `docs/reference/STORE-SCALE.md` §13): for each size, the exact sqlite-vec scan
//! against the usearch sidecar through the production API — build time, sidecar size, KNN p50/p95, recall@10 against
//! exact, hybrid p50/p95 with each index, reopen with a saved sidecar, and live ingest while the sidecar is active.
//!
//! `cargo run --release -p agenium-memory --example ann_bench -- [sizes …] [--models <dir with bge-small-en-v1.5>]`
//! (default size: 100000). With `--models` the vectors come from the local bge-small model (real embeddings, as in
//! ADR-50's recall figures); without it from the hashing embedder. Text is the synthetic Zipf corpus of `memory_bench`.

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use agenium_memory::{default_embedder, Embedder, HashEmbedder};
use agenium_store::{AgentOptions, AgentStore, IndexKind, IndexMode, NewFact};

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// The `memory_bench` vocabulary: real engineering words, then pseudo-words, Zipf-weighted (s = 1.1).
fn vocabulary() -> (Vec<String>, Vec<f64>) {
    let real = "the a to of and in for is on with run test build file config error fix deploy merge commit branch \
        ticket agent skill memory lesson review report api server client request response timeout retry cache index \
        query table column migration schema backup restore token secret key vault sandbox shell command path network \
        project team lead worker plan spec acceptance criterion evidence proof diff patch release version tag log";
    let mut v: Vec<String> = real.split_whitespace().map(str::to_owned).collect();
    let syll = [
        "ka", "ro", "mi", "tu", "sel", "dan", "ver", "lo", "pri", "zen", "ox", "qua", "bel", "tor",
        "nis",
    ];
    let mut r = Rng(0x9E37_79B9_7F4A_7C15);
    while v.len() < 6000 {
        let n = 2 + (r.next() % 3) as usize;
        let w: String = (0..n)
            .map(|_| syll[(r.next() % syll.len() as u64) as usize])
            .collect();
        if !v.contains(&w) {
            v.push(w);
        }
    }
    let weights: Vec<f64> = (1..=v.len()).map(|i| 1.0 / (i as f64).powf(1.1)).collect();
    let total: f64 = weights.iter().sum();
    let mut acc = 0.0;
    let cdf = weights.iter().map(|w| {
        acc += w / total;
        acc
    });
    (v, cdf.collect())
}

fn sentence(r: &mut Rng, v: &[String], cdf: &[f64], words: usize) -> String {
    (0..words)
        .map(|_| {
            let x = r.unit();
            v[cdf.partition_point(|c| *c < x).min(v.len() - 1)].as_str()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn pct(xs: &[f64], p: f64) -> f64 {
    let mut xs = xs.to_vec();
    xs.sort_by(f64::total_cmp);
    xs[((xs.len() as f64 - 1.0) * p).round() as usize]
}

fn time_ms<T>(f: impl FnOnce() -> T) -> (T, f64) {
    let t = Instant::now();
    let out = f();
    (out, t.elapsed().as_secs_f64() * 1000.0)
}

fn open(path: &Path, e: &dyn Embedder, mode: IndexMode) -> AgentStore {
    let opts = AgentOptions {
        index_mode: mode,
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

fn main() {
    let mut sizes: Vec<usize> = Vec::new();
    let mut models: Option<String> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == "--models" {
            models = args.next();
        } else if let Ok(n) = a.parse() {
            sizes.push(n);
        }
    }
    if sizes.is_empty() {
        sizes = vec![100_000];
    }
    let e: Arc<dyn Embedder> = match &models {
        Some(m) => default_embedder(Path::new(m)),
        None => Arc::new(HashEmbedder::new(384)),
    };
    let (vocab, cdf) = vocabulary();
    println!("embedder: {} ({}-d)", e.model_id(), e.dim());
    println!(
        "| Facts | exact KNN p50/p95 ms | ANN build s | sidecar MB | ANN KNN p50/p95 ms | recall@10 | exact hybrid p50/p95 ms | ANN hybrid p50/p95 ms | reopen w/ sidecar ms | live adds/s w/ ANN | Auto active |"
    );
    println!("|---|---|---|---|---|---|---|---|---|---|---|");
    for n in sizes {
        let tmp =
            tempfile::tempdir_in(std::env::var("TMP").unwrap_or_else(|_| ".".into())).unwrap();
        let path = tmp.path().join("agent.db");
        let mut r = Rng(0xA5A5_1234_DEAD_BEEF ^ n as u64);

        // Ingest without a sidecar (exact mode), one fact per transaction as the memory pipeline writes.
        {
            let store = open(&path, e.as_ref(), IndexMode::Exact);
            let t0 = Instant::now();
            let mut done = 0;
            while done < n {
                let texts: Vec<String> = (0..256.min(n - done))
                    .map(|_| {
                        let len = 10 + (r.next() % 11) as usize;
                        sentence(&mut r, &vocab, &cdf, len)
                    })
                    .collect();
                let vs = e.embed(&texts).unwrap();
                for (t, v) in texts.iter().zip(&vs) {
                    store
                        .add(&NewFact::fact(t.as_str(), "bench"), Some(v))
                        .unwrap();
                }
                done += texts.len();
                if done % 20_000 < 256 {
                    eprintln!(
                        "  ingested {done}/{n} ({:.0}/s)",
                        done as f64 / t0.elapsed().as_secs_f64()
                    );
                }
            }
        }

        let queries: Vec<String> = (0..200)
            .map(|_| {
                let len = 3 + (r.next() % 4) as usize;
                sentence(&mut r, &vocab, &cdf, len)
            })
            .collect();
        let qv = e.embed(&queries).unwrap();

        // Forced ANN on a database without a sidecar: `open` builds it.
        let (store, build_ms) = time_ms(|| open(&path, e.as_ref(), IndexMode::Ann));
        assert_eq!(store.index_kind(), IndexKind::Ann);
        let sidecar = std::fs::metadata(format!("{}.ann", path.display())).map_or(0, |m| m.len());
        let (mut ex, mut an, mut exh, mut anh) = (vec![], vec![], vec![], vec![]);
        let mut found = 0;
        for (q, v) in queries.iter().zip(&qv) {
            let (exact, t) = time_ms(|| store.knn_exact(v, 10).unwrap());
            ex.push(t);
            let (ann, t) = time_ms(|| store.knn(v, 10).unwrap());
            an.push(t);
            found += exact
                .iter()
                .filter(|h| ann.iter().any(|a| a.0 == h.0))
                .count();
            anh.push(time_ms(|| store.hybrid(q, Some(v), 8).unwrap()).1);
        }
        drop(store);
        let exact_store = open(&path, e.as_ref(), IndexMode::Exact);
        for (q, v) in queries.iter().zip(&qv) {
            exh.push(time_ms(|| exact_store.hybrid(q, Some(v), 8).unwrap()).1);
        }
        drop(exact_store);

        // Reopen with the saved sidecar (load + verify + journal replay), then live ingest through it.
        let (store, reopen_ms) = time_ms(|| open(&path, e.as_ref(), IndexMode::Ann));
        assert_eq!(
            store.index_status().builds,
            0,
            "the saved sidecar was not loaded"
        );
        let texts: Vec<String> = (0..2000)
            .map(|_| sentence(&mut r, &vocab, &cdf, 14))
            .collect();
        let vs = e.embed(&texts).unwrap();
        let t0 = Instant::now();
        for (t, v) in texts.iter().zip(&vs) {
            store
                .add(&NewFact::fact(t.as_str(), "bench"), Some(v))
                .unwrap();
        }
        let live_rate = texts.len() as f64 / t0.elapsed().as_secs_f64();
        drop(store);

        // The default options (Auto, 100 000): what an agent of this size gets.
        let auto = open(&path, e.as_ref(), IndexMode::Auto);
        auto.wait_for_index(std::time::Duration::from_secs(600));
        let auto_kind = auto.index_kind();
        drop(auto);

        println!(
            "| {n} | {:.2} / {:.2} | {:.1} | {:.0} | {:.3} / {:.3} | {:.4} | {:.2} / {:.2} | {:.2} / {:.2} | {:.0} | {:.0} | {auto_kind:?} |",
            pct(&ex, 0.5),
            pct(&ex, 0.95),
            build_ms / 1000.0,
            sidecar as f64 / 1e6,
            pct(&an, 0.5),
            pct(&an, 0.95),
            found as f64 / (queries.len() * 10) as f64,
            pct(&exh, 0.5),
            pct(&exh, 0.95),
            pct(&anh, 0.5),
            pct(&anh, 0.95),
            reopen_ms,
            live_rate,
        );
    }
}
