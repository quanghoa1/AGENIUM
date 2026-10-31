//! Sweeps the ranking parameters of `tool_search` (ADR-139) offline, on several sets of tools and requests at once: a
//! change counts only when it holds on every set. No model API is called; meaning is the local `bge-small-en-v1.5`.
//!
//! Swept: BM25 `k1` and `b`, how many times the tool's name counts, the text embedded (the BM25 text, the name and
//! whole description, or `rank::meaning_text`), the reciprocal-rank constant `k`, the weight of meaning against words,
//! how deep the meaning ranking goes (its first 30, as shipped, or all), and a third ranking tried and left out: the
//! place of the tool's server by meaning. The words are the shipped ones (`rank::words`: common words out, stems).
//! `--misses` prints the requests whose right tool is not among the 5 a search returns.
//!
//! Usage: `cargo run --release -p agenium-eval --example tool_search_sweep -- --models <models root> [--misses]
//!   --set <tools.json> <queries.json> [--set …]`. A tools file is a list of `{server?, name, description,
//! inputSchema}` (server `many` when absent); a query's `expect` is a tool name or a list of `server:name`. The sets of
//! ADR-139: `agent/mcp/data/many-tools*.json`, and `real-tools.json` (fetch it with `make_real_tools.py`) with
//! `real-tools-queries.json`.

use std::collections::HashMap;
use std::path::PathBuf;

use agenium_mcp::rank;
use agenium_memory::local::{LocalEmbedder, BGE_SMALL_EN};
use agenium_memory::Embedder;
use anyhow::{anyhow, Result};
use serde_json::Value;

/// What is embedded for a tool: the BM25 text (shipped until 2026-10-31), its name and description, or
/// `rank::meaning_text` (its name and the description's first 300 characters, shipped since).
const EMBEDS: [&str; 3] = ["bm25-text", "name+desc", "meaning-text"];

struct Tool {
    key: String,
    server: String,
    name: String,
    desc: String,
    schema: Value,
}

struct Query {
    text: String,
    expect: Vec<String>,
}

struct Set {
    label: String,
    tools: Vec<Tool>,
    queries: Vec<Query>,
    /// Each tool's place by meaning, per embedded-text variant and query.
    meaning: Vec<Vec<Vec<u32>>>,
    /// The place of each tool's server by meaning (the mean of its 3 closest tools), per variant and query.
    servers: Vec<Vec<Vec<u32>>>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Params {
    k1: f64,
    b: f64,
    name_times: usize,
    embed: usize,
    rrf_k: f64,
    meaning_w: f64,
    meaning_depth: usize,
    /// Weight of the server's place (0: off, as shipped).
    server_w: f64,
}

const RRF_KS: [f64; 8] = [1.0, 2.0, 3.0, 5.0, 10.0, 20.0, 60.0, 100.0];

const SHIPPED: Params = Params {
    k1: 1.2,
    b: 0.75,
    name_times: 2,
    embed: 2,
    rrf_k: 10.0,
    meaning_w: 1.5,
    meaning_depth: 30,
    server_w: 0.0,
};

/// As shipped until 2026-10-31: the BM25 text embedded, the textbook k = 60, words and meaning weighed alike.
const BEFORE: Params = Params {
    embed: 0,
    rrf_k: 60.0,
    meaning_w: 1.0,
    ..SHIPPED
};

/// A tool's fused score from its places by words, by meaning and of its server (0 = not ranked).
fn fuse_one(p: Params, w: u32, m: u32, sv: u32) -> f64 {
    let mut f = 0.0;
    if w > 0 {
        f += 1.0 / (p.rrf_k + w as f64);
    }
    if m > 0 && (m as usize) <= p.meaning_depth {
        f += p.meaning_w / (p.rrf_k + m as f64);
    }
    if f > 0.0 && p.server_w > 0.0 && sv > 0 {
        f += p.server_w / (p.rrf_k + sv as f64);
    }
    f
}

/// Each tool's server place for one query: servers ranked by the mean cosine of their 3 closest tools.
fn server_places(tools: &[Tool], cos: &[f64]) -> Vec<u32> {
    let mut by: HashMap<&str, Vec<f64>> = HashMap::new();
    for (t, c) in tools.iter().zip(cos) {
        by.entry(t.server.as_str()).or_default().push(*c);
    }
    let mut score: Vec<(&str, f64)> = by
        .into_iter()
        .map(|(srv, mut v)| {
            v.sort_by(|a, b| b.total_cmp(a));
            let top = &v[..v.len().min(3)];
            (srv, top.iter().sum::<f64>() / top.len() as f64)
        })
        .collect();
    score.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(b.0)));
    let place: HashMap<&str, u32> = score
        .iter()
        .enumerate()
        .map(|(i, (srv, _))| (*srv, i as u32 + 1))
        .collect();
    tools.iter().map(|t| place[t.server.as_str()]).collect()
}

#[derive(Clone, Copy, Default)]
struct Score {
    first: usize,
    in3: usize,
    in5: usize,
    mrr: f64,
    n: usize,
}

fn load_set(label: &str, tools: &str, queries: &str) -> Result<(Vec<Tool>, Vec<Query>)> {
    let t: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(tools)?)?;
    let tools: Vec<Tool> = t
        .into_iter()
        .map(|v| {
            let server = v["server"].as_str().unwrap_or("many").to_owned();
            let name = v["name"].as_str().unwrap_or_default().to_owned();
            Tool {
                key: format!("{server}:{name}"),
                server: server.clone(),
                name: format!("{server} {name}"),
                desc: v["description"].as_str().unwrap_or_default().to_owned(),
                schema: v["inputSchema"].clone(),
            }
        })
        .collect();
    let q: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(queries)?)?;
    let queries = q
        .into_iter()
        .map(|v| Query {
            text: v["query"].as_str().unwrap_or_default().to_owned(),
            expect: match &v["expect"] {
                Value::String(s) => vec![format!("many:{s}")],
                Value::Array(a) => a
                    .iter()
                    .filter_map(|x| x.as_str().map(str::to_owned))
                    .collect(),
                _ => vec![],
            },
        })
        .collect::<Vec<_>>();
    for q in &queries {
        for e in &q.expect {
            if !tools.iter().any(|t| &t.key == e) {
                return Err(anyhow!("{label}: {e} (for {:?}) is not a tool", q.text));
            }
        }
    }
    Ok((tools, queries))
}

/// The BM25 text of a tool with its name counted `times` times (`rank::tool_text` counts it twice).
fn text(t: &Tool, times: usize) -> String {
    let rest = rank::tool_text("", &t.desc, &t.schema);
    format!("{} {rest}", vec![t.name.as_str(); times].join(" "))
}

/// BM25 scores of every query against every tool.
fn bm25(texts: &[String], queries: &[Query], k1: f64, b: f64) -> Vec<Vec<f64>> {
    let docs: Vec<HashMap<String, u32>> = texts
        .iter()
        .map(|t| {
            let mut tf = HashMap::new();
            for w in rank::words(t) {
                *tf.entry(w).or_insert(0) += 1;
            }
            tf
        })
        .collect();
    let lens: Vec<f64> = texts.iter().map(|t| rank::words(t).len() as f64).collect();
    let avg = (lens.iter().sum::<f64>() / lens.len().max(1) as f64).max(1.0);
    let mut df: HashMap<&str, u32> = HashMap::new();
    for d in &docs {
        for k in d.keys() {
            *df.entry(k.as_str()).or_insert(0) += 1;
        }
    }
    let n = docs.len() as f64;
    queries
        .iter()
        .map(|q| {
            let mut words = rank::words(&q.text);
            words.sort();
            words.dedup();
            docs.iter()
                .zip(&lens)
                .map(|(tf, len)| {
                    words
                        .iter()
                        .filter_map(|w| {
                            let f = *tf.get(w)? as f64;
                            let d = *df.get(w.as_str())? as f64;
                            let idf = (1.0 + (n - d + 0.5) / (d + 0.5)).ln();
                            Some(idf * f * (k1 + 1.0) / (f + k1 * (1.0 - b + b * len / avg)))
                        })
                        .sum()
                })
                .collect()
        })
        .collect()
}

/// Each tool's place in a ranking (1-based, 0 = not ranked), per query.
type Places = Vec<Vec<u32>>;

/// Each tool's place (1-based, 0 = not ranked) in a ranking, per query.
fn places(scores: &[Vec<f64>]) -> Places {
    scores
        .iter()
        .map(|row| {
            let mut at = vec![0u32; row.len()];
            for (r, i) in rank::order(row).into_iter().enumerate() {
                at[i] = r as u32 + 1;
            }
            at
        })
        .collect()
}

/// Where the first right tool lands in the fused ranking: 1 + the tools fused above it (ties to the lower index, as
/// `rank::fuse`).
fn score(set: &Set, words: &[Vec<u32>], p: Params) -> Score {
    let (meaning, servers) = (&set.meaning[p.embed], &set.servers[p.embed]);
    let mut s = Score {
        n: set.queries.len(),
        ..Default::default()
    };
    for (qi, q) in set.queries.iter().enumerate() {
        let (w, m, sv) = (&words[qi], &meaning[qi], &servers[qi]);
        let f: Vec<f64> = (0..set.tools.len())
            .map(|t| fuse_one(p, w[t], m[t], sv[t]))
            .collect();
        let best = (0..set.tools.len())
            .filter(|t| q.expect.contains(&set.tools[*t].key) && f[*t] > 0.0)
            .min_by(|a, b| f[*b].total_cmp(&f[*a]).then(a.cmp(b)));
        let Some(best) = best else { continue };
        let above = (0..set.tools.len())
            .filter(|t| f[*t] > f[best] || (f[*t] == f[best] && *t < best))
            .count();
        let r = above + 1;
        s.first += (r == 1) as usize;
        s.in3 += (r <= 3) as usize;
        s.in5 += (r <= 5) as usize;
        s.mrr += 1.0 / r as f64;
    }
    s.mrr /= s.n.max(1) as f64;
    s
}

/// The first `n` tools of the fused ranking of one query (`w`, `m`: each tool's place by words and by meaning).
fn fused_top_q(set: &Set, words: &[u32], p: Params, qi: usize, n: usize) -> Vec<usize> {
    let (m, sv) = (&set.meaning[p.embed][qi], &set.servers[p.embed][qi]);
    let mut f: Vec<(usize, f64)> = (0..set.tools.len())
        .map(|t| (t, fuse_one(p, words[t], m[t], sv[t])))
        .filter(|x| x.1 > 0.0)
        .collect();
    f.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    f.into_iter().take(n).map(|x| x.0).collect()
}

fn show(s: &Score) -> String {
    format!(
        "{:>3}/{} {:>3} {:>3} {:.3}",
        s.first, s.n, s.in3, s.in5, s.mrr
    )
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let models = args
        .iter()
        .position(|a| a == "--models")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("--models <root> is required"))?;
    let embedder = LocalEmbedder::load(BGE_SMALL_EN, &models).map_err(|e| anyhow!(e))?;
    let mut sets = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--set" {
            let (tools, queries) = (&args[i + 1], &args[i + 2]);
            let label = PathBuf::from(queries)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let (tools, queries) = load_set(&label, tools, queries)?;
            let qv: Vec<Vec<f32>> = queries
                .iter()
                .map(|q| embedder.embed_query(&q.text))
                .collect::<Result<_, _>>()
                .map_err(|e| anyhow!(e))?;
            let variants: [Vec<String>; EMBEDS.len()] = [
                tools.iter().map(|t| text(t, 2)).collect(),
                tools
                    .iter()
                    .map(|t| format!("{}: {}", t.name, t.desc))
                    .collect(),
                tools
                    .iter()
                    .map(|t| rank::meaning_text(&t.name, &t.desc))
                    .collect(),
            ];
            let mut cos = Vec::new();
            let mut servers = Vec::new();
            for texts in variants {
                let tv = embedder.embed(&texts).map_err(|e| anyhow!(e))?;
                let raw: Vec<Vec<f64>> = qv
                    .iter()
                    .map(|q| {
                        tv.iter()
                            .map(|t| {
                                t.iter()
                                    .zip(q)
                                    .map(|(a, b)| (*a as f64) * (*b as f64))
                                    .sum()
                            })
                            .collect()
                    })
                    .collect();
                servers.push(raw.iter().map(|row| server_places(&tools, row)).collect());
                cos.push(places(&raw));
            }
            eprintln!("{label}: {} tools, {} requests", tools.len(), queries.len());
            sets.push(Set {
                label,
                tools,
                queries,
                meaning: cos,
                servers,
            });
            i += 3;
        } else {
            i += 1;
        }
    }
    if sets.is_empty() {
        return Err(anyhow!(
            "give at least one --set <tools.json> <queries.json>"
        ));
    }

    // BM25 scores for every (k1, b, name_times), per set.
    let k1s = [0.4, 0.6, 0.9, 1.2];
    let bs = [0.3, 0.5, 0.75, 0.9];
    let names = [1, 2, 3, 4];
    let mut words: HashMap<(u64, u64, usize), Vec<Places>> = HashMap::new();
    for &k1 in &k1s {
        for &b in &bs {
            for &nt in &names {
                let per: Vec<Places> = sets
                    .iter()
                    .map(|s| {
                        let texts: Vec<String> = s.tools.iter().map(|t| text(t, nt)).collect();
                        places(&bm25(&texts, &s.queries, k1, b))
                    })
                    .collect();
                words.insert((k1.to_bits(), b.to_bits(), nt), per);
            }
        }
    }
    let eval = |p: Params| -> Vec<Score> {
        let w = &words[&(p.k1.to_bits(), p.b.to_bits(), p.name_times)];
        sets.iter().zip(w).map(|(s, w)| score(s, w, p)).collect()
    };

    println!("set columns: first/n, in 3, in 5, MRR\n");
    let header = sets
        .iter()
        .map(|s| format!("{:<26}", s.label))
        .collect::<String>();
    println!("{:<70} {header}", "config");
    let line = |label: &str, p: Params| {
        let sc = eval(p);
        println!(
            "{label:<70} {}",
            sc.iter()
                .map(|s| format!("{:<26}", show(s)))
                .collect::<String>()
        );
    };
    line("shipped", SHIPPED);
    line(
        "words only (shipped BM25)",
        Params {
            meaning_w: 0.0,
            ..SHIPPED
        },
    );
    line(
        "meaning only (shipped text)",
        Params {
            meaning_w: 1e6,
            meaning_depth: usize::MAX,
            ..SHIPPED
        },
    );
    line("before 2026-10-31", BEFORE);
    line(
        "server place w=1 (tried, not shipped)",
        Params {
            server_w: 1.0,
            ..SHIPPED
        },
    );

    // The grid. A config is ranked by its worst set's MRR (it must hold everywhere), then by the mean MRR.
    let mut all: Vec<(Params, Vec<Score>)> = Vec::new();
    for &k1 in &k1s {
        for &b in &bs {
            for &nt in &names {
                for embed in 0..EMBEDS.len() {
                    for rrf_k in RRF_KS {
                        for meaning_w in [0.5, 0.75, 1.0, 1.25, 1.5, 2.0, 3.0] {
                            for meaning_depth in [30, usize::MAX] {
                                for server_w in [0.0, 0.5, 1.0, 2.0] {
                                    let p = Params {
                                        k1,
                                        b,
                                        name_times: nt,
                                        embed,
                                        rrf_k,
                                        meaning_w,
                                        meaning_depth,
                                        server_w,
                                    };
                                    all.push((p, eval(p)));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    let worst = |s: &[Score]| s.iter().map(|x| x.mrr).fold(f64::MAX, f64::min);
    let mean = |s: &[Score]| s.iter().map(|x| x.mrr).sum::<f64>() / s.len() as f64;
    all.sort_by(|a, b| {
        worst(&b.1)
            .total_cmp(&worst(&a.1))
            .then(mean(&b.1).total_cmp(&mean(&a.1)))
    });
    println!("\n{} configs; best by the worst set's MRR:", all.len());
    for (p, _) in all.iter().take(20) {
        let label = format!(
            "k1={} b={} name×{} embed={} k={} w={} depth={} server={}",
            p.k1,
            p.b,
            p.name_times,
            EMBEDS[p.embed],
            p.rrf_k,
            p.meaning_w,
            if p.meaning_depth == usize::MAX {
                "all".into()
            } else {
                p.meaning_depth.to_string()
            },
            p.server_w
        );
        line(&label, *p);
    }
    // The requests whose right tool is not in the 5 a search returns, shipped and best.
    if args.iter().any(|a| a == "--misses") {
        let best = all[0].0;
        for (label, p) in [("shipped", SHIPPED), ("best", best)] {
            println!(
                "
not in the first 5 ({label}):"
            );
            let w = &words[&(p.k1.to_bits(), p.b.to_bits(), p.name_times)];
            for (si, set) in sets.iter().enumerate() {
                for (qi, q) in set.queries.iter().enumerate() {
                    let top = fused_top_q(set, &w[si][qi], p, qi, 5);
                    if !top.iter().any(|t| q.expect.contains(&set.tools[*t].key)) {
                        let names: Vec<&str> = top
                            .iter()
                            .take(3)
                            .map(|t| set.tools[*t].key.as_str())
                            .collect();
                        println!(
                            "  [{}] {:?} -> want {:?}, got {:?}",
                            set.label, q.text, q.expect, names
                        );
                    }
                }
            }
        }
    }
    // One parameter at a time from the shipped config: what each moves alone.
    println!("\none at a time from shipped (mean MRR over sets):");
    let one = |label: &str, ps: Vec<Params>| {
        let v: Vec<String> = ps
            .iter()
            .map(|p| format!("{:.3}", mean(&eval(*p))))
            .collect();
        println!("  {label:<14} {}", v.join("  "));
    };
    one(
        "k1 0.4..2.0",
        k1s.iter().map(|&k1| Params { k1, ..SHIPPED }).collect(),
    );
    one(
        "b 0.2..0.9",
        bs.iter().map(|&b| Params { b, ..SHIPPED }).collect(),
    );
    one(
        "name ×1..4",
        names
            .iter()
            .map(|&n| Params {
                name_times: n,
                ..SHIPPED
            })
            .collect(),
    );
    one(
        "embed 0..2",
        (0..EMBEDS.len())
            .map(|embed| Params { embed, ..SHIPPED })
            .collect(),
    );
    one(
        "k 5..100",
        RRF_KS
            .iter()
            .map(|&rrf_k| Params { rrf_k, ..SHIPPED })
            .collect(),
    );
    one(
        "w .5..3",
        [0.5, 0.75, 1.0, 1.25, 1.5, 2.0, 3.0]
            .iter()
            .map(|&meaning_w| Params {
                meaning_w,
                ..SHIPPED
            })
            .collect(),
    );
    one(
        "depth 30/all",
        vec![
            SHIPPED,
            Params {
                meaning_depth: usize::MAX,
                ..SHIPPED
            },
        ],
    );
    Ok(())
}
