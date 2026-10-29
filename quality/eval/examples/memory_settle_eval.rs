//! The settling pass (ADR-145) on 70 labelled pairs of memories (`agent/memory/tests/data/settle-pairs.json`): 20 that
//! say the same, 20 that contradict, 20 on one subject that both hold, 10 unrelated. Each pair is put in a project of
//! its own, embedded with the local model, and settled without the judge and (with `JEV_KEY`) with it. Reports the
//! similarity of each kind of pair and what the pass did with them: a wrong fold (two facts that do not say the same
//! made one) is the error that must not happen.
//!
//! Usage: `[JEV_KEY=…] cargo run -p agenium-eval --example memory_settle_eval -- --models <root>/models [--same 0.95]
//! [--related 0.85]`

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use agenium_memory::settle::{settle, PairJudge, SettleOptions, RELATED, SAME};
use agenium_store::agent::{NewFact, PairVerdict};
use agenium_store::AgentStore;
use anyhow::{anyhow, Result};

#[derive(serde::Deserialize)]
struct Pair {
    label: String,
    a: String,
    b: String,
}

fn link_models(from: &Path, root: &Path) -> Result<()> {
    let to = root.join("models").join("bge-small-en-v1.5");
    std::fs::create_dir_all(&to)?;
    for e in std::fs::read_dir(from.join("bge-small-en-v1.5"))? {
        let e = e?;
        if e.file_type()?.is_file() {
            let dest = to.join(e.file_name());
            if std::fs::hard_link(e.path(), &dest).is_err() {
                std::fs::copy(e.path(), &dest)?;
            }
        }
    }
    Ok(())
}

/// What became of a pair: `fold`, `conflict`, `distinct`, `left` (related, not judged), `apart` (below related).
fn outcome(store: &AgentStore, a: i64, b: i64, s: f64, related: f64) -> Result<&'static str> {
    Ok(match store.pair_verdict(a, b)? {
        Some(PairVerdict::Same) => "fold",
        Some(PairVerdict::Conflict) => "conflict",
        Some(PairVerdict::Distinct) => "distinct",
        None if s >= related => "left",
        None => "apart",
    })
}

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let models = args
        .iter()
        .position(|a| a == "--models")
        .and_then(|i| args.get(i + 1))
        .ok_or_else(|| anyhow!("--models <dir>"))?;
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let pairs: Vec<Pair> = serde_json::from_str(&std::fs::read_to_string(
        repo.join("agent/memory/tests/data/settle-pairs.json"),
    )?)?;
    let tmp = tempfile::tempdir()?;
    let root = tmp.path().join("root");
    let (app, _) = agenium_orchestrator::App::open(
        &root,
        agenium_orchestrator::AppOptions {
            keyring: Arc::new(agenium_secrets::MemoryKeyring::default()),
            checks: Arc::new(agenium_orchestrator::NoCheckFactory),
        },
    )?;
    link_models(Path::new(models), &root)?;
    let e = app.embedder();
    let num = |n: &str, d: f64| -> f64 {
        args.iter()
            .position(|a| a == n)
            .and_then(|i| args.get(i + 1))
            .and_then(|v| v.parse().ok())
            .unwrap_or(d)
    };
    let (same, related) = (num("--same", SAME), num("--related", RELATED));
    println!(
        "embedder: {}; same from {same}, related from {related}",
        e.model_id()
    );
    let key = std::env::var("JEV_KEY").ok();
    if let Some(k) = &key {
        app.set_judge(&agenium_orchestrator::judge::JudgeInput {
            base_url: "https://api.typesafe.ai".into(),
            model: "jev-1.14.0".into(),
            memory_settle: true,
            ..Default::default()
        })?;
        app.add_judge_key("eval", k)?;
    }
    let texts: Vec<String> = pairs
        .iter()
        .flat_map(|p| [p.a.clone(), p.b.clone()])
        .collect();
    let vectors = e.embed(&texts).map_err(|e| anyhow!("{e}"))?;
    let cos = |x: &[f32], y: &[f32]| -> f64 {
        let d: f64 = x.iter().zip(y).map(|(a, b)| *a as f64 * *b as f64).sum();
        let n = |v: &[f32]| v.iter().map(|a| (*a as f64).powi(2)).sum::<f64>().sqrt();
        d / (n(x) * n(y))
    };
    let sims: Vec<f64> = (0..pairs.len())
        .map(|i| cos(&vectors[2 * i], &vectors[2 * i + 1]))
        .collect();

    println!("\n| Pair | n | similarity min | median | max |");
    println!("|---|---|---|---|---|");
    for label in ["same", "conflict", "distinct", "unrelated"] {
        let mut s: Vec<f64> = pairs
            .iter()
            .zip(&sims)
            .filter(|(p, _)| p.label == label)
            .map(|(_, s)| *s)
            .collect();
        s.sort_by(f64::total_cmp);
        println!(
            "| {label} | {} | {:.3} | {:.3} | {:.3} |",
            s.len(),
            s[0],
            s[s.len() / 2],
            s[s.len() - 1]
        );
    }

    let arms: Vec<(&str, bool)> = if key.is_some() {
        vec![("vectors only", false), ("with the judge", true)]
    } else {
        vec![("vectors only", false)]
    };
    for (arm, judged) in arms {
        let dir = tmp.path().join(arm.replace(' ', "-"));
        std::fs::create_dir_all(&dir)?;
        let store = AgentStore::open(&dir.join("a.db"), e.dim(), e.model_id(), &dir.join("b"))?;
        let mut ids = Vec::new();
        for (i, p) in pairs.iter().enumerate() {
            let add = |text: &str, v: &[f32]| -> Result<i64> {
                let mut n = NewFact::fact(text, "eval");
                n.project = Some(format!("p{i}"));
                Ok(store.add(&n, Some(v))?)
            };
            ids.push((add(&p.a, &vectors[2 * i])?, add(&p.b, &vectors[2 * i + 1])?));
        }
        let before = app.judge_activity(1000).len();
        let judge = app.settle_judge().filter(|_| judged);
        let t = std::time::Instant::now();
        let r = settle(
            &store,
            judge.as_ref().map(|j| j as &dyn PairJudge),
            SettleOptions {
                max_questions: 1000,
                same,
                related,
                dry_run: false,
            },
        )
        .await?;
        let ms = t.elapsed().as_millis();
        let mut table: BTreeMap<(&str, &str), usize> = BTreeMap::new();
        let mut wrong = Vec::new();
        for (i, p) in pairs.iter().enumerate() {
            let o = outcome(&store, ids[i].0, ids[i].1, sims[i], related)?;
            *table.entry((p.label.as_str(), o)).or_default() += 1;
            let bad =
                (o == "fold" && p.label != "same") || (o == "conflict" && p.label != "conflict");
            if bad || (o == "distinct" && p.label != "distinct" && p.label != "unrelated") {
                wrong.push(format!(
                    "  {} → {o} ({:.3}): {:?} / {:?}",
                    p.label, sims[i], p.a, p.b
                ));
            }
        }
        let activity = app.judge_activity(1000);
        let reads = &activity[..activity.len() - before];
        let tokens: u64 = reads
            .iter()
            .filter_map(|a| a.usage.as_ref()?["input_tokens"].as_u64())
            .sum();
        println!(
            "\n### {arm}: {} questions, {tokens} judge input tokens, {ms} ms",
            r.asked
        );
        println!("| Pair | fold | conflict | distinct | left | apart |");
        println!("|---|---|---|---|---|---|");
        for label in ["same", "conflict", "distinct", "unrelated"] {
            let c = |o| table.get(&(label, o)).copied().unwrap_or(0);
            println!(
                "| {label} | {} | {} | {} | {} | {} |",
                c("fold"),
                c("conflict"),
                c("distinct"),
                c("left"),
                c("apart")
            );
        }
        if !wrong.is_empty() {
            println!("Wrong:");
            wrong.iter().for_each(|w| println!("{w}"));
        }
    }
    Ok(())
}
