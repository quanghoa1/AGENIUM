//! M15-5: agent export/import round trip and agent deletion (decision 32).

mod common;

use std::path::Path;

use agenium_core::forms::CancelTicket;
use agenium_core::*;
use agenium_orchestrator::agent_io::{ExportOptions, ImportRequest};
use agenium_store::core::events;
use agenium_store::{NewFact, Via};
use common::*;

fn emb() -> std::sync::Arc<dyn agenium_memory::Embedder> {
    std::sync::Arc::new(agenium_memory::HashEmbedder::new(384))
}

/// Memory, files and profile of an agent with some history.
fn seed(f: &Fixture, id: AgentId) {
    let e = emb();
    let dir = f.app.layout().agent_dir(&id);
    write(
        &dir,
        "SOUL.md",
        "You are Ada. You verify before you claim.\n",
    );
    write(
        &dir,
        "skills/csv-report/SKILL.md",
        "---\nname: csv-report\ndescription: CSV reports.\n---\nSee [logo](assets/logo.bin).\n",
    );
    std::fs::create_dir_all(dir.join("skills/csv-report/assets")).unwrap();
    std::fs::write(
        dir.join("skills/csv-report/assets/logo.bin"),
        [0u8, 159, 146, 150, 255],
    )
    .unwrap();
    let mem = f.app.agent_store(id, e.dim(), e.model_id()).unwrap();
    let v = |t: &str| e.embed(&[t.to_owned()]).unwrap().remove(0);
    let a = mem
        .add(
            &NewFact::fact("The project uses port 8080", "ticket:AGN-1"),
            Some(&v("port 8080")),
        )
        .unwrap();
    let b = mem
        .supersede(
            a,
            &NewFact::fact("The project uses port 9090", "ticket:AGN-2"),
            Some(&v("port 9090")),
        )
        .unwrap();
    mem.add(
        &NewFact::fact("The user prefers short reports", "chat"),
        Some(&v("short reports")),
    )
    .unwrap();
    mem.record_usage("run-1", Some("AGN-2"), &[(b, 1, Via::Hybrid)])
        .unwrap();
    mem.append_message("run-1", &serde_json::json!({"role": "user", "parts": []}))
        .unwrap();
    // Graph memory travels with the agent.
    let ent = |n: &str| agenium_store::graph::NewEntity {
        kind: "service".into(),
        name: n.into(),
        scope: serde_json::json!({}),
        description: None,
        aliases: vec![],
        confidence: 0.6,
        evidence: None,
    };
    let (api, _) = mem.upsert_entity(&ent("API"), None).unwrap();
    let (db, _) = mem.upsert_entity(&ent("database"), None).unwrap();
    mem.add_edge(
        api,
        db,
        "depends_on",
        &serde_json::json!({}),
        0.7,
        Some(b),
        "extract",
    )
    .unwrap();
    mem.link_fact(b, api, "subject").unwrap();
    f.app
        .set_agent_skills(id, &["csv-report".to_string()])
        .unwrap();
}

fn memory_lines(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .filter(|l| l.contains("\"kind\":\"table\"") || l.contains("\"kind\":\"row\""))
        .map(str::to_owned)
        .collect()
}

#[test]
fn export_import_round_trip() {
    let f = fixture();
    let a = solo_with_plan(&f, &[("PI-1", &[])]);
    seed(&f, a.id);
    let out = f._tmp.path().join("ada.agent.jsonl");
    let s = f
        .app
        .export_agent(a.id, &out, emb().as_ref(), &ExportOptions::default())
        .unwrap();
    assert_eq!(s.files, 3, "SOUL.md + SKILL.md + asset");
    assert_eq!(
        s.memory_rows,
        3 + 2 + 2 + 1 + 1 + 1 + 1,
        "facts, entities, aliases, edge, link, usage, message"
    );

    // Import on "another machine".
    let g = fixture();
    let b = g
        .app
        .import_agent(
            &out,
            &ImportRequest {
                project: g.project.id,
                name: None,
            },
            emb().as_ref(),
        )
        .unwrap();
    assert_ne!(b.id, a.id);
    assert_eq!(b.name, "ada");
    assert_eq!(b.model, a.model);
    assert_eq!(b.tags, a.tags);
    assert_eq!(b.skills, vec!["csv-report"]);
    let (src, dst) = (
        f.app.layout().agent_dir(&a.id),
        g.app.layout().agent_dir(&b.id),
    );
    for rel in [
        "SOUL.md",
        "skills/csv-report/SKILL.md",
        "skills/csv-report/assets/logo.bin",
    ] {
        assert_eq!(
            std::fs::read(src.join(rel)).unwrap(),
            std::fs::read(dst.join(rel)).unwrap(),
            "{rel}"
        );
    }
    let mem = g
        .app
        .agent_store(b.id, emb().dim(), emb().model_id())
        .unwrap();
    assert_eq!(mem.count_live().unwrap(), 2);
    let hits = mem
        .knn(&emb().embed(&["port 9090".into()]).unwrap()[0], 1)
        .unwrap();
    assert_eq!(
        mem.get(hits[0].0).unwrap().text,
        "The project uses port 9090"
    );
    assert_eq!(mem.load_messages("run-1").unwrap().len(), 1);
    let linked = mem.entities_in_text("the API is slow", 5).unwrap();
    assert_eq!(linked.len(), 1, "entity index rebuilt after import");
    let r = mem
        .expand(&[linked[0].id], &serde_json::json!({}), &Default::default())
        .unwrap();
    assert_eq!(r.reached.len(), 1, "edge imported");

    // Exporting the imported agent gives the same memory, byte for byte.
    let again = g._tmp.path().join("ada-again.agent.jsonl.zst");
    g.app
        .export_agent(b.id, &again, emb().as_ref(), &ExportOptions::default())
        .unwrap();
    let plain = g._tmp.path().join("ada-again.agent.jsonl");
    std::fs::write(
        &plain,
        zstd::decode_all(std::fs::File::open(&again).unwrap()).unwrap(),
    )
    .unwrap();
    assert_eq!(memory_lines(&out), memory_lines(&plain));

    // Same name again is refused; a new name works.
    assert!(g
        .app
        .import_agent(
            &out,
            &ImportRequest {
                project: g.project.id,
                name: None,
            },
            emb().as_ref(),
        )
        .is_err());
}

#[test]
fn tampered_or_secret_exports_are_refused() {
    let f = fixture();
    let a = solo_with_plan(&f, &[("PI-1", &[])]);
    seed(&f, a.id);
    let out = f._tmp.path().join("ada.agent.jsonl");
    f.app
        .export_agent(a.id, &out, emb().as_ref(), &ExportOptions::default())
        .unwrap();
    let text = std::fs::read_to_string(&out).unwrap();
    let bad = f._tmp.path().join("bad.agent.jsonl");
    std::fs::write(&bad, text.replace("port 9090", "port 6666")).unwrap();
    let g = fixture();
    let e = g
        .app
        .import_agent(
            &bad,
            &ImportRequest {
                project: g.project.id,
                name: None,
            },
            emb().as_ref(),
        )
        .unwrap_err()
        .to_string();
    assert!(e.contains("checksum"), "{e}");
    assert!(g.app.agents().unwrap().is_empty(), "nothing was created");

    // A stored secret value anywhere in the data refuses the export.
    f.app
        .redactor()
        .register("db-password", "hunter2-correct-horse");
    let mem = f
        .app
        .agent_store(a.id, emb().dim(), emb().model_id())
        .unwrap();
    mem.add(
        &NewFact::fact("The DB password is hunter2-correct-horse", "chat"),
        None,
    )
    .unwrap();
    let e = f
        .app
        .export_agent(
            a.id,
            &f._tmp.path().join("x.jsonl"),
            emb().as_ref(),
            &ExportOptions {
                allow_secret_findings: true,
            },
        )
        .unwrap_err()
        .to_string();
    assert!(e.contains("stored secret"), "{e}");
    assert!(!f._tmp.path().join("x.jsonl").exists());
}

#[test]
fn delete_agent_rules() {
    let f = fixture();
    let a = solo_with_plan(&f, &[("PI-1", &[])]);
    seed(&f, a.id);
    let lead = Actor::Lead(a.id);
    f.app
        .apply(
            &ctx(&f, lead),
            Intent::CreateTicket(new_ticket("PI-1", "t", &[], &[])),
        )
        .unwrap();

    let e = f.app.delete_agent(a.id, "Ada").unwrap_err().to_string();
    assert!(e.contains("exact name"), "{e}");
    let e = f.app.delete_agent(a.id, "ada").unwrap_err().to_string();
    assert!(e.contains("open tickets (AGN-1)"), "{e}");

    f.app
        .apply(
            &ctx(&f, lead),
            Intent::CancelTicket(CancelTicket {
                ticket_id: key("AGN-1"),
                reason: "not needed any more".into(),
                replaced_by: None,
            }),
        )
        .unwrap();
    let r = f.app.delete_agent(a.id, "ada").unwrap();
    assert!(r.pending.is_empty(), "{:?}", r.pending);
    assert!(!f.app.layout().agent_dir(&a.id).exists());
    assert!(!f.app.layout().agent_db(&a.id).exists());
    assert!(f.app.agents().unwrap().is_empty());
    let tomb = f.app.agent(a.id).unwrap();
    assert!(tomb.name.starts_with("ada~") && tomb.status == AgentStatus::Offline);
    assert_eq!(
        f.app.ticket(f.project.id, &key("AGN-1")).unwrap().status,
        TicketStatus::Cancelled,
        "history keeps its references"
    );
    let evs = f.app.store().read(|c| events::since(c, 0, 10_000)).unwrap();
    assert!(evs.iter().any(|e| e.kind == "AgentDeleted"));
    assert!(
        f.app.delete_agent(a.id, &tomb.name).is_err(),
        "already deleted"
    );

    // The name is free and the project can get a new Solo agent.
    let (_, b) = f
        .app
        .create_solo(
            f.project.id,
            &agent_spec("ada", "solo", RoleKind::Lead, &[]),
        )
        .unwrap();
    assert_eq!(f.app.agents().unwrap()[0].id, b.id);
}
