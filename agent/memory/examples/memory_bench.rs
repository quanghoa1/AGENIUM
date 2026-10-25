//! Memory-store benchmark on the rewritten store — re-verifies `docs/reference/STORE-SCALE.md` §2/§4, which were
//! measured on the trial implementation. Everything goes through the production API: `AgentStore::{add, knn, keyword,
//! hybrid}` and the full `retrieve::recall` path (vector + BM25 + graph + scope filter + re-rank).
//!
//! `cargo run --release -p agenium-memory --example memory_bench -- [sizes …] [--readers-secs N]`
//! (default sizes: 10000 100000; default concurrency window: 10 s)
//!
//! Data is a synthetic stand-in (the trial's real 24 k-sentence corpus is not in the pack): sentences of 10–20 words drawn
//! from a 6 000-word Zipf vocabulary (s = 1.1) so BM25 sees natural-looking term frequencies, and 384-d vectors from the
//! hashing embedder. Exact-KNN latency does not depend on the vector distribution; recall quality is measured elsewhere.

use std::sync::Arc;
use std::time::{Duration, Instant};

use agenium_memory::retrieve::{recall, RecallContext};
use agenium_memory::{Embedder, HashEmbedder};
use agenium_store::{AgentStore, NewFact};

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

/// A 6 000-word vocabulary (some real engineering words first, then pronounceable pseudo-words) and its Zipf CDF.
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

fn word<'a>(r: &mut Rng, v: &'a [String], cdf: &[f64]) -> &'a str {
    let x = r.unit();
    let i = cdf.partition_point(|c| *c < x).min(v.len() - 1);
    &v[i]
}

fn sentence(r: &mut Rng, v: &[String], cdf: &[f64], words: usize) -> String {
    (0..words)
        .map(|_| word(r, v, cdf))
        .collect::<Vec<_>>()
        .join(" ")
}

fn pct(xs: &mut [f64], p: f64) -> f64 {
    if xs.is_empty() {
        return f64::NAN;
    }
    xs.sort_by(f64::total_cmp);
    xs[((xs.len() as f64 - 1.0) * p).round() as usize]
}

fn time_ms<T>(f: impl FnOnce() -> T) -> (T, f64) {
    let t = Instant::now();
    let out = f();
    (out, t.elapsed().as_secs_f64() * 1000.0)
}

fn dir_size(p: &std::path::Path) -> u64 {
    std::fs::read_dir(p)
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| e.metadata().ok())
                .filter(|m| m.is_file())
                .map(|m| m.len())
                .sum()
        })
        .unwrap_or(0)
}

fn main() {
    let mut secs = 10u64;
    let mut sizes: Vec<usize> = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == "--readers-secs" {
            secs = args.next().and_then(|s| s.parse().ok()).unwrap_or(secs);
        } else if let Ok(n) = a.parse() {
            sizes.push(n);
        }
    }
    if sizes.is_empty() {
        sizes = vec![10_000, 100_000];
    }
    let e: Arc<dyn Embedder> = Arc::new(HashEmbedder::new(384));
    let (vocab, cdf) = vocabulary();
    println!(
        "| Facts | Ingest/s (1 txn per add) | DB on disk | KNN p50/p95 ms | BM25 p50/p95 ms | hybrid p50/p95 ms | recall p50/p95 ms | needle | reopen ms | 4R+1W: reader hybrid p95 ms, writer adds/s |"
    );
    println!("|---|---|---|---|---|---|---|---|---|---|");
    for n in sizes {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("agent.db");
        let store =
            Arc::new(AgentStore::open(&path, 384, e.model_id(), &tmp.path().join("pre")).unwrap());
        let mut r = Rng(0xA5A5_1234_DEAD_BEEF ^ n as u64);

        // Ingest, the way the memory pipeline writes: one fact per transaction.
        let mut texts = Vec::with_capacity(1000);
        let t0 = Instant::now();
        let mut done = 0;
        while done < n {
            texts.clear();
            for _ in 0..1000.min(n - done) {
                let len = 10 + (r.next() % 11) as usize;
                texts.push(sentence(&mut r, &vocab, &cdf, len));
            }
            let vs = e.embed(&texts).unwrap();
            for (t, v) in texts.iter().zip(&vs) {
                store
                    .add(&NewFact::fact(t.as_str(), "bench"), Some(v))
                    .unwrap();
            }
            done += texts.len();
        }
        let ingest = n as f64 / t0.elapsed().as_secs_f64();
        let size = dir_size(tmp.path());

        // 200 queries of 3–6 words from the same distribution.
        let queries: Vec<String> = (0..200)
            .map(|_| {
                let len = 3 + (r.next() % 4) as usize;
                sentence(&mut r, &vocab, &cdf, len)
            })
            .collect();
        let qv = e.embed(&queries).unwrap();
        let (mut knn, mut kw, mut hy, mut rc) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        let ctx = RecallContext {
            project: None,
            scope: serde_json::json!({}),
        };
        for (q, v) in queries.iter().zip(&qv) {
            knn.push(time_ms(|| store.knn(v, 50).unwrap()).1);
            kw.push(time_ms(|| store.keyword(q, 50).unwrap()).1);
            hy.push(time_ms(|| store.hybrid(q, Some(v), 8).unwrap()).1);
            rc.push(time_ms(|| recall(&store, e.as_ref(), q, &ctx, 8).unwrap()).1);
        }

        // Needle: a unique token whose vector points elsewhere must still be found by hybrid (BM25 side).
        let mut found = 0;
        for i in 0..50 {
            let token = format!("zqneedle{i}x{n}");
            let other = e
                .embed(&[sentence(&mut r, &vocab, &cdf, 12)])
                .unwrap()
                .pop()
                .unwrap();
            let id = store
                .add(
                    &NewFact::fact(
                        format!("{} {token}", sentence(&mut r, &vocab, &cdf, 8)),
                        "bench",
                    ),
                    Some(&other),
                )
                .unwrap();
            let q = e
                .embed(&[sentence(&mut r, &vocab, &cdf, 5)])
                .unwrap()
                .pop()
                .unwrap();
            if store
                .hybrid(&token, Some(&q), 8)
                .unwrap()
                .iter()
                .any(|h| h.fact.id == id)
            {
                found += 1;
            }
        }

        // Reopen (before the concurrency phase, so it does not include WAL recovery of that phase's writes).
        drop(store);
        let (store, reopen) = time_ms(|| {
            Arc::new(AgentStore::open(&path, 384, e.model_id(), &tmp.path().join("pre")).unwrap())
        });

        // Concurrency: 4 readers running hybrid while 1 writer adds facts.
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (reader_p95, writer_rate) = std::thread::scope(|s| {
            let mut readers = Vec::new();
            for t in 0..4u64 {
                let (store, stop, queries, qv) = (store.clone(), stop.clone(), &queries, &qv);
                readers.push(s.spawn(move || {
                    let mut lat = Vec::new();
                    let mut i = t as usize * 37;
                    while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                        let k = i % queries.len();
                        lat.push(time_ms(|| store.hybrid(&queries[k], Some(&qv[k]), 8).unwrap()).1);
                        i += 1;
                    }
                    lat
                }));
            }
            let writer = {
                let (store, stop, e) = (store.clone(), stop.clone(), e.clone());
                let mut wr = Rng(42 ^ n as u64);
                let (vocab, cdf) = (&vocab, &cdf);
                s.spawn(move || {
                    let mut added = 0u64;
                    while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                        let t = sentence(&mut wr, vocab, cdf, 14);
                        let v = e.embed(std::slice::from_ref(&t)).unwrap().pop().unwrap();
                        store
                            .add(&NewFact::fact(t.as_str(), "bench"), Some(&v))
                            .unwrap();
                        added += 1;
                    }
                    added
                })
            };
            std::thread::sleep(Duration::from_secs(secs));
            stop.store(true, std::sync::atomic::Ordering::Relaxed);
            let mut all: Vec<f64> = readers
                .into_iter()
                .flat_map(|h| h.join().unwrap())
                .collect();
            let added = writer.join().unwrap();
            (pct(&mut all, 0.95), added as f64 / secs as f64)
        });

        println!(
            "| {n} | {:.0} | {:.0} MB | {:.2} / {:.2} | {:.2} / {:.2} | {:.2} / {:.2} | {:.2} / {:.2} | {found}/50 | {:.0} | {:.1}, {:.0} |",
            ingest,
            size as f64 / 1e6,
            pct(&mut knn.clone(), 0.5),
            pct(&mut knn, 0.95),
            pct(&mut kw.clone(), 0.5),
            pct(&mut kw, 0.95),
            pct(&mut hy.clone(), 0.5),
            pct(&mut hy, 0.95),
            pct(&mut rc.clone(), 0.5),
            pct(&mut rc, 0.95),
            reopen,
            reader_p95,
            writer_rate
        );
    }
}
