use agenium_store::{AgentStore, FactKind, NewFact, StoreError, Via};

/// Deterministic toy embedding: bag of hashed words, L2-normalised (enough to test the index plumbing).
fn embed(text: &str, dim: usize) -> Vec<f32> {
    let mut v = vec![0f32; dim];
    for w in text
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
    {
        let mut h: u64 = 1469598103934665603;
        for b in w.bytes() {
            h = (h ^ b as u64).wrapping_mul(1099511628211);
        }
        v[(h % dim as u64) as usize] += 1.0;
    }
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-6);
    v.iter().map(|x| x / n).collect()
}

const DIM: usize = 64;

fn open(dir: &std::path::Path) -> AgentStore {
    AgentStore::open(
        &dir.join("agents/a.db"),
        DIM,
        "toy-hash",
        &dir.join("backups"),
    )
    .unwrap()
}

fn add(s: &AgentStore, text: &str) -> i64 {
    s.add(
        &NewFact::fact(text, "ticket:AGN-1"),
        Some(&embed(text, DIM)),
    )
    .unwrap()
}

#[test]
fn knn_keyword_and_hybrid() {
    let tmp = tempfile::tempdir().unwrap();
    let s = open(tmp.path());
    let ids: Vec<i64> = [
        "The project uses tokio for async runtime",
        "Tests are run with cargo nextest",
        "The user prefers short commit messages",
        "Database migrations live in crates/store/schema",
        "Windows builds use the GNU toolchain with MSYS2 gcc",
    ]
    .iter()
    .map(|t| add(&s, t))
    .collect();
    assert_eq!(s.count_live().unwrap(), 5);

    let q = "which toolchain do windows builds use";
    let knn = s.knn(&embed(q, DIM), 3).unwrap();
    assert_eq!(knn[0].0, ids[4]);
    assert!(knn.windows(2).all(|w| w[0].1 <= w[1].1));
    let kw = s.keyword("migrations schema", 3).unwrap();
    assert_eq!(kw[0].0, ids[3]);
    assert!(s.keyword("  ", 3).unwrap().is_empty());
    assert!(
        s.keyword("\"; DROP TABLE fact; --", 3).is_ok(),
        "FTS syntax is neutralised"
    );

    let hits = s.hybrid(q, Some(&embed(q, DIM)), 2).unwrap();
    assert_eq!(hits[0].fact.id, ids[4]);
    assert_eq!(hits[0].via, Via::Hybrid);
    assert_eq!(hits.len(), 2);

    s.record_usage(
        "run-1",
        Some("AGN-7"),
        &[(ids[4], 1, Via::Hybrid), (ids[0], 2, Via::Vector)],
    )
    .unwrap();
    assert_eq!(s.usage_for_run("run-1").unwrap().len(), 2);
    assert!(s.get(ids[4]).unwrap().used_at.is_some());
}

#[test]
fn supersede_and_retire_hide_old_facts_but_keep_history() {
    let tmp = tempfile::tempdir().unwrap();
    let s = open(tmp.path());
    let old = add(&s, "The API base URL is http://localhost:3000");
    let newer = NewFact {
        kind: FactKind::Fact,
        ..NewFact::fact("The API base URL is http://localhost:8080", "ticket:AGN-2")
    };
    let new_id = s
        .supersede(old, &newer, Some(&embed(&newer.text, DIM)))
        .unwrap();
    assert!(matches!(
        s.supersede(old, &newer, None),
        Err(StoreError::Conflict(_))
    ));
    let q = "API base URL";
    let hits = s.hybrid(q, Some(&embed(q, DIM)), 5).unwrap();
    assert_eq!(
        hits.iter().map(|h| h.fact.id).collect::<Vec<_>>(),
        vec![new_id]
    );
    assert_eq!(
        s.get(old).unwrap().superseded_by,
        Some(new_id),
        "history is kept"
    );

    s.retire(new_id).unwrap();
    assert!(s.hybrid(q, Some(&embed(q, DIM)), 5).unwrap().is_empty());
    assert!(s.knn(&embed(q, DIM), 5).unwrap().is_empty());
    assert!(matches!(s.retire(new_id), Err(StoreError::NotFound(_))));
}

#[test]
fn dimension_is_fixed_and_conversation_round_trips() {
    let tmp = tempfile::tempdir().unwrap();
    {
        let s = open(tmp.path());
        assert!(s.add(&NewFact::fact("x", "chat"), Some(&[0.0; 3])).is_err());
        s.append_message(
            "ses",
            &serde_json::json!({"role":"user","parts":[{"Text":"hi"}]}),
        )
        .unwrap();
        s.append_message("ses", &serde_json::json!({"role":"assistant","parts":[]}))
            .unwrap();
    }
    let e = AgentStore::open(
        &tmp.path().join("agents/a.db"),
        32,
        "other",
        &tmp.path().join("backups"),
    )
    .unwrap_err();
    assert!(matches!(e, StoreError::Invalid(_)), "{e}");
    let s = open(tmp.path());
    let msgs = s.load_messages("ses").unwrap();
    assert_eq!(msgs.len(), 2);
    assert_eq!(msgs[0]["role"], "user");
    assert_eq!(s.meta("embedding_dim").unwrap().as_deref(), Some("64"));
}

#[test]
fn reindex_replaces_vectors() {
    let tmp = tempfile::tempdir().unwrap();
    let s = open(tmp.path());
    let a = add(&s, "alpha beta");
    add(&s, "gamma delta");
    // A "new model": reverse the toy vector. The nearest neighbour of the reversed query is still the same fact.
    let n = s
        .reindex(|texts| {
            Ok(texts
                .iter()
                .map(|t| embed(t, DIM).into_iter().rev().collect())
                .collect())
        })
        .unwrap();
    assert_eq!(n, 2);
    let q: Vec<f32> = embed("alpha beta", DIM).into_iter().rev().collect();
    assert_eq!(s.knn(&q, 1).unwrap()[0].0, a);
}

/// Decision 35: lessons count the contexts they held in; unused facts fade by half every 90 days and hide below
/// 0.2; duplicates fold into the most trusted copy (history kept).
#[test]
fn validation_decay_and_consolidation() {
    let tmp = tempfile::tempdir().unwrap();
    let s = open(tmp.path());
    let a = add(&s, "Run cargo test --workspace before submitting");
    assert_eq!(s.validate_in(a, "project:p1").unwrap(), 1);
    assert_eq!(
        s.validate_in(a, "project:p1").unwrap(),
        1,
        "same context once"
    );
    assert_eq!(s.validate_in(a, "project:p2").unwrap(), 2);
    assert_eq!(s.get(a).unwrap().validated, ["project:p1", "project:p2"]);
    // Audit M3: the contexts a fact misled the work in are counted apart from those it held in, per fact.
    let b = add(&s, "Pin exact crate versions in examples");
    assert_eq!(s.misled_in(a, "p1/ticket:A-1").unwrap(), 1);
    assert_eq!(
        s.misled_in(a, "p1/ticket:A-1").unwrap(),
        1,
        "same context once"
    );
    assert_eq!(s.misled_in(b, "p1/ticket:A-1").unwrap(), 1, "per fact");
    assert_eq!(s.misled_in(a, "p2/ticket:A-1").unwrap(), 2);
    assert_eq!(
        s.get(a).unwrap().validated.len(),
        2,
        "held-in contexts unchanged"
    );

    let now = chrono::Utc::now();
    assert!(s.decay(now, 90.0).unwrap() <= 1);
    assert!(s.get(a).unwrap().weight > 0.99);
    let later = now + chrono::Duration::days(90);
    s.decay(later, 90.0).unwrap();
    assert!((s.get(a).unwrap().weight - 0.5).abs() < 0.01);
    s.decay(now + chrono::Duration::days(240), 90.0).unwrap();
    assert!(s.get(a).unwrap().weight < agenium_store::agent::HIDE_BELOW_WEIGHT);

    let b = add(&s, "run  cargo TEST --workspace before submitting");
    let c = add(&s, "Something else");
    assert_eq!(s.consolidate().unwrap(), 1);
    let (fa, fb) = (s.get(a).unwrap(), s.get(b).unwrap());
    assert!(fa.is_live() != fb.is_live(), "one of the two stays");
    assert!(s.get(c).unwrap().is_live());
    assert_eq!(s.consolidate().unwrap(), 0);
}

/// Consolidation keeps trust and project apart: an untrusted copy of a trusted fact (even a more confident one) does
/// not replace it, copies learned in two projects stay two, and the contexts a folded copy was validated in move to
/// the one that stays.
#[test]
fn consolidation_respects_trust_and_projects_and_keeps_validations() {
    let tmp = tempfile::tempdir().unwrap();
    let s = open(tmp.path());
    let fact = |text: &str, untrusted: bool, project: Option<&str>, confidence: f64| {
        let mut f = NewFact::fact(text, "ticket:AGN-1");
        f.untrusted = untrusted;
        f.project = project.map(str::to_owned);
        f.confidence = confidence;
        s.add(&f, Some(&embed(text, DIM))).unwrap()
    };
    let trusted = fact("Deploy with the blue-green script", false, None, 0.5);
    let untrusted = fact("deploy with the  blue-green script", true, None, 0.9);
    let p1 = fact("Use port 8080", false, Some("p1"), 0.5);
    let p2 = fact("use port 8080", false, Some("p2"), 0.5);
    assert_eq!(s.consolidate().unwrap(), 0);
    for id in [trusted, untrusted, p1, p2] {
        assert!(s.get(id).unwrap().is_live(), "{id}");
    }

    let older = fact("Run migrations first", false, Some("p1"), 0.5);
    let newer = fact("run migrations  first", false, Some("p1"), 0.5);
    s.validate_in(older, "ticket:AGN-2").unwrap();
    s.validate_in(newer, "ticket:AGN-3").unwrap();
    assert_eq!(s.consolidate().unwrap(), 1);
    let kept = s.get(newer).unwrap();
    assert!(kept.is_live() && !s.get(older).unwrap().is_live());
    assert_eq!(kept.validated, ["ticket:AGN-3", "ticket:AGN-2"]);
}

/// A lesson injected as a rule of the work's context is logged as used: the CHECK on `fact_usage.via` did not allow
/// 'rule' and `INSERT OR IGNORE` dropped the row without a word. A fact logged twice for one run keeps one row.
#[test]
fn rule_injected_lessons_are_recorded_as_used() {
    let tmp = tempfile::tempdir().unwrap();
    let s = open(tmp.path());
    let a = add(&s, "Use the staging database for migrations");
    let b = add(&s, "Run cargo fmt before committing");
    s.record_usage(
        "run-1",
        Some("AGN-1"),
        &[(a, 0, Via::Rule), (b, 1, Via::Vector), (b, 2, Via::Keyword)],
    )
    .unwrap();
    assert_eq!(
        s.usage_for_run("run-1").unwrap(),
        vec![(a, 0, "rule".to_owned()), (b, 1, "vector".to_owned())]
    );
    assert!(s.get(a).unwrap().used_at.is_some());
}

/// Migration 4 rebuilds `fact_usage` (a CHECK cannot be altered): the rows of a v3 database survive with their outcome
/// signals, and a rule can be logged afterwards.
#[test]
fn v3_usage_log_survives_the_rule_migration() {
    let tmp = tempfile::tempdir().unwrap();
    let db = tmp.path().join("agents/a.db");
    std::fs::create_dir_all(db.parent().unwrap()).unwrap();
    {
        let c = rusqlite::Connection::open(&db).unwrap();
        for sql in [
            include_str!("../src/schema/agent_001.sql"),
            include_str!("../src/schema/agent_002.sql"),
            include_str!("../src/schema/agent_003.sql"),
        ] {
            c.execute_batch(sql).unwrap();
        }
        c.execute_batch(&format!(
            "PRAGMA user_version = 3;
             INSERT INTO agent_meta(key, value) VALUES ('embedding_dim', '{DIM}');
             INSERT INTO fact(id, text, kind, source) VALUES (7, 'Run the linter', 'lesson', 'ticket:AGN-1');
             INSERT INTO fact_usage(fact, ticket, run, rank, via, outcome, signal)
               VALUES (7, 'AGN-1', 'run-0', 0, 'hybrid', 'helped', 'closed');"
        ))
        .unwrap();
    }
    let s = open(tmp.path());
    assert_eq!(
        s.usage_for_run("run-0").unwrap(),
        vec![(7, 0, "hybrid".to_owned())]
    );
    let (outcome, signal): (String, String) = s
        .db()
        .read(|c| -> Result<_, StoreError> {
            Ok(c.query_row(
                "SELECT outcome, signal FROM fact_usage WHERE run = 'run-0'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?)
        })
        .unwrap();
    assert_eq!((outcome.as_str(), signal.as_str()), ("helped", "closed"));
    s.record_usage("run-1", None, &[(7, 0, Via::Rule)]).unwrap();
    assert_eq!(
        s.usage_for_run("run-1").unwrap(),
        vec![(7, 0, "rule".to_owned())]
    );
}

/// A zero vector has no direction (cosine distance undefined; sqlite-vec returns NULL). The hashing embedder produces one
/// for text without a word of two or more letters ("C#", "C++"): it must neither be stored nor break searches, whether
/// it is the query or a stored vector.
#[test]
fn zero_vectors_are_not_indexed_and_do_not_break_search() {
    use agenium_store::graph::NewEntity;
    let tmp = tempfile::tempdir().unwrap();
    let s = open(tmp.path());
    let zero = vec![0f32; DIM];
    let a = add(&s, "Windows builds use the GNU toolchain");
    // Stored with a zero vector (as if every word were one letter): kept, found by keyword only.
    let z = s
        .add(
            &NewFact::fact("C# and C++ interop", "ticket:AGN-1"),
            Some(&zero),
        )
        .unwrap();
    assert_eq!(
        s.stale_vectors("toy-hash").unwrap(),
        0,
        "a fact without a usable vector is not stale"
    );
    let knn = s.knn(&embed("windows toolchain", DIM), 5).unwrap();
    assert_eq!(knn.iter().map(|h| h.0).collect::<Vec<_>>(), vec![a]);
    assert!(s.knn(&zero, 5).unwrap().is_empty());
    assert_eq!(
        s.keyword("interop", 5).unwrap().first().map(|h| h.0),
        Some(z)
    );
    assert_eq!(
        s.hybrid("interop", Some(&zero), 5)
            .unwrap()
            .first()
            .map(|h| h.fact.id),
        Some(z)
    );

    let ent = |name: &str| NewEntity {
        kind: "language".into(),
        name: name.into(),
        scope: serde_json::json!({}),
        description: None,
        aliases: vec![],
        confidence: 0.6,
        evidence: None,
    };
    let (rust, _) = s
        .upsert_entity(&ent("Rust"), Some(&embed("Rust. systems language", DIM)))
        .unwrap();
    let (csharp, _) = s.upsert_entity(&ent("C#"), Some(&zero)).unwrap();
    let c = s
        .entity_candidates("language", "Rust", Some(&embed("Rust", DIM)), 5)
        .unwrap();
    assert_eq!(c.first().map(|c| c.entity.id), Some(rust));
    let c = s
        .entity_candidates("language", "C#", Some(&zero), 5)
        .unwrap();
    assert_eq!(
        c.first().map(|c| (c.entity.id, c.via)),
        Some((csharp, "exact"))
    );
    s.reindex(|t| Ok(t.iter().map(|_| vec![0f32; DIM]).collect()))
        .unwrap();
    s.reindex_entities(|t| Ok(t.iter().map(|_| vec![0f32; DIM]).collect()))
        .unwrap();
    assert!(s
        .knn(&embed("windows toolchain", DIM), 5)
        .unwrap()
        .is_empty());
    assert_eq!(s.stale_vectors("toy-hash").unwrap(), 0);
}

/// ADR-94: "C#", "C++" and "C" were all keyed "c", so a mention of C++ resolved to the C# entity. Keys keep `+` and
/// `#` now, and a database written before is re-keyed when it is opened.
#[test]
fn symbol_names_get_their_own_keys_and_old_keys_are_rewritten() {
    use agenium_store::graph::{canonical, NewEntity};
    assert_eq!(canonical("C#"), "c#");
    assert_eq!(canonical("C++ / C"), "c++ c");
    assert_eq!(canonical("Synology DS920+"), "synology ds920+");
    assert_eq!(canonical("#1 priority"), "1 priority");
    let tmp = tempfile::tempdir().unwrap();
    let ent = |name: &str| NewEntity {
        kind: "language".into(),
        name: name.into(),
        scope: serde_json::json!({}),
        description: None,
        aliases: vec![],
        confidence: 0.6,
        evidence: None,
    };
    let csharp = {
        let s = open(tmp.path());
        let (id, _) = s.upsert_entity(&ent("C#"), None).unwrap();
        // As an older build stored it.
        s.db()
            .write(|tx| -> Result<(), StoreError> {
                tx.execute("UPDATE entity SET canonical_key = 'c' WHERE id = ?1", [id])?;
                tx.execute(
                    "UPDATE entity_alias SET alias_norm = 'c' WHERE entity_id = ?1",
                    [id],
                )?;
                tx.execute("DELETE FROM agent_meta WHERE key = 'canonical'", [])?;
                Ok(())
            })
            .unwrap();
        id
    };
    let s = open(tmp.path());
    assert_eq!(s.entity(csharp).unwrap().canonical_key, "c#");
    let c = s.entity_candidates("language", "C#", None, 5).unwrap();
    assert_eq!(
        c.first().map(|c| (c.entity.id, c.via)),
        Some((csharp, "exact"))
    );
    assert!(s
        .entity_candidates("language", "C++", None, 5)
        .unwrap()
        .iter()
        .all(|c| c.entity.id != csharp || c.via == "fts"));
    let (cpp, created) = s.upsert_entity(&ent("C++"), None).unwrap();
    assert!(created && cpp != csharp);
    assert_eq!(
        s.entities_in_text("we port C# code to C++", 5)
            .unwrap()
            .len(),
        2
    );
}
