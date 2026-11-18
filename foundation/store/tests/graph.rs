//! Graph memory store (ADR-51, M15-7/9).

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::Duration;

use agenium_store::graph::*;
use agenium_store::{AgentStore, NewFact};
use proptest::prelude::*;
use serde_json::{json, Value};

const DIM: usize = 16;

fn vecf(seed: u64) -> Vec<f32> {
    let mut v: Vec<f32> = (0..DIM as u64)
        .map(|i| ((seed * 31 + i * 7) % 13) as f32 + 0.1)
        .collect();
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    v.iter_mut().for_each(|x| *x /= n);
    v
}

fn open(dir: &std::path::Path) -> AgentStore {
    AgentStore::open(&dir.join("agents/a.db"), DIM, "toy", &dir.join("backups")).unwrap()
}

fn ent(s: &AgentStore, kind: &str, name: &str, scope: Value) -> i64 {
    s.upsert_entity(
        &NewEntity {
            kind: kind.into(),
            name: name.into(),
            scope,
            description: None,
            aliases: vec![],
            confidence: 0.9,
            evidence: None,
        },
        Some(&vecf(name.len() as u64)),
    )
    .unwrap()
    .0
}

fn x(depth: u32) -> Expansion {
    Expansion {
        max_depth: depth,
        hub_degree: 200,
        limit: 1000,
        budget: Duration::from_secs(5),
    }
}

fn names(s: &AgentStore, r: &ExpandResult) -> Vec<String> {
    let mut v: Vec<String> = r
        .reached
        .iter()
        .map(|n| s.entity(n.entity).unwrap().name)
        .collect();
    v.sort();
    v
}

#[test]
fn v1_database_migrates_to_the_graph_schema() {
    let tmp = tempfile::tempdir().unwrap();
    let db = tmp.path().join("agents/a.db");
    std::fs::create_dir_all(db.parent().unwrap()).unwrap();
    {
        let c = rusqlite::Connection::open(&db).unwrap();
        c.execute_batch(include_str!("../src/schema/agent_001.sql"))
            .unwrap();
        c.execute_batch(
            "PRAGMA user_version = 1;
             INSERT INTO agent_meta(key, value) VALUES ('embedding_dim', '16');
             INSERT INTO fact(text, kind, source) VALUES ('The API listens on port 8080', 'fact', 'ticket:AGN-1');",
        )
        .unwrap();
    }
    let s = open(tmp.path());
    assert_eq!(s.live(10).unwrap().len(), 1, "facts survive");
    assert!(
        std::fs::read_dir(tmp.path().join("backups"))
            .unwrap()
            .next()
            .is_some(),
        "pre-migrate snapshot"
    );
    let a = ent(&s, "service", "API", json!({}));
    let fact = s.live(10).unwrap()[0].id;
    s.link_fact(fact, a, "subject").unwrap();
    assert_eq!(s.fact_entities(fact).unwrap(), vec![a]);
}

#[test]
fn identity_aliases_and_linking() {
    let tmp = tempfile::tempdir().unwrap();
    let s = open(tmp.path());
    let n = NewEntity {
        kind: "Product".into(),
        name: "Archer C7".into(),
        scope: json!({"vendor": "TP-Link"}),
        description: Some("A Wi-Fi router.".into()),
        aliases: vec!["AC1750".into()],
        confidence: 0.8,
        evidence: Some("ticket:AGN-1".into()),
    };
    let (id, created) = s.upsert_entity(&n, Some(&vecf(1))).unwrap();
    assert!(created);
    let (again, created) = s
        .upsert_entity(
            &NewEntity {
                name: "archer-c7".into(),
                ..n.clone()
            },
            None,
        )
        .unwrap();
    assert_eq!(
        (again, created),
        (id, false),
        "same type + canonical name + identity scope"
    );
    assert_eq!(s.entity(id).unwrap().mentions, 2);
    // Same name, other vendor: a different entity.
    let (other, created) = s
        .upsert_entity(
            &NewEntity {
                scope: json!({"vendor": "netgear"}),
                ..n.clone()
            },
            None,
        )
        .unwrap();
    assert!(created && other != id);

    let c = s.entity_candidates("product", "AC1750", None, 5).unwrap();
    assert_eq!(c[0].entity.id, id);
    assert_eq!(c[0].via, "alias");
    let c = s
        .entity_candidates("product", "Archer router", None, 5)
        .unwrap();
    assert!(c.iter().any(|x| x.entity.id == id && x.via == "fts"));
    assert!(
        s.entity_candidates("vendor", "Archer C7", None, 5)
            .unwrap()
            .is_empty(),
        "type must match"
    );
    let found = s
        .entities_in_text("The Archer C7 drops DHCP leases", 5)
        .unwrap();
    assert_eq!(found.len(), 2, "both Archer C7 entities are linked by name");
}

#[test]
fn traversal_depth_direction_cycles_scope_and_hubs() {
    let tmp = tempfile::tempdir().unwrap();
    let s = open(tmp.path());
    let router = ent(&s, "product", "Archer C7", json!({"vendor": "tp-link"}));
    let fw = ent(&s, "software", "firmware 1.2", json!({}));
    let bug = ent(&s, "error", "DHCP lease loss", json!({}));
    let fix = ent(&s, "concept", "static lease workaround", json!({}));
    let ng = ent(&s, "concept", "netgear reboot workaround", json!({}));
    s.add_edge(router, fw, "uses", &json!({}), 0.9, None, "extract")
        .unwrap();
    s.add_edge(fw, bug, "causes", &json!({}), 0.9, None, "extract")
        .unwrap();
    s.add_edge(
        fix,
        bug,
        "workaround_for",
        &json!({"vendor": "tp-link"}),
        0.9,
        None,
        "extract",
    )
    .unwrap();
    s.add_edge(
        ng,
        bug,
        "workaround_for",
        &json!({"vendor": "netgear"}),
        0.9,
        None,
        "extract",
    )
    .unwrap();
    s.add_edge(bug, router, "related_to", &json!({}), 0.5, None, "extract")
        .unwrap(); // cycle
    assert_eq!(s.entity(bug).unwrap().degree, 4);
    // Re-observing an edge strengthens it instead of duplicating it.
    s.add_edge(router, fw, "uses", &json!({}), 0.95, None, "extract")
        .unwrap();
    assert_eq!(s.entity(router).unwrap().degree, 2);

    let ctx = json!({"vendor": "tp-link"});
    assert_eq!(
        names(&s, &s.expand(&[router], &ctx, &x(1)).unwrap()),
        vec!["DHCP lease loss", "firmware 1.2"]
    );
    let r = s.expand(&[router], &ctx, &x(2)).unwrap();
    assert_eq!(
        names(&s, &r),
        vec!["DHCP lease loss", "firmware 1.2", "static lease workaround"],
        "backward arm"
    );
    assert!(r.reached.iter().all(|n| n.hops <= 2));
    // Netgear context: the TP-Link workaround edge is not crossed, the Netgear one is.
    let r = s
        .expand(&[bug], &json!({"vendor": "netgear"}), &x(1))
        .unwrap();
    let n = names(&s, &r);
    assert!(
        n.contains(&"netgear reboot workaround".to_string())
            && !n.contains(&"static lease workaround".to_string()),
        "{n:?}"
    );
    // No context key: both are kept (unknown scope is not a conflict).
    let n = names(&s, &s.expand(&[bug], &json!({}), &x(1)).unwrap());
    assert!(
        n.contains(&"static lease workaround".to_string())
            && n.contains(&"netgear reboot workaround".to_string())
    );
    // Hub cap: a node with more live edges than the cap is not expanded.
    let capped = Expansion {
        hub_degree: 3,
        ..x(2)
    };
    let n = names(&s, &s.expand(&[router], &ctx, &capped).unwrap());
    assert_eq!(
        n,
        vec!["DHCP lease loss", "firmware 1.2"],
        "the bug (degree 4) is reached but not expanded"
    );
    // Ended edges are not followed.
    let e = s
        .edges_of(fw)
        .unwrap()
        .into_iter()
        .find(|e| e.kind == "causes")
        .unwrap();
    s.invalidate_edge(e.id, None).unwrap();
    assert_eq!(s.entity(bug).unwrap().degree, 3);
    assert_eq!(
        names(&s, &s.expand(&[fw], &ctx, &x(1)).unwrap()),
        vec!["Archer C7"]
    );

    // Facts attached to reached entities, with the scope filter on the fact itself.
    let f1 = s
        .add(
            &NewFact {
                scope: json!({"vendor": "tp-link"}),
                ..NewFact::fact("Pin DHCP leases on TP-Link firmware 1.2", "t")
            },
            None,
        )
        .unwrap();
    let f2 = s
        .add(
            &NewFact {
                scope: json!({"vendor": "netgear"}),
                ..NewFact::fact("Reboot the Netgear router nightly", "t")
            },
            None,
        )
        .unwrap();
    s.link_fact(f1, fix, "subject").unwrap();
    s.link_fact(f2, ng, "subject").unwrap();
    let got = s
        .facts_of_entities(&[(fix, 0.9), (ng, 0.9)], &json!({"vendor": "netgear"}), 10)
        .unwrap();
    assert_eq!(
        got.iter().map(|(f, _)| f.id).collect::<Vec<_>>(),
        vec![f2],
        "TP-Link lesson excluded for Netgear"
    );
}

#[test]
fn query_plan_uses_the_partial_edge_indexes() {
    let tmp = tempfile::tempdir().unwrap();
    let s = open(tmp.path());
    let plan: Vec<String> = s
        .db()
        .read(|c| -> Result<Vec<String>, agenium_store::StoreError> {
            let mut st = c.prepare(
                "EXPLAIN QUERY PLAN SELECT e.dst FROM edge e INDEXED BY edge_out WHERE e.src = 1 AND e.valid_to IS NULL
                 UNION ALL SELECT e.src FROM edge e INDEXED BY edge_in WHERE e.dst = 1 AND e.valid_to IS NULL",
            )?;
            let v = st.query_map([], |r| r.get::<_, String>(3))?.collect::<Result<_, _>>()?;
            Ok(v)
        })
        .unwrap();
    let text = plan.join(" | ");
    assert!(
        text.contains("edge_out") && text.contains("edge_in"),
        "{text}"
    );
    assert!(!text.contains("SCAN e"), "no full scan: {text}");
}

#[test]
fn merging_rewires_and_logs() {
    let tmp = tempfile::tempdir().unwrap();
    let s = open(tmp.path());
    let a = ent(&s, "library", "serde json", json!({}));
    let b = ent(&s, "library", "serde_json crate", json!({}));
    let c = ent(&s, "language", "Rust", json!({}));
    let d = ent(&s, "concept", "parsing", json!({}));
    s.add_edge(a, c, "uses", &json!({}), 0.8, None, "extract")
        .unwrap();
    s.add_edge(b, c, "uses", &json!({}), 0.8, None, "extract")
        .unwrap(); // becomes a duplicate
    s.add_edge(b, d, "uses", &json!({}), 0.8, None, "extract")
        .unwrap();
    s.add_edge(a, b, "related_to", &json!({}), 0.5, None, "extract")
        .unwrap(); // becomes a self-loop
    let f = s
        .add(&NewFact::fact("serde_json parses JSON", "t"), None)
        .unwrap();
    s.link_fact(f, b, "subject").unwrap();
    s.merge_entities(b, a, Some(0.93), "auto").unwrap();

    assert_eq!(s.entity(b).unwrap().merged_into, Some(a));
    assert_eq!(s.live_entity(b).unwrap().id, a);
    assert_eq!(
        s.fact_entities(f).unwrap(),
        vec![a, b],
        "link copied to the survivor (old row kept for undo)"
    );
    let live: Vec<_> = s
        .edges_of(a)
        .unwrap()
        .into_iter()
        .filter(|e| e.valid_to.is_none())
        .collect();
    assert_eq!(
        live.len(),
        2,
        "uses Rust (weight 2) + uses parsing: {live:?}"
    );
    assert!(live.iter().any(|e| e.dst == c && e.weight >= 2.0));
    assert_eq!(s.entity(a).unwrap().degree, 2);
    assert_eq!(s.entity(c).unwrap().degree, 1);
    assert!(s
        .entity_candidates("library", "serde_json crate", None, 5)
        .unwrap()
        .iter()
        .all(|x| x.entity.id == a));
    let log: i64 = s
        .db()
        .read(|c| -> Result<i64, agenium_store::StoreError> {
            Ok(c.query_row(
                "SELECT count(*) FROM entity_merge_log WHERE from_id = ?1",
                [b],
                |r| r.get(0),
            )?)
        })
        .unwrap();
    assert_eq!(log, 1);
    // Different types or conflicting identity scopes never merge.
    assert!(s.merge_entities(c, a, None, "auto").is_err());
    let t1 = ent(&s, "product", "router", json!({"vendor": "tp-link"}));
    let t2 = ent(&s, "product", "router", json!({"vendor": "netgear"}));
    assert!(s.merge_entities(t1, t2, Some(0.99), "auto").is_err());
}

#[test]
fn time_budget_interrupts_expansion() {
    let tmp = tempfile::tempdir().unwrap();
    let s = open(tmp.path());
    // A dense graph: every node linked to the next 30 (paths explode at depth 3).
    let ids: Vec<i64> = (0..300)
        .map(|i| ent(&s, "concept", &format!("n{i}"), json!({})))
        .collect();
    for i in 0..ids.len() {
        for j in 1..=30 {
            s.add_edge(
                ids[i],
                ids[(i + j) % ids.len()],
                "related_to",
                &json!({}),
                0.9,
                None,
                "extract",
            )
            .unwrap();
        }
    }
    let r = s
        .expand(
            &[ids[0]],
            &json!({}),
            &Expansion {
                max_depth: 3,
                hub_degree: 1000,
                limit: 50,
                budget: Duration::from_millis(1),
            },
        )
        .unwrap();
    assert!(r.timed_out, "budget enforced");
    let r = s
        .expand(
            &[ids[0]],
            &json!({}),
            &Expansion {
                max_depth: 1,
                ..x(1)
            },
        )
        .unwrap();
    assert_eq!(r.reached.len(), 60, "30 out + 30 in");
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    /// Random graphs with cycles: the expansion matches a reference BFS (same reachable set within the depth,
    /// minimal hop counts), never crosses a conflicting edge and terminates.
    #[test]
    fn expansion_matches_reference_bfs(
        n in 3usize..18,
        edges in proptest::collection::vec((0usize..18, 0usize..18, any::<bool>()), 1..60),
        depth in 1u32..4,
        seed in 0usize..18,
    ) {
        let tmp = tempfile::tempdir().unwrap();
        let s = open(tmp.path());
        let ids: Vec<i64> = (0..n).map(|i| ent(&s, "concept", &format!("node {i}"), json!({}))).collect();
        let mut adj: HashMap<usize, HashSet<usize>> = HashMap::new();
        for (a, b, scoped) in edges {
            let (a, b) = (a % n, b % n);
            if a == b { continue; }
            let scope = if scoped { json!({"vendor": "other"}) } else { json!({}) };
            s.add_edge(ids[a], ids[b], "related_to", &scope, 0.9, None, "extract").unwrap();
            if !scoped {
                adj.entry(a).or_default().insert(b);
                adj.entry(b).or_default().insert(a);
            }
        }
        let seed = seed % n;
        // Reference BFS over unscoped edges only (context vendor=mine conflicts with "other").
        let mut dist: HashMap<usize, u32> = HashMap::from([(seed, 0)]);
        let mut q = VecDeque::from([seed]);
        while let Some(u) = q.pop_front() {
            let du = dist[&u];
            if du == depth {
                continue;
            }
            for &v in adj.get(&u).into_iter().flatten() {
                if let std::collections::hash_map::Entry::Vacant(e) = dist.entry(v) {
                    e.insert(du + 1);
                    q.push_back(v);
                }
            }
        }
        let r = s.expand(&[ids[seed]], &json!({"vendor": "mine"}), &x(depth)).unwrap();
        prop_assert!(!r.timed_out);
        let got: HashMap<i64, u32> = r.reached.iter().map(|n| (n.entity, n.hops)).collect();
        let want: HashMap<i64, u32> = dist.iter().filter(|(k, _)| **k != seed).map(|(k, d)| (ids[*k], *d)).collect();
        prop_assert_eq!(got, want);
    }
}
