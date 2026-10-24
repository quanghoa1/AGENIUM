//! Graph-memory traversal benchmark (ADR-51 §13 item 1, M15-10) on the real Rust store.
//!
//! `cargo run --release -p agenium-store --example graph_bench -- [edges ...]` (default: 100000 1000000)
//!
//! Synthetic graph per size: entities = edges / 3, destinations drawn with a skewed (Zipf-like) distribution, 10% of
//! edges scoped to vendor tp-link or netgear, context vendor=netgear; one extra hub with 2 000 incoming edges.
//! For depth 1..3: 100 random seeds and 100 hub-biased seeds (the 1% highest in-degree), hub cap on (200) and off,
//! no time budget (raw latency) plus the share of queries that would exceed the 25 ms budget. Warm cache.

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

fn main() {
    let sizes: Vec<usize> = std::env::args()
        .skip(1)
        .filter_map(|a| a.parse().ok())
        .collect::<Vec<_>>();
    let sizes = if sizes.is_empty() {
        vec![100_000, 1_000_000]
    } else {
        sizes
    };
    println!("| edges | seeds | hub cap | depth | p50 ms | p95 ms | p99 ms | max ms | > 25 ms | avg nodes |");
    println!("|---|---|---|---|---|---|---|---|---|---|");
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
    }
}
