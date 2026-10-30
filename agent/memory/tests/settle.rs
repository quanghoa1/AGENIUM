//! The settling pass (ADR-145) on a real agent store with vectors set by hand: what is folded, what goes to the user,
//! what the judge is asked and how often.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use agenium_memory::settle::{settle, Order, PairJudge, SettleOptions, Verdict};
use agenium_store::agent::{ConflictChoice, Fact, FactKind, NewFact, PairVerdict};
use agenium_store::AgentStore;
use async_trait::async_trait;

fn store(dir: &std::path::Path) -> AgentStore {
    AgentStore::open(&dir.join("a.db"), 4, "test-4", &dir.join("b")).unwrap()
}

/// A unit vector `angle` degrees from `base` in the plane of axes 0 and 1 (cosine = cos(angle)), or on axis 2/3.
fn at(angle: f64) -> Vec<f32> {
    let r = angle.to_radians();
    vec![r.cos() as f32, r.sin() as f32, 0.0, 0.0]
}

fn add(s: &AgentStore, text: &str, v: &[f32], f: impl FnOnce(&mut NewFact)) -> i64 {
    let mut n = NewFact::fact(text, "ticket:AGN-1");
    f(&mut n);
    s.add(&n, Some(v)).unwrap()
}

/// Answers by the texts it is given; counts its calls.
struct Judge {
    answers: Vec<(&'static str, Option<Verdict>)>,
    calls: AtomicUsize,
    seen: Mutex<Vec<(i64, i64)>>,
}

#[async_trait]
impl PairJudge for Judge {
    async fn judge(&self, a: &Fact, b: &Fact) -> Option<Verdict> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.seen.lock().unwrap().push((a.id, b.id));
        self.answers
            .iter()
            .find(|(w, _)| a.text.contains(w) || b.text.contains(w))
            .and_then(|(_, v)| *v)
    }
}

/// cos 5°-10° ≈ 0.99 (SAME band), cos 22°-40° ≈ 0.77-0.93 (RELATED band), cos 60° and more ≤ 0.5 (unrelated).
#[tokio::test]
async fn paraphrases_fold_changed_figures_go_to_the_user_without_a_judge() {
    let tmp = tempfile::tempdir().unwrap();
    let s = store(tmp.path());
    let a = add(&s, "The dev server listens on port 8080.", &at(0.0), |n| {
        n.confidence = 0.6
    });
    let b = add(&s, "On port 8080 the dev server listens.", &at(5.0), |n| {
        n.confidence = 0.9
    });
    let c = add(
        &s,
        "The dev server listens on port 9090.",
        &at(10.0),
        |_| {},
    );
    let d = add(
        &s,
        "Deploys go through the staging branch.",
        &at(40.0),
        |_| {},
    );
    let e = add(&s, "The CI runs on Linux.", &at(70.0), |_| {});
    // Another trust, project or kind is never compared.
    let f = add(&s, "The dev server listens on port 8080.", &at(0.0), |n| {
        n.untrusted = true
    });
    let g = add(&s, "The dev server listens on port 8080.", &at(0.0), |n| {
        n.project = Some("other".into())
    });
    let h = add(&s, "The dev server listens on port 8080.", &at(0.0), |n| {
        n.kind = FactKind::Lesson
    });

    let r = settle(&s, None, SettleOptions::default()).await.unwrap();
    // a and b say the same: b (more confident) stays, a is folded into it.
    assert_eq!(r.folded(), 1, "{r:?}");
    assert!(!s.get(a).unwrap().is_live());
    assert_eq!(s.get(a).unwrap().superseded_by, Some(b));
    // c has another port: a conflict for the user, both kept.
    let open = s.open_conflicts().unwrap();
    assert_eq!(open.len(), 1, "{open:?}");
    assert_eq!(
        (
            open[0].a.id.min(open[0].b.id),
            open[0].a.id.max(open[0].b.id)
        ),
        (b.min(c), b.max(c))
    );
    assert_eq!(open[0].by, "figures");
    // d is only related and there is no judge: left; e is unrelated.
    assert!(r.left >= 1);
    for id in [b, c, d, e, f, g, h] {
        assert!(s.get(id).unwrap().is_live(), "{id}");
    }

    // A second pass judges nothing again.
    let r2 = settle(&s, None, SettleOptions::default()).await.unwrap();
    assert_eq!(r2.done.len(), 0, "{r2:?}");
    assert!(r2.known >= 1);

    // The user decides: port 9090 holds.
    s.resolve_conflict(
        open[0].a.id,
        open[0].b.id,
        if open[0].a.id == c {
            ConflictChoice::A
        } else {
            ConflictChoice::B
        },
    )
    .unwrap();
    assert!(s.get(c).unwrap().is_live());
    assert!(!s.get(b).unwrap().is_live());
    assert!(s.open_conflicts().unwrap().is_empty());
}

#[tokio::test]
async fn the_judge_decides_the_related_pairs_within_its_questions() {
    let tmp = tempfile::tempdir().unwrap();
    let s = store(tmp.path());
    let a = add(&s, "Releases are tagged from main.", &at(0.0), |_| {});
    let b = add(
        &s,
        "Release tags are cut on the main branch.",
        &at(22.0),
        |_| {},
    );
    s.validate_in(a, "ticket:AGN-1").unwrap();
    s.validate_in(b, "ticket:AGN-2").unwrap();
    let c = add(&s, "Use pnpm, not npm.", &at(100.0), |_| {});
    let d = add(&s, "Use npm for installs.", &at(123.0), |_| {});
    let e = add(&s, "Logs go to stdout.", &at(200.0), |_| {});
    let f = add(
        &s,
        "Logs are also written to logs/app.log.",
        &at(224.0),
        |_| {},
    );
    let judge = Judge {
        answers: vec![
            ("tagged", Some(Verdict::Same)),
            ("pnpm", Some(Verdict::Conflict)),
            ("Logs", Some(Verdict::Distinct)),
        ],
        calls: AtomicUsize::new(0),
        seen: Mutex::new(vec![]),
    };
    // One question only: the closest pair is asked first, the others are left for a later pass.
    let r = settle(
        &s,
        Some(&judge),
        SettleOptions {
            max_questions: 1,
            dry_run: false,
            order: Order::Similarity,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(
        (r.asked, judge.calls.load(Ordering::SeqCst)),
        (1, 1),
        "{r:?}"
    );
    assert_eq!(r.left, 2, "{r:?}");
    // Folded by the judge, the contexts it held in kept together.
    assert_eq!(r.folded(), 1);
    let kept = if s.get(a).unwrap().is_live() { a } else { b };
    assert_eq!(s.get(kept).unwrap().validated.len(), 2);

    let r = settle(&s, Some(&judge), SettleOptions::default())
        .await
        .unwrap();
    assert_eq!(r.asked, 2, "{r:?}");
    assert_eq!(s.open_conflicts().unwrap().len(), 1);
    assert_eq!(s.pair_verdict(e, f).unwrap(), Some(PairVerdict::Distinct));
    assert_eq!(s.pair_verdict(c, d).unwrap(), Some(PairVerdict::Conflict));
    for id in [c, d, e, f] {
        assert!(s.get(id).unwrap().is_live());
    }
    // Nothing asked twice.
    let r = settle(&s, Some(&judge), SettleOptions::default())
        .await
        .unwrap();
    assert_eq!(r.asked, 0, "{r:?}");
    assert_eq!(judge.calls.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn an_unsure_judge_leaves_the_pair_and_a_dry_run_changes_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let s = store(tmp.path());
    let a = add(&s, "The API is versioned in the URL.", &at(0.0), |_| {});
    let b = add(&s, "API versions are in the path.", &at(23.0), |_| {});
    let c = add(&s, "Use tabs.", &at(90.0), |_| {});
    let d = add(&s, "Tabs, use.", &at(92.0), |_| {});
    let judge = Judge {
        answers: vec![],
        calls: AtomicUsize::new(0),
        seen: Mutex::new(vec![]),
    };
    let dry = settle(
        &s,
        Some(&judge),
        SettleOptions {
            max_questions: 5,
            dry_run: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(dry.folded(), 1, "{dry:?}");
    assert_eq!(dry.left, 1, "the unsure verdict leaves a/b: {dry:?}");
    for id in [a, b, c, d] {
        assert!(s.get(id).unwrap().is_live(), "a dry run folded {id}");
    }
    assert_eq!(s.pair_verdict(c, d).unwrap(), None);
    // The real pass: c/d folded, a/b asked again (it was left, not judged).
    let r = settle(&s, Some(&judge), SettleOptions::default())
        .await
        .unwrap();
    assert_eq!(r.folded(), 1);
    assert!(s.get(c).unwrap().is_live() != s.get(d).unwrap().is_live());
    assert_eq!(judge.calls.load(Ordering::SeqCst), 2);
}

/// Parallel facts (one sentence about three or more subjects) are not put to the judge, also when a subject is a word
/// of the sentence too ("the gateway service is called by the gateway"); a value swapped for another is asked first.
#[tokio::test]
async fn parallel_facts_are_not_asked_and_swapped_values_come_first() {
    let tmp = tempfile::tempdir().unwrap();
    let s = store(tmp.path());
    for (i, svc) in ["billing", "search", "gateway", "orders"]
        .iter()
        .enumerate()
    {
        add(
            &s,
            &format!("The {svc} service is called by the gateway over gRPC."),
            &at(30.0 + i as f64 * 3.0),
            |_| {},
        );
    }
    // Far from the family; a value swapped (pnpm / npm), less alike than the family's pairs.
    let a = add(&s, "Use pnpm to install dependencies.", &at(200.0), |_| {});
    let b = add(&s, "Use npm to install dependencies.", &at(225.0), |_| {});
    let judge = Judge {
        answers: vec![("npm", Some(Verdict::Conflict))],
        calls: AtomicUsize::new(0),
        seen: Mutex::new(vec![]),
    };
    let r = settle(
        &s,
        Some(&judge),
        SettleOptions {
            max_questions: 1,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(r.parallel, 6, "{r:?}");
    let seen: Vec<(i64, i64)> = judge
        .seen
        .lock()
        .unwrap()
        .iter()
        .map(|(x, y)| (*x.min(y), *x.max(y)))
        .collect();
    assert_eq!(seen, vec![(a.min(b), a.max(b))]);
    assert_eq!(s.open_conflicts().unwrap().len(), 1);
}

/// ADR-146: the earlier versions of a lesson (what UPDATEs replaced by it), so a reflection on a refined lesson knows
/// what was decided of the one before; a retired fact (its own tombstone) is not its own earlier version.
#[test]
fn a_fact_knows_its_earlier_versions() {
    let tmp = tempfile::tempdir().unwrap();
    let s = store(tmp.path());
    let v1 = add(&s, "Team documents start with Owner.", &at(0.0), |_| {});
    let v2 = s
        .supersede(
            v1,
            &NewFact::fact(
                "Team documents in deliverables/ start with Owner: docs-team.",
                "ticket:AGN-2",
            ),
            Some(&at(1.0)),
        )
        .unwrap();
    let v3 = s
        .supersede(
            v2,
            &NewFact::fact(
                "Every team document starts with Owner: docs-team.",
                "ticket:AGN-3",
            ),
            Some(&at(2.0)),
        )
        .unwrap();
    let other = add(&s, "Logs go to stdout.", &at(90.0), |_| {});
    s.retire(other).unwrap();
    assert_eq!(s.earlier_versions(v3).unwrap(), vec![v2, v1]);
    assert_eq!(s.earlier_versions(v1).unwrap(), Vec::<i64>::new());
    assert_eq!(s.earlier_versions(other).unwrap(), Vec::<i64>::new());
}
