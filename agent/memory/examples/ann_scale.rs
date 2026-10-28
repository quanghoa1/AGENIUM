//! Memory at scale (ADR-125, `docs/reference/STORE-SCALE.md` §15): exact search against the ANN sidecar from 100 000
//! to 1 000 000 facts, on any machine, without the embedding model at run time.
//!
//! 1. `ann_scale embed --corpus <file> --models <dir> --out <vectors.bin> [--from-db <db>]` embeds every corpus
//!    sentence (bge-small) once into a flat little-endian f32 file, and the 500 sentences after them, cut to eight
//!    words, as queries into `<vectors.bin>.queries` (with `embed_query`). `--from-db` reuses the vectors of a database
//!    built by `ann_sweep` from the same corpus (fact id i+1 = sentence i).
//! 2. `ann_scale run --corpus <file> --vectors <vectors.bin> --db <dir> <n>…` stores `n` facts (resuming a kept
//!    database) and measures. Facts past the corpus are blends of two real ones: vector `normalize(w·a + (1-w)·b)`,
//!    `w` in [0.3, 0.7], text = first half of `a` + second half of `b` — they keep the clustered shape of real
//!    embeddings (random vectors would not), but they are not real sentences.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use agenium_memory::retrieve::{recall_with, RecallContext};
use agenium_store::{
    AgentOptions, AgentStore, AnnOptions, IndexKind, IndexMode, NewFact, StoreError,
};

const DIM: usize = 384;
const MODEL: &str = "bge-small-en-v1.5";

fn pct(xs: &[f64], p: f64) -> f64 {
    let mut xs = xs.to_vec();
    xs.sort_by(f64::total_cmp);
    xs[((xs.len() as f64 - 1.0) * p).round() as usize]
}

fn ms<T>(f: impl FnOnce() -> T) -> (T, f64) {
    let t = Instant::now();
    let out = f();
    (out, t.elapsed().as_secs_f64() * 1000.0)
}

fn read_vectors(path: &Path) -> Vec<Vec<f32>> {
    let b = std::fs::read(path).unwrap();
    b.as_chunks::<4>()
        .0
        .chunks(DIM)
        .map(|c| c.iter().map(|x| f32::from_le_bytes(*x)).collect())
        .collect()
}

fn write_vectors(path: &Path, vs: &[Vec<f32>]) {
    let mut f = std::io::BufWriter::new(std::fs::File::create(path).unwrap());
    for v in vs {
        for x in v {
            f.write_all(&x.to_le_bytes()).unwrap();
        }
    }
}

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

fn corpus(args: &[String]) -> Vec<String> {
    std::fs::read_to_string(arg(args, "--corpus").expect("--corpus <file>"))
        .unwrap()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(str::to_owned)
        .collect()
}

fn queries_of(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .map(|s| s.split_whitespace().take(8).collect::<Vec<_>>().join(" "))
        .collect()
}

fn embed(args: &[String]) {
    let lines = corpus(args);
    let e = agenium_memory::default_embedder(Path::new(
        &arg(args, "--models").expect("--models <dir>"),
    ));
    assert_eq!(e.model_id(), MODEL, "the bge-small model was not found");
    let out = PathBuf::from(arg(args, "--out").expect("--out <file>"));
    let facts = &lines[..lines.len() - 500];
    let mut vs: Vec<Option<Vec<f32>>> = vec![None; facts.len()];
    if let Some(db) = arg(args, "--from-db") {
        let s = AgentStore::open_with(
            Path::new(&db),
            DIM,
            MODEL,
            &Path::new(&db).with_extension("pre"),
            AgentOptions {
                index_mode: IndexMode::Exact,
                ..AgentOptions::default()
            },
        )
        .unwrap();
        let rows: Vec<(i64, Vec<u8>)> = s
            .db()
            .read(|c| {
                let mut st = c.prepare("SELECT rowid, embedding FROM fact_vec")?;
                let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
                Ok::<_, StoreError>(rows.collect::<Result<_, _>>()?)
            })
            .unwrap();
        for (id, b) in rows {
            let i = id as usize - 1;
            if i < vs.len() && s.get(id).map(|f| f.text == facts[i]).unwrap_or(false) {
                vs[i] = Some(
                    b.as_chunks::<4>()
                        .0
                        .iter()
                        .map(|x| f32::from_le_bytes(*x))
                        .collect(),
                );
            }
        }
        eprintln!("reused {} vectors from {db}", vs.iter().flatten().count());
    }
    // The rest, shortest first so a batch pads little, on a few threads.
    let mut todo: Vec<usize> = (0..facts.len()).filter(|i| vs[*i].is_none()).collect();
    todo.sort_by_key(|i| facts[*i].len());
    let t0 = Instant::now();
    for (n, block) in todo.chunks(1536).enumerate() {
        let parts: Vec<Vec<Vec<f32>>> = std::thread::scope(|s| {
            let hs: Vec<_> = block
                .chunks(block.len().div_ceil(3))
                .map(|part| {
                    let e = &e;
                    s.spawn(move || {
                        let texts: Vec<String> = part.iter().map(|i| facts[*i].clone()).collect();
                        e.embed(&texts).unwrap()
                    })
                })
                .collect();
            hs.into_iter().map(|h| h.join().unwrap()).collect()
        });
        for (i, v) in block.iter().zip(parts.into_iter().flatten()) {
            vs[*i] = Some(v);
        }
        let done = (n + 1) * 1536;
        eprintln!(
            "  embedded {}/{} ({:.0}/s)",
            done.min(todo.len()),
            todo.len(),
            done as f64 / t0.elapsed().as_secs_f64()
        );
    }
    write_vectors(
        &out,
        &vs.into_iter().map(Option::unwrap).collect::<Vec<_>>(),
    );
    let qs: Vec<Vec<f32>> = queries_of(&lines[lines.len() - 500..])
        .iter()
        .map(|q| e.embed_query(q).unwrap())
        .collect();
    write_vectors(&PathBuf::from(format!("{}.queries", out.display())), &qs);
    eprintln!(
        "wrote {} fact vectors and {} query vectors",
        facts.len(),
        qs.len()
    );
}

/// Fact `i` of the scale set: a corpus sentence, or past the corpus a blend of two (see the module comment).
fn fact(i: usize, texts: &[String], vs: &[Vec<f32>]) -> (String, Vec<f32>) {
    if i < vs.len() {
        return (texts[i].clone(), vs[i].clone());
    }
    let mut x = (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let mut next = || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    let (a, b) = (
        (next() % vs.len() as u64) as usize,
        (next() % vs.len() as u64) as usize,
    );
    let w = 0.3 + 0.4 * (next() >> 11) as f32 / (1u64 << 53) as f32;
    let mut v: Vec<f32> = vs[a]
        .iter()
        .zip(&vs[b])
        .map(|(x, y)| w * x + (1.0 - w) * y)
        .collect();
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    v.iter_mut().for_each(|x| *x /= n);
    let wa: Vec<&str> = texts[a].split_whitespace().collect();
    let wb: Vec<&str> = texts[b].split_whitespace().collect();
    let text = [&wa[..wa.len() / 2], &wb[wb.len() / 2..]]
        .concat()
        .join(" ");
    (text, v)
}

fn open(path: &Path, mode: IndexMode) -> AgentStore {
    let opts = AgentOptions {
        index_mode: mode,
        ann: AnnOptions::default(),
        ..AgentOptions::default()
    };
    AgentStore::open_with(path, DIM, MODEL, &path.with_extension("pre"), opts).unwrap()
}

/// Peak resident memory of this process (Linux), for the sidecar's cost.
fn peak_rss_mb() -> String {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmHWM:"))
                .and_then(|l| l.split_whitespace().nth(1)?.parse::<f64>().ok())
        })
        .map_or("n/a".into(), |kb| format!("{:.0}", kb / 1024.0))
}

fn run(args: &[String]) {
    let lines = corpus(args);
    let texts = &lines[..lines.len() - 500];
    let vectors = PathBuf::from(arg(args, "--vectors").expect("--vectors <file>"));
    let vs = read_vectors(&vectors);
    assert_eq!(vs.len(), texts.len(), "vectors do not match the corpus");
    let qv = read_vectors(&PathBuf::from(format!("{}.queries", vectors.display())));
    let qt = queries_of(&lines[lines.len() - 500..]);
    let nq: usize = arg(args, "--queries").map_or(200, |q| q.parse().unwrap());
    let dir = PathBuf::from(arg(args, "--db").expect("--db <dir>"));
    std::fs::create_dir_all(&dir).unwrap();
    let sizes: Vec<usize> = args
        .iter()
        .filter_map(|a| a.parse().ok())
        .filter(|n| *n >= 1000)
        .collect();
    println!(
        "machine: {} threads; {} real facts, {} queries",
        std::thread::available_parallelism().map_or(0, |n| n.get()),
        vs.len(),
        nq
    );
    println!("| Facts | ingest/s | exact KNN p50/p95 ms | ANN build s | sidecar MB | ANN KNN p50/p95 ms | recall@10 | hybrid exact → ANN p50 ms | hybrid top-8 = exact | recall exact → ANN p50 ms | recall top-8 = exact | reopen ms | live adds/s | peak RSS MB |");
    println!("|---|---|---|---|---|---|---|---|---|---|---|---|---|---|");
    for n in sizes {
        let path = dir.join(format!("scale-{n}.db"));
        let ingest = {
            let s = open(&path, IndexMode::Exact);
            let have = s.count_live().unwrap() as usize;
            let t0 = Instant::now();
            for i in have..n {
                let (t, v) = fact(i, texts, &vs);
                s.add(&NewFact::fact(t, "scale"), Some(&v)).unwrap();
                if (i + 1) % 100_000 == 0 {
                    eprintln!("  {n}: stored {}", i + 1);
                }
            }
            if n > have {
                format!("{:.0}", (n - have) as f64 / t0.elapsed().as_secs_f64())
            } else {
                "kept".into()
            }
        };
        let ctx = RecallContext {
            project: None,
            scope: serde_json::json!({ "project": "scale" }),
        };
        type Seen = (Vec<Vec<i64>>, Vec<Vec<i64>>, Vec<Vec<i64>>, [Vec<f64>; 3]);
        let measure = |s: &AgentStore| -> Seen {
            let (mut knn, mut hy, mut rc, mut t) =
                (vec![], vec![], vec![], [vec![], vec![], vec![]]);
            for (q, v) in qt.iter().zip(&qv).take(nq) {
                let (r, a) = ms(|| s.knn(v, 10).unwrap());
                knn.push(r.into_iter().map(|h| h.0).collect());
                let (r, b) = ms(|| s.hybrid(q, Some(v), 8).unwrap());
                hy.push(r.into_iter().map(|h| h.fact.id).collect());
                let (r, c) = ms(|| recall_with(s, v, q, &ctx, 8).unwrap());
                rc.push(r.into_iter().map(|h| h.id).collect());
                for (x, y) in t.iter_mut().zip([a, b, c]) {
                    x.push(y);
                }
            }
            (knn, hy, rc, t)
        };
        eprintln!("  {n}: exact queries");
        let exact = measure(&open(&path, IndexMode::Exact));
        for f in agenium_store::ann::sidecar_files(&path) {
            let _ = std::fs::remove_file(f);
        }
        eprintln!("  {n}: building the sidecar");
        let (store, build_ms) = ms(|| open(&path, IndexMode::Ann));
        assert_eq!(store.index_kind(), IndexKind::Ann);
        let sidecar = std::fs::metadata(format!("{}.ann", path.display())).map_or(0, |m| m.len());
        let ann = measure(&store);
        drop(store);
        let (store, reopen_ms) = ms(|| open(&path, IndexMode::Ann));
        assert_eq!(
            store.index_status().builds,
            0,
            "the saved sidecar was not loaded"
        );
        let t0 = Instant::now();
        for i in 0..2000 {
            let (t, v) = fact(vs.len() + 10_000_000 + i, texts, &vs);
            store
                .add(&NewFact::fact(t, "scale-live"), Some(&v))
                .unwrap();
        }
        let live = 2000.0 / t0.elapsed().as_secs_f64();
        // Leave the database as it was for a later run.
        let added: Vec<i64> = store
            .live(2000)
            .unwrap()
            .into_iter()
            .filter(|f| f.source == "scale-live")
            .map(|f| f.id)
            .collect();
        for id in added {
            store.retire(id).unwrap();
        }
        drop(store);
        let agree = |a: &[Vec<i64>], b: &[Vec<i64>]| {
            let (mut same, mut total) = (0, 0);
            for (x, y) in a.iter().zip(b) {
                same += x.iter().filter(|id| y.contains(id)).count();
                total += x.len().max(y.len());
            }
            same as f64 / total.max(1) as f64
        };
        println!(
            "| {n} | {ingest} | {:.1} / {:.1} | {:.1} | {:.0} | {:.2} / {:.2} | {:.4} | {:.1} → {:.1} | {:.4} | {:.1} → {:.1} | {:.4} | {:.0} | {:.0} | {} |",
            pct(&exact.3[0], 0.5),
            pct(&exact.3[0], 0.95),
            build_ms / 1000.0,
            sidecar as f64 / 1e6,
            pct(&ann.3[0], 0.5),
            pct(&ann.3[0], 0.95),
            agree(&exact.0, &ann.0),
            pct(&exact.3[1], 0.5),
            pct(&ann.3[1], 0.5),
            agree(&exact.1, &ann.1),
            pct(&exact.3[2], 0.5),
            pct(&ann.3[2], 0.5),
            agree(&exact.2, &ann.2),
            reopen_ms,
            live,
            peak_rss_mb(),
        );
        std::io::stdout().flush().unwrap();
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("embed") => embed(&args),
        Some("run") => run(&args),
        _ => eprintln!("usage: ann_scale embed … | ann_scale run …"),
    }
}
