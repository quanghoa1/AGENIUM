//! The settling pass (ADR-145) at the scale and crowding of a real memory: about a thousand true facts of one project
//! about forty services (each service its own port, language, database, owner…, so many facts read alike and differ
//! by a name or a figure and all hold), with the 70 labelled pairs of `settle-pairs.json` among them — every fact in
//! one group, as an agent's memory is. A fold of two facts that do not say the same, or a conflict between two that
//! both hold, is an error; passes run until nothing is left to do, the judge (with `JEV_KEY`) at most 20 questions a
//! pass as in the app.
//!
//! Usage: `[JEV_KEY=…] cargo run -p agenium-eval --example memory_settle_scale -- --models <root>/models [--passes 30]`

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use agenium_memory::settle::{settle, Order, PairJudge, SettleOptions, Verdict};
use agenium_store::agent::{NewFact, PairVerdict};
use agenium_store::AgentStore;
use agenium_store::Fact;
use anyhow::{anyhow, Result};
use async_trait::async_trait;

/// A judge that knows the labels (`--oracle`): answers every pair right, and notes at which question it met a labelled
/// pair, to measure the order the judge reads pairs in without spending its tokens.
struct Oracle {
    truth: std::sync::Mutex<HashMap<(i64, i64), String>>,
    asked: std::sync::Mutex<Vec<String>>,
}

#[async_trait]
impl PairJudge for Oracle {
    async fn judge(&self, a: &Fact, b: &Fact) -> Option<Verdict> {
        let t = self
            .truth
            .lock()
            .unwrap()
            .get(&(a.id.min(b.id), a.id.max(b.id)))
            .cloned()
            .unwrap_or_else(|| "distinct".into());
        self.asked.lock().unwrap().push(t.clone());
        Some(match t.as_str() {
            "same" => Verdict::Same,
            "conflict" => Verdict::Conflict,
            _ => Verdict::Distinct,
        })
    }
}

#[derive(serde::Deserialize)]
struct Pair {
    label: String,
    a: String,
    b: String,
}

const SERVICES: &[&str] = &[
    "billing",
    "search",
    "auth",
    "cart",
    "checkout",
    "inventory",
    "shipping",
    "catalog",
    "pricing",
    "reviews",
    "notifications",
    "payments",
    "invoicing",
    "reporting",
    "analytics",
    "recommendations",
    "media",
    "accounts",
    "orders",
    "returns",
    "loyalty",
    "coupons",
    "gateway",
    "scheduler",
    "exports",
    "imports",
    "audit",
    "webhooks",
    "sessions",
    "profiles",
    "wishlist",
    "tax",
    "fraud",
    "support",
    "chat",
    "email",
    "sms",
    "maps",
    "feeds",
    "admin",
];
const LANGS: &[&str] = &[
    "Rust",
    "Go",
    "Python",
    "TypeScript",
    "Kotlin",
    "Java",
    "Elixir",
    "Ruby",
];
const DBS: &[&str] = &[
    "PostgreSQL",
    "MySQL",
    "Redis",
    "MongoDB",
    "SQLite",
    "DynamoDB",
    "Cassandra",
];
const TEAMS: &[&str] = &[
    "Falcon", "Otter", "Lynx", "Heron", "Badger", "Marten", "Puffin", "Wren",
];
const DAYS: &[&str] = &["Monday", "Tuesday", "Wednesday", "Thursday", "Friday"];
const QUEUES: &[&str] = &["Kafka", "RabbitMQ", "SQS", "NATS"];

/// The true facts of service `i`: one value per attribute, so no two of them say the same or contradict.
fn service_facts(i: usize, s: &str) -> Vec<String> {
    let pick = |xs: &[&str], k: usize| xs[(i * 7 + k * 3) % xs.len()].to_owned();
    vec![
        format!("The {s} service listens on port {}.", 8100 + i),
        format!("The {s} service is written in {}.", pick(LANGS, 1)),
        format!("The {s} service stores its data in {}.", pick(DBS, 2)),
        format!("The {s} service is owned by team {}.", pick(TEAMS, 3)),
        format!("The {s} service is deployed on {}.", pick(DAYS, 4)),
        format!(
            "The {s} service keeps its logs for {} days.",
            14 + (i % 6) * 15
        ),
        format!("The {s} service reads its jobs from {}.", pick(QUEUES, 5)),
        format!("The {s} service's health check is at /{s}/healthz."),
        format!("The {s} service's code lives in services/{s}/."),
        format!("Run the {s} service's tests with make test-{s}."),
        format!("Alerts of the {s} service go to the #{s}-oncall channel."),
        format!(
            "The {s} service times out requests after {} seconds.",
            5 + (i % 5) * 5
        ),
        format!("The {s} service runs {} replicas in production.", 2 + i % 4),
        format!(
            "The {s} service caches responses for {} minutes.",
            1 + i % 9
        ),
        format!("The {s} service's API is documented in docs/api/{s}.md."),
        format!("The {s} service is called by the gateway over gRPC."),
        format!("The {s} service's feature flags are prefixed with {s}_."),
        format!("The {s} service's metrics are named {s}_requests_total and {s}_errors_total."),
        format!("The {s} service's database migrations live in services/{s}/migrations."),
        format!("The {s} service's staging URL is https://{s}.staging.example.com."),
        format!("The {s} service's runbook is docs/runbooks/{s}.md."),
        format!(
            "The {s} service's error budget is {} percent a month.",
            [1, 2, 5][i % 3]
        ),
        format!("The {s} service's Docker image is registry.example.com/{s}."),
        format!(
            "The {s} service's on-call rotation changes every {}.",
            DAYS[i % 5]
        ),
        format!("The {s} service sends its traces to Tempo."),
    ]
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

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let arg = |n: &str| {
        args.iter()
            .position(|a| a == n)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let models = arg("--models").ok_or_else(|| anyhow!("--models <dir>"))?;
    let max_passes: usize = arg("--passes").and_then(|p| p.parse().ok()).unwrap_or(30);
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
    link_models(Path::new(&models), &root)?;
    let e = app.embedder();
    if let Ok(k) = std::env::var("JEV_KEY") {
        app.set_judge(&agenium_orchestrator::judge::JudgeInput {
            base_url: "https://api.typesafe.ai".into(),
            model: "jev-1.14.0".into(),
            memory_settle: true,
            ..Default::default()
        })?;
        app.add_judge_key("eval", &k)?;
    }
    // Every text once: the labelled pairs share some texts.
    let mut texts: Vec<String> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut put = |t: &str, texts: &mut Vec<String>| -> usize {
        *index.entry(t.to_owned()).or_insert_with(|| {
            texts.push(t.to_owned());
            texts.len() - 1
        })
    };
    let fillers: Vec<usize> = SERVICES
        .iter()
        .enumerate()
        .flat_map(|(i, s)| service_facts(i, s))
        .collect::<Vec<_>>()
        .iter()
        .map(|t| put(t, &mut texts))
        .collect();
    // `--labels same,conflict`: only the pairs of those labels (the 70 pairs share texts across labels: a text folded
    // with its paraphrase would leave its contradiction labelled on a fact no longer live).
    let labels: Option<Vec<String>> =
        arg("--labels").map(|l| l.split(',').map(str::to_owned).collect());
    let labelled: Vec<(usize, usize, &str)> = pairs
        .iter()
        .filter(|p| labels.as_ref().is_none_or(|l| l.contains(&p.label)))
        .map(|p| {
            (
                put(&p.a, &mut texts),
                put(&p.b, &mut texts),
                p.label.as_str(),
            )
        })
        .collect();
    let t0 = std::time::Instant::now();
    let vectors = e.embed(&texts).map_err(|e| anyhow!("{e}"))?;
    println!(
        "embedder {}: {} facts ({} service facts, {} labelled texts) embedded in {} ms",
        e.model_id(),
        texts.len(),
        fillers.len(),
        texts.len() - fillers.len(),
        t0.elapsed().as_millis()
    );
    let store = AgentStore::open(
        &tmp.path().join("a.db"),
        e.dim(),
        e.model_id(),
        &tmp.path().join("b"),
    )?;
    let ids: Vec<i64> = texts
        .iter()
        .zip(&vectors)
        .map(|(t, v)| {
            let mut n = NewFact::fact(t, "ticket:AGN-1");
            n.project = Some("shop".into());
            store.add(&n, Some(v))
        })
        .collect::<Result<_, _>>()?;
    let id_of = |i: usize| ids[i];
    let text_of: HashMap<i64, &str> = ids
        .iter()
        .zip(&texts)
        .map(|(i, t)| (*i, t.as_str()))
        .collect();
    // What each pair of facts truly is: the labelled pairs as labelled; any other pair holds both ("distinct").
    let mut truth: HashMap<(i64, i64), String> = HashMap::new();
    for (a, b, l) in &labelled {
        let (x, y) = (id_of(*a).min(id_of(*b)), id_of(*a).max(id_of(*b)));
        truth.insert((x, y), l.to_string());
    }

    let order = match arg("--order").as_deref() {
        Some("similarity") => Order::Similarity,
        _ => Order::SwapsFirst,
    };
    let oracle = args.iter().any(|a| a == "--oracle").then(|| Oracle {
        truth: std::sync::Mutex::new(truth.clone()),
        asked: Default::default(),
    });
    let judge = app.settle_judge();
    let judge: Option<&dyn PairJudge> = match &oracle {
        Some(o) => Some(o),
        None => judge.as_ref().map(|j| j as &dyn PairJudge),
    };
    let questions: usize = arg("--questions")
        .and_then(|q| q.parse().ok())
        .unwrap_or(20);
    println!("order {order:?}, {questions} questions a pass");
    let (mut asked, mut done, mut ms) = (0, Vec::new(), 0u128);
    for pass in 1..=max_passes {
        let t = std::time::Instant::now();
        let r = settle(
            &store,
            judge,
            SettleOptions {
                max_questions: questions,
                order,
                ..Default::default()
            },
        )
        .await?;
        ms += t.elapsed().as_millis();
        if pass == 1 {
            println!(
                "pass 1: {} live facts, {} related pairs ({} parallel facts, {} left for the judge), {} ms",
                r.facts,
                r.related,
                r.parallel,
                r.left + r.asked,
                t.elapsed().as_millis()
            );
        }
        asked += r.asked;
        for d in r.done.iter().filter(|d| d.verdict == PairVerdict::Same) {
            let (keep, gone) = (d.a, d.b);
            let moved: Vec<((i64, i64), String)> = truth
                .iter()
                .filter(|((x, y), _)| *x == gone || *y == gone)
                .map(|((x, y), l)| {
                    let other = if *x == gone { *y } else { *x };
                    ((keep.min(other), keep.max(other)), l.clone())
                })
                .filter(|((x, y), _)| x != y)
                .collect();
            for (k, l) in moved {
                truth.entry(k).or_insert(l);
            }
        }
        if let Some(o) = &oracle {
            *o.truth.lock().unwrap() = truth.clone();
        }
        let n = r.done.len();
        done.extend(r.done);
        if n == 0 && r.asked == 0 {
            println!("settled after {} passes", pass - 1);
            break;
        }
        if pass == max_passes {
            println!("still {} related pairs left after {pass} passes", r.left);
        }
    }
    let (mut wrong, mut right) = (Vec::new(), [0usize; 4]);
    for d in &done {
        let key = (d.a.min(d.b), d.a.max(d.b));
        let truly = truth.get(&key).map(String::as_str).unwrap_or("distinct");
        let ok = match d.verdict {
            PairVerdict::Same => truly == "same",
            PairVerdict::Conflict => truly == "conflict",
            PairVerdict::Distinct => truly != "same" && truly != "conflict",
        };
        if ok {
            right[match d.verdict {
                PairVerdict::Same => 0,
                PairVerdict::Conflict => 1,
                PairVerdict::Distinct => 2,
            }] += 1;
        } else {
            right[3] += 1;
            wrong.push(format!(
                "  {:?} by {} ({:.3}) but {truly}: {:?} / {:?}",
                d.verdict, d.by, d.similarity, text_of[&d.a], text_of[&d.b]
            ));
        }
    }
    let labelled_same = labelled.iter().filter(|l| l.2 == "same").count();
    let labelled_conflict = labelled.iter().filter(|l| l.2 == "conflict").count();
    println!(
        "\n{} judge questions, {ms} ms of settling\nright: {} folds (of {labelled_same} same pairs), {} conflicts (of \
         {labelled_conflict}), {} kept as both holding\nwrong: {}",
        asked, right[0], right[1], right[2], right[3]
    );
    wrong.iter().take(40).for_each(|w| println!("{w}"));
    // Labelled pairs that say the same or contradict and that the passes never decided, with their similarity.
    let decided: std::collections::HashSet<(i64, i64)> =
        done.iter().map(|d| (d.a.min(d.b), d.a.max(d.b))).collect();
    let cos = |x: &[f32], y: &[f32]| -> f64 {
        let d: f64 = x.iter().zip(y).map(|(a, b)| *a as f64 * *b as f64).sum();
        let n = |v: &[f32]| v.iter().map(|a| (*a as f64).powi(2)).sum::<f64>().sqrt();
        d / (n(x) * n(y))
    };
    for (a, b, l) in &labelled {
        let key = (id_of(*a).min(id_of(*b)), id_of(*a).max(id_of(*b)));
        if (*l == "same" || *l == "conflict") && !decided.contains(&key) {
            println!(
                "  missed {l} ({:.3}): {:?} / {:?}",
                cos(&vectors[*a], &vectors[*b]),
                texts[*a],
                texts[*b]
            );
        }
    }
    if let Some(o) = &oracle {
        let seen = o.asked.lock().unwrap();
        let positives = labelled_same + labelled_conflict;
        let mut found = 0;
        let mut at = Vec::new();
        for (q, t) in seen.iter().enumerate() {
            if t == "same" || t == "conflict" {
                found += 1;
                at.push(q + 1);
            }
        }
        let within = |n: usize| at.iter().filter(|q| **q <= n).count();
        println!(
            "oracle: {} questions; labelled same/conflict pairs met: {found} of {positives} (the rest decided without the \
             judge or below related); within 20 questions {}, 100 {}, 400 {}, 2000 {}",
            seen.len(),
            within(20),
            within(100),
            within(400),
            within(2000)
        );
    }
    Ok(())
}
