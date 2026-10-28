//! ANN sidecar (ADR-50): it answers like exact search, reflects every committed write before the write returns, and
//! a sidecar that is missing, corrupt, foreign or behind a trimmed journal is detected and rebuilt — never served.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use agenium_store::{
    AgentOptions, AgentStore, AnnOptions, IndexKind, IndexMode, NewFact, StoreError,
    DEFAULT_ANN_THRESHOLD,
};

const DIM: usize = 24;

/// Deterministic pseudo-random vectors: `clusters` centres plus noise (real embeddings are clustered, ADR-50 §3).
struct Gen(u64);

impl Gen {
    fn next(&mut self) -> f32 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 40) as f32 / (1u64 << 24) as f32) - 0.5
    }
    fn vec(&mut self) -> Vec<f32> {
        (0..DIM).map(|_| self.next()).collect()
    }
    fn near(&mut self, c: &[f32]) -> Vec<f32> {
        c.iter().map(|x| x + 0.15 * self.next()).collect()
    }
}

fn opts(mode: IndexMode) -> AgentOptions {
    AgentOptions {
        index_mode: mode,
        ann_threshold: DEFAULT_ANN_THRESHOLD,
        ann: AnnOptions {
            save_every: 300,
            build_threads: 4,
            ..AnnOptions::default()
        },
    }
}

fn db(dir: &Path) -> PathBuf {
    dir.join("agents/a.db")
}

fn open(dir: &Path, o: AgentOptions) -> AgentStore {
    AgentStore::open_with(&db(dir), DIM, "toy", &dir.join("backups"), o).unwrap()
}

fn add(s: &AgentStore, i: usize, v: &[f32]) -> i64 {
    s.add(&NewFact::fact(format!("fact number {i}"), "t"), Some(v))
        .unwrap()
}

/// Every live vector finds itself first, but for at most 0.5 % (HNSW is approximate: a parallel build on these tight
/// clusters misses ~0.04 %, a serial one none), and only live facts are ever returned.
fn assert_self_hits(s: &AgentStore, live: &[(i64, Vec<f32>)]) {
    let ids: std::collections::HashSet<i64> = live.iter().map(|l| l.0).collect();
    let mut misses = 0;
    for (id, v) in live {
        let hits = s.knn(v, 3).unwrap();
        assert!(
            hits.iter().all(|h| ids.contains(&h.0)),
            "a fact that is not live was served: {hits:?}"
        );
        if hits.first().map(|h| h.0) != Some(*id) {
            misses += 1;
        }
    }
    assert!(
        misses * 200 <= live.len(),
        "{misses} self-hit misses in {}",
        live.len()
    );
}

fn recall_at_10(s: &AgentStore, queries: &[Vec<f32>]) -> f64 {
    let mut found = 0;
    for q in queries {
        let exact: Vec<i64> = s
            .knn_exact(q, 10)
            .unwrap()
            .into_iter()
            .map(|h| h.0)
            .collect();
        let ann: Vec<i64> = s.knn(q, 10).unwrap().into_iter().map(|h| h.0).collect();
        found += exact.iter().filter(|id| ann.contains(id)).count();
    }
    found as f64 / (queries.len() * 10) as f64
}

fn populate(s: &AgentStore, g: &mut Gen, n: usize) -> Vec<(i64, Vec<f32>)> {
    let centres: Vec<Vec<f32>> = (0..20).map(|_| g.vec()).collect();
    (0..n)
        .map(|i| {
            let v = g.near(&centres[i % centres.len()]);
            (add(s, i, &v), v)
        })
        .collect()
}

#[test]
fn defaults_follow_adr_50_and_125() {
    let o = AgentOptions::default();
    assert_eq!(o.index_mode, IndexMode::Auto);
    assert_eq!(o.ann_threshold, 100_000);
    assert_eq!(DEFAULT_ANN_THRESHOLD, 100_000);
    let a = AnnOptions::default();
    assert_eq!(
        (a.connectivity, a.expansion_add, a.expansion_search),
        (32, 128, 256)
    );
    assert!(a.scale_ef && !a.rerank);
    assert_eq!(a.save_every, 20_000);
}

#[test]
fn ann_answers_like_exact_and_follows_every_write() {
    let tmp = tempfile::tempdir().unwrap();
    let s = open(tmp.path(), opts(IndexMode::Ann));
    assert_eq!(s.index_kind(), IndexKind::Ann);
    let mut g = Gen(7);
    let mut live = populate(&s, &mut g, 1500);
    let queries: Vec<Vec<f32>> = (0..50).map(|_| g.vec()).collect();
    let r = recall_at_10(&s, &queries);
    assert!(r >= 0.98, "recall@10 {r}");

    // Retire and supersede: the old ids are gone from results as soon as the call returns.
    let mut gone = vec![];
    for i in (0..300).step_by(3) {
        let (id, _) = live[i].clone();
        s.retire(id).unwrap();
        gone.push(id);
    }
    for i in (1..300).step_by(3) {
        let (old, _) = live[i].clone();
        let v = g.vec();
        let new = s
            .supersede(
                old,
                &NewFact::fact(format!("replacement {i}"), "t"),
                Some(&v),
            )
            .unwrap();
        gone.push(old);
        live[i] = (new, v);
    }
    live.retain(|(id, _)| !gone.contains(id));
    for q in queries.iter().chain(live.iter().map(|l| &l.1).take(100)) {
        for (id, _) in s.knn(q, 50).unwrap() {
            assert!(!gone.contains(&id), "retired fact {id} served");
        }
    }
    assert_self_hits(&s, &live);
    let st = s.index_status();
    assert_eq!(
        (st.active, st.size, st.last_discard.clone()),
        (IndexKind::Ann, live.len(), None)
    );
}

#[test]
fn reopen_loads_the_saved_sidecar_and_replays_the_journal() {
    let tmp = tempfile::tempdir().unwrap();
    let mut g = Gen(11);
    let live = {
        let s = open(tmp.path(), opts(IndexMode::Ann));
        populate(&s, &mut g, 700)
    };
    // Closed with unsaved changes: saved on close; reopening loads it (no rebuild).
    let s = open(tmp.path(), opts(IndexMode::Ann));
    let st = s.index_status();
    assert_eq!((st.builds, st.last_discard), (0, None));
    assert_self_hits(&s, &live);
    drop(s);

    // Writes made while the sidecar is not loaded (exact mode) are replayed from the journal on the next open.
    let mut live = live;
    {
        let s = open(tmp.path(), opts(IndexMode::Exact));
        for (id, _) in live.drain(..100) {
            s.retire(id).unwrap();
        }
        for i in 0..100 {
            let v = g.vec();
            live.push((add(&s, 10_000 + i, &v), v));
        }
        // A sidecar exists on disk, so the exact-mode maintenance tick keeps the journal.
        s.maintain().unwrap();
    }
    // 100 tombstones in 700 vectors: loaded, then compacted by a fresh build.
    let s = open(tmp.path(), opts(IndexMode::Ann));
    let st = s.index_status();
    assert_eq!((st.builds, st.last_discard, st.size), (1, None, live.len()));
    assert_self_hits(&s, &live);
}

fn corrupt(path: &Path) {
    let mut b = std::fs::read(path).unwrap();
    let at = b.len() / 2;
    b[at] ^= 0x5a;
    std::fs::write(path, b).unwrap();
}

#[test]
fn missing_corrupt_or_foreign_sidecar_is_rebuilt() {
    let tmp = tempfile::tempdir().unwrap();
    let mut g = Gen(13);
    let live = {
        let s = open(tmp.path(), opts(IndexMode::Ann));
        populate(&s, &mut g, 600)
    };
    let ann = PathBuf::from(format!("{}.ann", db(tmp.path()).display()));
    let meta = PathBuf::from(format!("{}.ann.json", db(tmp.path()).display()));

    corrupt(&ann);
    let s = open(tmp.path(), opts(IndexMode::Ann));
    let st = s.index_status();
    assert!(
        st.last_discard.as_deref().unwrap().contains("crc32"),
        "{st:?}"
    );
    assert_eq!((st.builds, st.active), (1, IndexKind::Ann));
    assert_self_hits(&s, &live);
    drop(s);

    std::fs::remove_file(&meta).unwrap();
    let s = open(tmp.path(), opts(IndexMode::Ann));
    assert_eq!(s.index_status().builds, 1);
    assert_self_hits(&s, &live);
    drop(s);

    std::fs::remove_file(&ann).unwrap();
    let s = open(tmp.path(), opts(IndexMode::Ann));
    assert_eq!(s.index_status().builds, 1);
    assert_self_hits(&s, &live);
    drop(s);

    // Another database's sidecar next to this one (copied files) is refused by identity.
    let other = tempfile::tempdir().unwrap();
    {
        let s = open(other.path(), opts(IndexMode::Ann));
        populate(&s, &mut g, 600);
    }
    std::fs::copy(format!("{}.ann", db(other.path()).display()), &ann).unwrap();
    std::fs::copy(format!("{}.ann.json", db(other.path()).display()), &meta).unwrap();
    let s = open(tmp.path(), opts(IndexMode::Ann));
    let st = s.index_status();
    assert!(
        st.last_discard
            .as_deref()
            .unwrap()
            .contains("another database"),
        "{st:?}"
    );
    assert_self_hits(&s, &live);
}

#[test]
fn a_journal_gap_or_a_restored_older_database_forces_a_rebuild() {
    let tmp = tempfile::tempdir().unwrap();
    let mut g = Gen(17);
    let mut live = {
        let s = open(tmp.path(), opts(IndexMode::Ann));
        populate(&s, &mut g, 500)
    };
    // Journal entries the sidecar never saw are lost (trimmed by hand): the replay cannot be trusted.
    {
        let s = open(tmp.path(), opts(IndexMode::Exact));
        for (id, _) in live.drain(..50) {
            s.retire(id).unwrap();
        }
        s.db()
            .write(|tx| Ok::<_, StoreError>(tx.execute("DELETE FROM ann_journal", [])?))
            .unwrap();
    }
    let s = open(tmp.path(), opts(IndexMode::Ann));
    let st = s.index_status();
    assert!(
        st.last_discard.as_deref().unwrap().contains("journal"),
        "{st:?}"
    );
    assert_eq!(st.size, live.len());
    assert_self_hits(&s, &live);
    drop(s);

    // The database goes back in time (a restore) while the sidecar is newer: refused.
    let backup = tmp.path().join("old.db");
    {
        let s = open(tmp.path(), opts(IndexMode::Exact));
        s.db()
            .with_writer(|c| c.execute("VACUUM INTO ?1", [backup.to_str().unwrap()]))
            .unwrap();
    }
    {
        let s = open(tmp.path(), opts(IndexMode::Ann));
        for i in 0..40 {
            let v = g.vec();
            add(&s, 20_000 + i, &v);
        }
    }
    for suffix in ["-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", db(tmp.path()).display()));
    }
    std::fs::copy(&backup, db(tmp.path())).unwrap();
    let s = open(tmp.path(), opts(IndexMode::Ann));
    let st = s.index_status();
    assert!(st.last_discard.is_some(), "{st:?}");
    assert_eq!(st.size, live.len());
    assert_self_hits(&s, &live);
}

#[test]
fn auto_mode_switches_to_ann_at_the_threshold() {
    let tmp = tempfile::tempdir().unwrap();
    let o = AgentOptions {
        ann_threshold: 700,
        ..opts(IndexMode::Auto)
    };
    let mut g = Gen(19);
    let s = Arc::new(open(tmp.path(), o.clone()));
    let mut live = populate(&s, &mut g, 600);
    assert_eq!(s.index_kind(), IndexKind::Exact);
    // Below the threshold and without a sidecar, the maintenance tick empties the journal.
    s.maintain().unwrap();
    let journal = |s: &AgentStore| {
        s.db()
            .read(|c| {
                Ok::<i64, StoreError>(
                    c.query_row("SELECT count(*) FROM ann_journal", [], |r| r.get(0))?,
                )
            })
            .unwrap()
    };
    assert_eq!(journal(&s), 0);
    // The write that reaches 700 starts a background build; exact serves meanwhile.
    let more = populate(&s, &mut g, 99);
    live.extend(more);
    assert_eq!(s.index_kind(), IndexKind::Exact);
    assert!(!s.index_status().building);
    let more = populate(&s, &mut g, 501);
    live.extend(more);
    assert!(s.wait_for_index(Duration::from_secs(60)));
    assert_eq!(s.index_kind(), IndexKind::Ann);
    assert_eq!(s.index_status().size, live.len());
    assert_self_hits(&s, &live);
    drop(s);
    // Over the threshold on open: the saved sidecar is loaded in the background.
    let s = open(tmp.path(), o.clone());
    assert!(s.wait_for_index(Duration::from_secs(60)));
    let st = s.index_status();
    assert_eq!((st.active, st.builds), (IndexKind::Ann, 0), "{st:?}");
    // Back below the threshold: an active sidecar keeps serving…
    for (id, _) in live.drain(..600) {
        s.retire(id).unwrap();
    }
    assert_eq!(s.index_kind(), IndexKind::Ann);
    assert_self_hits(&s, &live);
    drop(s);
    // …but the next open stays exact, and maintenance drops the stale sidecar so the journal can be emptied.
    let s = open(tmp.path(), o);
    assert_eq!(s.index_kind(), IndexKind::Exact);
    s.maintain().unwrap();
    assert!(!PathBuf::from(format!("{}.ann.json", db(tmp.path()).display())).exists());
    assert_eq!(journal(&s), 0);
}

#[test]
fn readers_never_see_a_fact_after_its_retire_returned() {
    let tmp = tempfile::tempdir().unwrap();
    let s = Arc::new(open(tmp.path(), opts(IndexMode::Ann)));
    let mut g = Gen(23);
    let live = populate(&s, &mut g, 400);
    let retired = Arc::new(std::sync::Mutex::new(Vec::<i64>::new()));
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let readers: Vec<_> = (0..3)
        .map(|r| {
            let (s, retired, stop, live) = (s.clone(), retired.clone(), stop.clone(), live.clone());
            std::thread::spawn(move || {
                let mut n = 0;
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    // Snapshot of what was retired *before* the query started.
                    let before = retired.lock().unwrap().clone();
                    let q = &live[(n * 7 + r) % live.len()].1;
                    for (id, _) in s.knn(q, 20).unwrap() {
                        assert!(
                            !before.contains(&id),
                            "fact {id} served after its retire returned"
                        );
                    }
                    n += 1;
                }
                n
            })
        })
        .collect();
    let mut g2 = Gen(29);
    for (i, (id, _)) in live.iter().enumerate().take(200) {
        s.retire(*id).unwrap();
        retired.lock().unwrap().push(*id);
        let v = g2.vec();
        let new = add(&s, 30_000 + i, &v);
        assert_eq!(
            s.knn(&v, 1).unwrap()[0].0,
            new,
            "a fact is found once add returned"
        );
    }
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    for r in readers {
        assert!(r.join().unwrap() > 0);
    }
}

#[test]
fn rerank_returns_exact_distances() {
    let tmp = tempfile::tempdir().unwrap();
    let s = open(
        tmp.path(),
        AgentOptions {
            ann: AnnOptions {
                rerank: true,
                ..opts(IndexMode::Ann).ann
            },
            ..opts(IndexMode::Ann)
        },
    );
    let mut g = Gen(31);
    populate(&s, &mut g, 500);
    for _ in 0..20 {
        let q = g.vec();
        let exact = s.knn_exact(&q, 5).unwrap();
        let ann = s.knn(&q, 5).unwrap();
        assert_eq!(ann.len(), 5);
        assert_eq!(ann[0].0, exact[0].0);
        assert!((ann[0].1 - exact[0].1).abs() < 1e-5, "{ann:?} vs {exact:?}");
    }
}

#[test]
fn reindex_and_vectorless_facts_stay_consistent() {
    let tmp = tempfile::tempdir().unwrap();
    let s = open(tmp.path(), opts(IndexMode::Ann));
    let mut g = Gen(37);
    let live = populate(&s, &mut g, 300);
    // A fact without a vector (and a zero vector) never reaches the index.
    s.add(&NewFact::fact("no vector here", "t"), None).unwrap();
    s.add(&NewFact::fact("zero vector", "t"), Some(&[0.0; DIM]))
        .unwrap();
    assert_eq!(s.index_status().size, live.len());
    // Re-embedding every fact replaces every vector in the sidecar.
    let fresh: std::collections::HashMap<String, Vec<f32>> = s
        .live(10_000)
        .unwrap()
        .into_iter()
        .map(|f| (f.text, g.vec()))
        .collect();
    let f2 = fresh.clone();
    s.reindex(move |texts| Ok(texts.iter().map(|t| f2[t].clone()).collect()))
        .unwrap();
    let live: Vec<(i64, Vec<f32>)> = s
        .live(10_000)
        .unwrap()
        .into_iter()
        .map(|f| (f.id, fresh[&f.text].clone()))
        .collect();
    assert_eq!(s.index_status().size, live.len());
    assert_self_hits(&s, &live);
}
