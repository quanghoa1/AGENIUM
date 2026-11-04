//! Undoing a learning run: its facts out, those it replaced or retired back; a proven lesson asks first; once only.

use super::*;
use agenium_store::{FactKind, NewFact};

fn learning_run(f: &Fixture, agent: AgentId, changes: serde_json::Value) -> RunId {
    let run = RunId::new();
    f.app
        .store()
        .commit(None, "system", |w| {
            agenium_store::core::runs::insert(
                w.tx,
                &runs::Run {
                    id: run,
                    ticket: None,
                    agent,
                    provider: "mock".into(),
                    model: "m".into(),
                    started_at: "2026-11-04T00:00:00Z".into(),
                    ended_at: None,
                    outcome: None,
                    steps: 1,
                    usage: UsageSummary::default(),
                    heartbeat: None,
                },
            )
        })
        .unwrap();
    f.app
        .run_log(
            run,
            "memory",
            serde_json::json!({ "extracted": 2, "changes": changes }),
        )
        .unwrap();
    run
}

#[test]
fn a_learning_run_is_undone_once_and_a_proven_lesson_asks_first() {
    let f = &fixture();
    let (_, a) = f
        .app
        .create_solo(
            f.project.id,
            &agent_spec("ada", "solo", RoleKind::Lead, &["rust"]),
        )
        .unwrap();
    let mem = f.app.agent_store(a.id, 8, "m").unwrap();
    let old = mem
        .add(
            &NewFact::fact("Tests run with cargo test", "ticket:AGN-1"),
            None,
        )
        .unwrap();
    // The episode: one lesson added (proven since), the old fact replaced.
    let mut lesson = NewFact::fact("Run clippy before you submit", "ticket:AGN-2");
    lesson.kind = FactKind::Lesson;
    let added = mem.add(&lesson, None).unwrap();
    let upd = mem
        .supersede(
            old,
            &NewFact::fact("Tests run with cargo nextest", "ticket:AGN-2"),
            None,
        )
        .unwrap();
    mem.validate_in(added, "P/ticket:AGN-3").unwrap();
    let run = learning_run(
        f,
        a.id,
        serde_json::json!([
            {"op": "add", "kind": "lesson", "text": "Run clippy before you submit", "id": added},
            {"op": "update", "kind": "fact", "text": "Tests run with cargo nextest", "id": upd, "old_id": old},
        ]),
    );
    let r = f.app.revert_memory_run(run, 8, "m", false).unwrap();
    assert_eq!(r["done"], false, "{r}");
    assert_eq!(r["warnings"].as_array().unwrap().len(), 1, "{r}");
    assert!(
        mem.get(added).unwrap().superseded_by.is_none(),
        "nothing changed without force"
    );
    let r = f.app.revert_memory_run(run, 8, "m", true).unwrap();
    assert_eq!(r["done"], true);
    assert_eq!(r["undone"], 2);
    let live: Vec<i64> = mem.live(100).unwrap().iter().map(|f| f.id).collect();
    assert_eq!(live, vec![old], "only the fact before the episode is live");
    let again = f.app.revert_memory_run(run, 8, "m", true).unwrap_err();
    assert!(again.to_string().contains("undone already"), "{again}");
    // A run learned before changes named their facts cannot be undone.
    let legacy = learning_run(
        f,
        a.id,
        serde_json::json!([{"op": "add", "kind": "fact", "text": "x"}]),
    );
    assert!(f.app.revert_memory_run(legacy, 8, "m", false).is_err());
}
