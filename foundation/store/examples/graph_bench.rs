//! Graph-memory traversal benchmark (ADR-51 §13 item 1, M15-10) on the real Rust store.
//!
//! `cargo run --release -p agenium-store --example graph_bench -- [edges ...] [--readers-secs N]`
//! (default: 100000 1000000; concurrency window 10 s)
//!
//! Synthetic graph per size: entities = edges / 3, destinations drawn with a skewed (Zipf-like) distribution, 10% of
//! edges scoped to vendor tp-link or netgear, context vendor=netgear; one extra hub with 2 000 incoming edges.
//! For depth 1..3: 100 random seeds and 100 hub-biased seeds (the 1% highest in-degree), hub cap on (200) and off,
//! no time budget (raw latency) plus the share of queries that would exceed the 25 ms budget. Warm cache.
//!
//! Then, per size, the cases ADR-51 §13 listed and the first run left out (depth 2, random seeds, hub cap 200, the
//! production budget of 25 ms): the first 100 queries on a freshly opened store (SQLite's page cache empty; the OS
//! cache is not dropped, which needs administrator rights), edge ingest through `add_edge` alone (one transaction
//! per edge, as the memory pipeline writes), 4 readers alone, and 4 readers while 1 writer adds edges.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use agenium_store::graph::Expansion;
use agenium_store::AgentStore;
use rusqlite::params;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
}

fn pct(v: &mut [f64], p: f64) -> f64 {
    v.sort_by(|a, b| a.total_cmp(b));
    let i = ((v.len() as f64 - 1.0) * p).round() as usize;
    v[i]
}

fn build(dir: &std::path::Path, edges: usize) -> (AgentStore, Vec<i64>, i64) {
    let s = AgentStore::open(&dir.join("bench.db"), 4, "m", &dir.join("backups")).unwrap();
    let n = (edges / 3).max(10) as i64;
    let mut r = Rng(0x9e37_79b9_7f4a_7c15 ^ edges as u64);
    let t = Instant::now();
    s.db()
        .write(|tx| -> Result<(), agenium_store::StoreError> {
            {
                let mut ins = tx.prepare(
                    "INSERT INTO entity(id, type, name, canonical_key, scope_key) VALUES (?1, 'concept', ?2, ?2, '')",
                )?;
                for i in 1..=n + 1 {
                    ins.execute(params![i, format!("e{i}")])?;
                }
                let mut ins = tx.prepare(
                    "INSERT OR IGNORE INTO edge(src, dst, type, scope, scope_key, confidence) VALUES (?1, ?2, ?3, ?4, ?5, 0.8)",
                )?;
                let types = ["uses", "depends_on", "causes", "fixes", "part_of", "related_to"];
                for _ in 0..edges {
                    let src = 1 + r.below(n as u64) as i64;
                    // Zipf-like destination: low ids are popular.
                    let dst = 1 + ((n as f64) * r.unit().powf(3.0)) as i64 % n;
                    if src == dst {
                        continue;
                    }
                    let (scope, sk) = match r.below(20) {
                        0 => ("{\"vendor\":\"tp link\"}", "vendor=tp link"),
                        1 => ("{\"vendor\":\"netgear\"}", "vendor=netgear"),
                        _ => ("{}", ""),
                    };
                    ins.execute(params![src, dst, types[r.below(6) as usize], scope, sk])?;
                }
                // The hub: entity n + 1 with 2 000 incoming edges.
                for k in 0..2000 {
                    ins.execute(params![1 + (k * 7919) % n, n + 1, "related_to", "{}", ""])?;
                }
            }
            tx.execute_batch(
                "CREATE TEMP TABLE deg AS SELECT x AS id, count(*) AS c
                   FROM (SELECT src AS x FROM edge WHERE valid_to IS NULL UNION ALL SELECT dst FROM edge WHERE valid_to IS NULL)
                   GROUP BY x;
                 CREATE UNIQUE INDEX temp.deg_id ON deg(id);
                 UPDATE entity SET degree = COALESCE((SELECT c FROM deg WHERE deg.id = entity.id), 0);
                 DROP TABLE deg;
                 ANALYZE;",
            )?;
            Ok(())
        })
        .unwrap();
    let live: i64 = s
        .db()
        .read(|c| -> Result<i64, agenium_store::StoreError> {
            Ok(c.query_row("SELECT count(*) FROM edge", [], |r| r.get(0))?)
        })
        .unwrap();
    eprintln!(
        "built {live} edges / {} entities in {:.1} s",
        n + 1,
        t.elapsed().as_secs_f64()
    );
    let hubs: Vec<i64> = s
        .db()
        .read(|c| -> Result<Vec<i64>, agenium_store::StoreError> {
            let mut st = c.prepare("SELECT id FROM entity ORDER BY degree DESC LIMIT ?1")?;
            let v = st
                .query_map([(n / 100).max(1)], |r| r.get(0))?
                .collect::<Result<_, _>>()?;
            Ok(v)
        })
        .unwrap();
    (s, hubs, n)
}

/// Depth-2 expansion from random seeds with the production settings; returns the latency in ms.
fn query(s: &AgentStore, r: &mut Rng, n: i64, ctx: &serde_json::Value) -> f64 {
    let seed = 1 + r.below(n as u64) as i64;
    let t = Instant::now();
    s.expand(&[seed], ctx, &Expansion::default()).unwrap();
    t.elapsed().as_secs_f64() * 1000.0
}

fn summary(v: &mut [f64]) -> String {
    if v.is_empty() {
        return "-".into();
    }
    let max = v.iter().cloned().fold(0.0, f64::max);
    format!(
        "{:.2} / {:.2} / {max:.1}",
        pct(&mut v.to_vec(), 0.5),
        pct(&mut v.to_vec(), 0.95)
    )
}

/// `readers` threads querying for `secs` while (optionally) one thread adds edges: reader latencies, edges/s.
fn concurrent(
    s: &Arc<AgentStore>,
    n: i64,
    readers: usize,
    writer: bool,
    secs: u64,
) -> (Vec<f64>, f64) {
    let stop = Arc::new(AtomicBool::new(false));
    let written = Arc::new(AtomicU64::new(0));
    let mut handles = Vec::new();
    for k in 0..readers {
        let (s, stop) = (s.clone(), stop.clone());
        handles.push(std::thread::spawn(move || {
            let mut r = Rng(0x5eed + k as u64);
            let ctx = serde_json::json!({"vendor": "netgear"});
            let mut lat = Vec::new();
            while !stop.load(Ordering::Relaxed) {
                lat.push(query(&s, &mut r, n, &ctx));
            }
            lat
        }));
    }
    let w = writer.then(|| {
        let (s, stop, written) = (s.clone(), stop.clone(), written.clone());
        std::thread::spawn(move || {
            let mut r = Rng(0xfeed);
            while !stop.load(Ordering::Relaxed) {
                let (a, b) = (1 + r.below(n as u64) as i64, 1 + r.below(n as u64) as i64);
                if a != b
                    && s.add_edge(
                        a,
                        b,
                        "related_to",
                        &serde_json::json!({}),
                        0.8,
                        None,
                        "bench",
                    )
                    .is_ok()
                {
                    written.fetch_add(1, Ordering::Relaxed);
                }
            }
        })
    });
    let t = Instant::now();
    std::thread::sleep(Duration::from_secs(secs));
    stop.store(true, Ordering::Relaxed);
    let lat: Vec<f64> = handles
        .into_iter()
        .flat_map(|h| h.join().unwrap())
        .collect();
    if let Some(w) = w {
        w.join().unwrap();
    }
    (
        lat,
        written.load(Ordering::Relaxed) as f64 / t.elapsed().as_secs_f64(),
    )
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let secs: u64 = args
        .iter()
        .position(|a| a == "--readers-secs")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(10);
    let sizes: Vec<usize> = args
        .iter()
        .enumerate()
        .filter(|(i, _)| *i == 0 || args[i - 1] != "--readers-secs")
        .filter_map(|(_, a)| a.parse().ok())
        .collect::<Vec<_>>();
    let sizes = if sizes.is_empty() {
        vec![100_000, 1_000_000]
    } else {
        sizes
    };
    println!("| edges | seeds | hub cap | depth | p50 ms | p95 ms | p99 ms | max ms | > 25 ms | avg nodes |");
    println!("|---|---|---|---|---|---|---|---|---|---|");
    let mut later = Vec::new();
    for edges in sizes {
        let tmp = tempfile::tempdir().unwrap();
        let (s, hubs, n) = build(tmp.path(), edges);
        let ctx = serde_json::json!({"vendor": "netgear"});
        let mut r = Rng(42);
        for (label, cap) in [("on (200)", 200), ("off", i64::MAX)] {
            for seeds in ["random", "hub-biased"] {
                for depth in 1..=3u32 {
                    let x = Expansion {
                        max_depth: depth,
                        hub_degree: cap,
                        limit: 50,
                        budget: Duration::from_secs(60),
                    };
                    let mut lat = Vec::new();
                    let mut nodes = 0usize;
                    for q in 0..100 {
                        let seed = if seeds == "random" {
                            1 + r.below(n as u64) as i64
                        } else {
                            hubs[q % hubs.len()]
                        };
                        let t = Instant::now();
                        let res = s.expand(&[seed], &ctx, &x).unwrap();
                        lat.push(t.elapsed().as_secs_f64() * 1000.0);
                        nodes += res.reached.len();
                    }
                    let over = lat.iter().filter(|l| **l > 25.0).count();
                    let max = lat.iter().cloned().fold(0.0, f64::max);
                    println!(
                        "| {edges} | {seeds} | {label} | {depth} | {:.2} | {:.2} | {:.2} | {max:.1} | {over}% | {:.1} |",
                        pct(&mut lat.clone(), 0.5),
                        pct(&mut lat.clone(), 0.95),
                        pct(&mut lat, 0.99),
                        nodes as f64 / 100.0
                    );
                }
            }
        }
        // The single big hub as a seed (cap off) at depth 1..3.
        for depth in 1..=3u32 {
            let x = Expansion {
                max_depth: depth,
                hub_degree: i64::MAX,
                limit: 50,
                budget: Duration::from_secs(60),
            };
            let t = Instant::now();
            let res = s.expand(&[n + 1], &ctx, &x).unwrap();
            println!(
                "| {edges} | the 2000-edge hub | off | {depth} | {:.2} | - | - | - | - | {} |",
                t.elapsed().as_secs_f64() * 1000.0,
                res.reached.len()
            );
        }
        later.push((edges, tmp, s, n));
    }

    println!();
    println!("Depth 2, random seeds, hub cap 200, 25 ms budget (the production settings); {secs} s per concurrent case:");
    println!("| edges | case | query p50 / p95 / max ms | queries | edges written/s |");
    println!("|---|---|---|---|---|");
    for (edges, tmp, s, n) in later {
        // Freshly opened: SQLite's page cache is empty (the OS cache is not).
        drop(s);
        let s = Arc::new(
            AgentStore::open(
                &tmp.path().join("bench.db"),
                4,
                "m",
                &tmp.path().join("backups"),
            )
            .unwrap(),
        );
        let ctx = serde_json::json!({"vendor": "netgear"});
        let mut r = Rng(7);
        let mut lat: Vec<f64> = (0..100).map(|_| query(&s, &mut r, n, &ctx)).collect();
        println!(
            "| {edges} | first 100 queries after open | {} | 100 | - |",
            summary(&mut lat)
        );
        let t = Instant::now();
        let mut written = 0;
        while written < 2000 {
            let (a, b) = (1 + r.below(n as u64) as i64, 1 + r.below(n as u64) as i64);
            if a != b
                && s.add_edge(
                    a,
                    b,
                    "related_to",
                    &serde_json::json!({}),
                    0.8,
                    None,
                    "bench",
                )
                .is_ok()
            {
                written += 1;
            }
        }
        println!(
            "| {edges} | add_edge alone (one transaction each) | - | - | {:.0} |",
            written as f64 / t.elapsed().as_secs_f64()
        );
        let (mut lat, _) = concurrent(&s, n, 4, false, secs);
        let count = lat.len();
        println!(
            "| {edges} | 4 readers | {} | {count} | - |",
            summary(&mut lat)
        );
        let (mut lat, rate) = concurrent(&s, n, 4, true, secs);
        let count = lat.len();
        println!(
            "| {edges} | 4 readers + 1 writer | {} | {count} | {rate:.0} |",
            summary(&mut lat)
        );
    }
}
