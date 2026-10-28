//! Crash consistency (decision 22, spike S1/S4 re-run on this platform): a child process commits intents in a
//! loop and is killed hard (TerminateProcess on Windows, SIGKILL on Unix). After each kill the store must open,
//! pass `integrity_check`, keep a gap-free event log, contain every acknowledged intent, and never contain a
//! half-applied intent.

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};

use agenium_core::{DomainEvent, ProjectId};
use agenium_store::core::{events, projects};
use agenium_store::{CoreStore, StoreError};

const CHILD_ENV: &str = "AGENIUM_CRASH_CHILD_DIR";

#[test]
#[ignore = "helper: run by kill_during_writes_loses_nothing_acknowledged"]
fn crash_child() {
    let Ok(dir) = std::env::var(CHILD_ENV) else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    let s = CoreStore::open(&dir.join("core.db"), &dir.join("backups")).unwrap();
    let project = match s.read(projects::list).unwrap().first() {
        Some(p) => p.id,
        None => {
            let np = projects::NewProject {
                id: ProjectId::new(),
                name: "p".into(),
                path: "X:/p".into(),
                key_prefix: "AGN".into(),
                main_branch: "main".into(),
            };
            s.commit(None, "user", |w| projects::insert(w.tx, &np))
                .unwrap()
                .value
                .id
        }
    };
    let start = s.read(events::last_seq).unwrap();
    let mut i = 0u64;
    loop {
        let intent = format!("crash-{start}-{i}");
        // Two events + a state change per intent: all or nothing.
        s.commit(Some(&intent), "system", |w| -> Result<(), StoreError> {
            w.emit(DomainEvent::ProjectStatusChanged {
                project,
                status: "Paused".into(),
            })?;
            projects::set_status(
                w.tx,
                project,
                if i.is_multiple_of(2) {
                    projects::ProjectStatus::Paused
                } else {
                    projects::ProjectStatus::Active
                },
            )?;
            w.emit(DomainEvent::ProjectStatusChanged {
                project,
                status: "Active".into(),
            })?;
            Ok(())
        })
        .unwrap();
        println!("ACK {intent}");
        i += 1;
    }
}

#[test]
fn kill_during_writes_loses_nothing_acknowledged() {
    let tmp = tempfile::tempdir().unwrap();
    let exe = std::env::current_exe().unwrap();
    let rounds: usize = std::env::var("AGENIUM_CRASH_ROUNDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(5);
    let mut acked_total = Vec::new();
    for round in 0..rounds {
        let mut child = Command::new(&exe)
            .args([
                "crash_child",
                "--exact",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CHILD_ENV, tmp.path())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let out = BufReader::new(child.stdout.take().unwrap());
        let want = 30 + round * 17;
        let mut acked = Vec::new();
        for line in out.lines() {
            let line = line.unwrap();
            if let Some(id) = line.strip_prefix("ACK ") {
                acked.push(id.to_owned());
                if acked.len() >= want {
                    break;
                }
            }
        }
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(acked.len() >= want, "child stopped early in round {round}");
        acked_total.extend(acked);

        let s = CoreStore::open(&tmp.path().join("core.db"), &tmp.path().join("backups")).unwrap();
        let ok: String = s
            .read(|c| Ok(c.query_row("PRAGMA integrity_check", [], |r| r.get(0))?))
            .unwrap();
        assert_eq!(ok, "ok");
        let all = s.read(|c| events::since(c, 0, usize::MAX >> 1)).unwrap();
        for (i, e) in all.iter().enumerate() {
            assert_eq!(e.seq, i as u64 + 1, "gap in the event log");
        }
        // Every intent row has exactly its two events; no event without its intent row.
        let (intents, orphan): (i64, i64) = s
            .read(|c| {
                Ok((
                    c.query_row("SELECT count(*) FROM intent", [], |r| r.get(0))?,
                    c.query_row(
                        "SELECT count(*) FROM event e WHERE e.intent_id IS NOT NULL AND NOT EXISTS (SELECT 1 FROM intent i WHERE i.id = e.intent_id)",
                        [],
                        |r| r.get(0),
                    )?,
                ))
            })
            .unwrap();
        assert_eq!(orphan, 0, "events of an uncommitted intent");
        let with_intent = all.iter().filter(|e| e.intent_id.is_some()).count() as i64;
        assert_eq!(with_intent, intents * 2, "half-applied intent");
        for id in &acked_total {
            assert!(
                s.intent_result(id).unwrap().is_some(),
                "acknowledged intent {id} lost"
            );
        }
    }
}

const GRAPH_CHILD_ENV: &str = "AGENIUM_GRAPH_CRASH_CHILD_DIR";

#[test]
#[ignore = "helper: run by kill_during_graph_writes_keeps_the_graph_consistent"]
fn graph_crash_child() {
    let Ok(dir) = std::env::var(GRAPH_CHILD_ENV) else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    let s = agenium_store::AgentStore::open(&dir.join("agent.db"), 4, "m", &dir.join("backups"))
        .unwrap();
    let mut i = s.live(1).unwrap().first().map(|f| f.id as u64).unwrap_or(0) * 1000;
    let ent = |n: String| agenium_store::graph::NewEntity {
        kind: "concept".into(),
        name: n,
        scope: serde_json::json!({}),
        description: None,
        aliases: vec![],
        confidence: 0.5,
        evidence: None,
    };
    loop {
        // Extraction-like burst: fact, entities, links, edges; every 7th round a merge.
        let f = s
            .add(
                &agenium_store::NewFact::fact(format!("fact {i}"), "t"),
                Some(&[1.0, 0.0, 0.0, 0.0]),
            )
            .unwrap();
        let (a, _) = s
            .upsert_entity(&ent(format!("e{}", i % 50)), Some(&[0.0, 1.0, 0.0, 0.0]))
            .unwrap();
        let (b, _) = s
            .upsert_entity(&ent(format!("e{}", (i * 7 + 3) % 50)), None)
            .unwrap();
        s.link_fact(f, a, "subject").unwrap();
        if a != b {
            s.add_edge(
                a,
                b,
                "related_to",
                &serde_json::json!({}),
                0.5,
                Some(f),
                "extract",
            )
            .unwrap();
        }
        if i % 7 == 6 {
            let (c, _) = s.upsert_entity(&ent(format!("dup {i}")), None).unwrap();
            s.add_edge(c, a, "uses", &serde_json::json!({}), 0.5, None, "extract")
                .unwrap();
            s.merge_entities(c, a, Some(0.95), "auto").unwrap_or(());
        }
        println!("ACK {i}");
        i += 1;
    }
}

#[test]
fn kill_during_graph_writes_keeps_the_graph_consistent() {
    let tmp = tempfile::tempdir().unwrap();
    let exe = std::env::current_exe().unwrap();
    let rounds: usize = std::env::var("AGENIUM_CRASH_ROUNDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(5);
    for round in 0..rounds {
        let mut child = Command::new(&exe)
            .args([
                "graph_crash_child",
                "--exact",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(GRAPH_CHILD_ENV, tmp.path())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let out = BufReader::new(child.stdout.take().unwrap());
        let want = 20 + round * 11;
        let mut n = 0;
        for line in out.lines() {
            if line.unwrap().starts_with("ACK ") {
                n += 1;
                if n >= want {
                    break;
                }
            }
        }
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(n >= want, "child stopped early in round {round}");

        let s = agenium_store::AgentStore::open(
            &tmp.path().join("agent.db"),
            4,
            "m",
            &tmp.path().join("backups"),
        )
        .unwrap();
        let checks: (String, i64, i64, i64, i64) = s
            .db()
            .read(|c| -> Result<_, StoreError> {
                Ok((
                    c.query_row("PRAGMA integrity_check", [], |r| r.get(0))?,
                    c.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| r.get(0))?,
                    // Degree cache equals the live edges of every entity.
                    c.query_row(
                        "SELECT count(*) FROM entity e WHERE e.degree <>
                           (SELECT count(*) FROM edge x WHERE x.valid_to IS NULL AND (x.src = e.id OR x.dst = e.id))",
                        [],
                        |r| r.get(0),
                    )?,
                    // Live edges never touch a merged entity.
                    c.query_row(
                        "SELECT count(*) FROM edge x JOIN entity e ON e.id IN (x.src, x.dst)
                         WHERE x.valid_to IS NULL AND e.merged_into IS NOT NULL",
                        [],
                        |r| r.get(0),
                    )?,
                    // Merge chains end (no entity merged into a merged entity).
                    c.query_row(
                        "SELECT count(*) FROM entity a JOIN entity b ON b.id = a.merged_into WHERE b.merged_into IS NOT NULL",
                        [],
                        |r| r.get(0),
                    )?,
                ))
            })
            .unwrap();
        assert_eq!(checks, ("ok".to_string(), 0, 0, 0, 0), "round {round}");
    }
}

const ANN_CHILD_ENV: &str = "AGENIUM_ANN_CRASH_CHILD_DIR";

/// The vector of the `i`-th fact in the ANN crash test: deterministic, distinct, never zero.
fn ann_vec(i: u64) -> Vec<f32> {
    let mut x = i.wrapping_mul(0x9e3779b97f4a7c15) | 1;
    (0..16)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            ((x >> 40) as f32 / (1u64 << 24) as f32) - 0.5
        })
        .collect()
}

fn ann_options() -> agenium_store::AgentOptions {
    agenium_store::AgentOptions {
        index_mode: agenium_store::IndexMode::Ann,
        ann: agenium_store::AnnOptions {
            save_every: 97,
            build_threads: 2,
            ..Default::default()
        },
        ..Default::default()
    }
}

#[test]
#[ignore = "helper: run by kill_during_ann_writes_never_serves_a_wrong_result"]
fn ann_crash_child() {
    let Ok(dir) = std::env::var(ANN_CHILD_ENV) else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    let s = agenium_store::AgentStore::open_with(
        &dir.join("agent.db"),
        16,
        "m",
        &dir.join("backups"),
        ann_options(),
    )
    .unwrap();
    let mut i = s.count_live().unwrap() * 3;
    loop {
        // Adds, plus a retire every third fact: the sidecar is saved every 97 changes, so kills land before, during
        // and after saves.
        let text = format!("fact {i}");
        let id = s
            .add(&agenium_store::NewFact::fact(text, "t"), Some(&ann_vec(i)))
            .unwrap();
        if i % 3 == 2 {
            s.retire(id).unwrap();
        }
        println!("ACK {i}");
        i += 1;
    }
}

#[test]
fn kill_during_ann_writes_never_serves_a_wrong_result() {
    let tmp = tempfile::tempdir().unwrap();
    let exe = std::env::current_exe().unwrap();
    let rounds: usize = std::env::var("AGENIUM_CRASH_ROUNDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(20);
    let mut misses = 0;
    for round in 0..rounds {
        let mut child = Command::new(&exe)
            .args([
                "ann_crash_child",
                "--exact",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(ANN_CHILD_ENV, tmp.path())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let out = BufReader::new(child.stdout.take().unwrap());
        let want = 60 + round * 13;
        let mut n = 0;
        for line in out.lines() {
            if line.unwrap().starts_with("ACK ") {
                n += 1;
                if n >= want {
                    break;
                }
            }
        }
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(n >= want, "child stopped early in round {round}");

        let s = agenium_store::AgentStore::open_with(
            &tmp.path().join("agent.db"),
            16,
            "m",
            &tmp.path().join("backups"),
            ann_options(),
        )
        .unwrap();
        assert_eq!(s.index_kind(), agenium_store::IndexKind::Ann);
        let live = s.live(1 << 20).unwrap();
        assert_eq!(s.index_status().size, live.len(), "round {round}");
        let live_ids: std::collections::HashSet<i64> = live.iter().map(|f| f.id).collect();
        for f in &live {
            let i: u64 = f.text.strip_prefix("fact ").unwrap().parse().unwrap();
            let hits = s.knn(&ann_vec(i), 5).unwrap();
            if hits.first().map(|h| h.0) != Some(f.id) {
                misses += 1;
            }
            assert!(
                hits.iter().all(|h| live_ids.contains(&h.0)),
                "round {round}: a retired fact was served"
            );
        }
    }
    assert_eq!(misses, 0, "ANN self-hit misses after {rounds} kills");
}
