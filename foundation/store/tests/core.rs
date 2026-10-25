use std::sync::{Arc, Mutex};

use agenium_core::comment::PermissionOutcome;
use agenium_core::forms::{
    PermissionDecision, PermissionKind, PermissionRequest, Question, QuestionCategory, Urgency,
};
use agenium_core::*;
use agenium_store::core::{agents, comments, events, projects, runs, tickets};
use agenium_store::{CoreStore, StoreError};

struct Capture(Mutex<Vec<u64>>);
impl EventSink for Capture {
    fn emit(&self, e: &EventRecord) {
        self.0.lock().unwrap().push(e.seq);
    }
}

fn open(dir: &std::path::Path) -> CoreStore {
    CoreStore::open(&dir.join("db/core.db"), &dir.join("backups")).unwrap()
}

fn new_project(s: &CoreStore, path: &str) -> projects::Project {
    let np = projects::NewProject {
        id: ProjectId::new(),
        name: "demo".into(),
        path: path.into(),
        key_prefix: "AGN".into(),
        main_branch: "main".into(),
    };
    s.commit(None, "user", |w| -> Result<_, StoreError> {
        let p = projects::insert(w.tx, &np)?;
        w.emit(DomainEvent::ProjectCreated {
            project: p.id,
            name: p.name.clone(),
            path: p.path.clone(),
        })?;
        Ok(p)
    })
    .unwrap()
    .value
}

fn solo_agent(s: &CoreStore) -> agents::Agent {
    let team = agents::Team {
        id: TeamId::new(),
        name: "solo".into(),
        template: "coding@1".into(),
        project: None,
        lead: None,
        solo: true,
        created_at: now_iso(),
    };
    let a = agents::Agent {
        id: AgentId::new(),
        name: "ada".into(),
        team: Some(team.id),
        role: "lead".into(),
        role_kind: RoleKind::Lead,
        tags: vec!["rust".into()],
        status: AgentStatus::Idle,
        backend: BackendKind::Api,
        model: ModelBinding {
            provider: "mock".into(),
            model: "m".into(),
            effort: Effort::High,
            max_tokens: None,
        },
        mcp: serde_json::json!([]),
        skills: vec![],
        created_at: now_iso(),
    };
    s.commit(None, "user", |w| -> Result<_, StoreError> {
        agents::insert_team(w.tx, &team)?;
        agents::insert_agent(w.tx, &a)?;
        agents::set_team_lead(w.tx, team.id, Some(a.id))?;
        Ok(())
    })
    .unwrap();
    a
}

fn ticket_insert(s: &CoreStore, p: &projects::Project, deps: Vec<TicketId>) -> Ticket {
    s.commit(None, "user", |w| -> Result<_, StoreError> {
        let key = projects::next_ticket_key(w.tx, p.id)?;
        let t = tickets::insert(
            w.tx,
            &tickets::TicketInsert {
                id: TicketId::new(),
                key: key.clone(),
                project: p.id,
                parent: None,
                kind: TicketKind::Task,
                title: format!("ticket {key}"),
                description: "d".into(),
                acceptance: vec![AcceptanceCriterion {
                    id: "AC-1".into(),
                    text: "works".into(),
                }],
                plan_item: "PI-1".into(),
                priority: 2,
                depends_on: deps,
                required_role: None,
                required_tags: vec![],
                scope_paths: vec!["src/**".into()],
                report_template: "coding@1".into(),
                created_at: now_iso(),
            },
        )?;
        w.emit(DomainEvent::TicketCreated {
            ticket: t.id,
            key: t.key.clone(),
            title: t.title.clone(),
        })?;
        Ok(t)
    })
    .unwrap()
    .value
}

#[test]
fn intent_is_idempotent_and_sink_sees_each_event_once() {
    let tmp = tempfile::tempdir().unwrap();
    let s = open(tmp.path());
    let cap = Arc::new(Capture(Mutex::new(vec![])));
    s.set_sink(Some(cap.clone()));
    let p = new_project(&s, "D:/p1");
    let t = ticket_insert(&s, &p, vec![]);

    let run = |status: TicketStatus| {
        s.commit(
            Some("01INTENT"),
            "user",
            |w| -> Result<String, StoreError> {
                let mut tk = tickets::get(w.tx, t.id)?;
                let from = tk.status;
                tk.status = status;
                tickets::update(w.tx, &tk)?;
                w.emit(DomainEvent::TicketStatusChanged {
                    ticket: tk.id,
                    key: tk.key.clone(),
                    from,
                    to: status,
                })?;
                w.emit(DomainEvent::TicketUpdated {
                    ticket: tk.id,
                    key: tk.key.clone(),
                    fields: vec!["status".into()],
                })?;
                Ok(format!("now {status}"))
            },
        )
        .unwrap()
    };
    let first = run(TicketStatus::Cancelled);
    assert!(!first.replayed);
    assert_eq!(first.events.len(), 2);
    let second = run(TicketStatus::Resolved);
    assert!(second.replayed);
    assert_eq!(
        second.value, "now Cancelled",
        "the stored result is returned"
    );
    assert!(second.events.is_empty());
    assert_eq!(
        s.read(|c| tickets::get(c, t.id)).unwrap().status,
        TicketStatus::Cancelled
    );
    let evs = s.read(|c| events::for_intent(c, "01INTENT")).unwrap();
    assert_eq!(evs.len(), 2);
    assert_eq!(*cap.0.lock().unwrap(), vec![1, 2, 3, 4]);
    assert_eq!(
        s.intent_result("01INTENT").unwrap(),
        Some(serde_json::json!("now Cancelled"))
    );
}

#[test]
fn seq_is_gap_free_across_rollbacks_and_reopen() {
    let tmp = tempfile::tempdir().unwrap();
    {
        let s = open(tmp.path());
        let p = new_project(&s, "D:/p1");
        let r: Result<_, StoreError> = s.commit(None, "user", |w| {
            w.emit(DomainEvent::ProjectStatusChanged {
                project: p.id,
                status: "Paused".into(),
            })?;
            Err::<(), _>(StoreError::Invalid("rule failed".into()))
        });
        assert!(r.is_err());
        ticket_insert(&s, &p, vec![]);
    }
    let s = open(tmp.path());
    let all = s.read(|c| events::since(c, 0, 100)).unwrap();
    assert_eq!(all.iter().map(|e| e.seq).collect::<Vec<_>>(), vec![1, 2]);
    assert_eq!(
        all[1]
            .decode()
            .map(|e| matches!(e, DomainEvent::TicketCreated { .. })),
        Some(true)
    );
    assert_eq!(s.read(events::last_seq).unwrap(), 2);
    assert_eq!(
        s.migration().from,
        agenium_store::core::CORE_MIGRATIONS.last().unwrap().version,
        "reopen sees the already migrated schema"
    );
}

#[test]
fn projects_keys_tickets_and_deps() {
    let tmp = tempfile::tempdir().unwrap();
    let s = open(tmp.path());
    let p = new_project(&s, "D:/p1");
    let dup = projects::NewProject {
        id: ProjectId::new(),
        name: "x".into(),
        path: "D:/p1".into(),
        key_prefix: "AGN".into(),
        main_branch: "main".into(),
    };
    assert!(matches!(
        s.commit(None, "user", |w| projects::insert(w.tx, &dup)),
        Err(StoreError::Conflict(_))
    ));

    let a = ticket_insert(&s, &p, vec![]);
    let b = ticket_insert(&s, &p, vec![a.id]);
    assert_eq!((a.key.as_str(), b.key.as_str()), ("AGN-1", "AGN-2"));
    assert_eq!(b.depends_on, vec![a.key.clone()]);
    assert_eq!(
        s.read(|c| tickets::dependents(c, a.id)).unwrap(),
        vec![b.id]
    );
    let g = s.read(|c| tickets::dep_graph(c, p.id)).unwrap();
    assert_eq!(g.get(&b.id), Some(&vec![a.id]));

    let agent = solo_agent(&s);
    let mut t = s.read(|c| tickets::get_by_key(c, p.id, &b.key)).unwrap();
    t.assignee = Some(agent.id);
    t.status = TicketStatus::Assigned;
    t.usage.tokens_in = 42;
    s.commit(None, "user", |w| tickets::update(w.tx, &t))
        .unwrap();
    let back = s.read(|c| tickets::get(c, t.id)).unwrap();
    assert_eq!(
        (back.assignee, back.status, back.usage.tokens_in),
        (Some(agent.id), TicketStatus::Assigned, 42)
    );
    assert!(back.updated_at >= t.updated_at);
    let open = s.read(|c| tickets::open_for_agent(c, agent.id)).unwrap();
    assert_eq!(open.len(), 1);
    let f = tickets::TicketFilter {
        statuses: vec![TicketStatus::New],
        ..Default::default()
    };
    assert_eq!(s.read(|c| tickets::list(c, p.id, &f)).unwrap().len(), 1);
}

#[test]
fn comments_blocking_inbox_and_grants() {
    let tmp = tempfile::tempdir().unwrap();
    let s = open(tmp.path());
    let p = new_project(&s, "D:/p1");
    let t = ticket_insert(&s, &p, vec![]);
    let agent = solo_agent(&s);
    let q = CommentBody::Question(Question {
        ticket_id: t.key.clone(),
        category: QuestionCategory::Requirement,
        question: "Which redirect URI should dev use?".into(),
        context: String::new(),
        options: vec![],
        recommendation: None,
        blocking: true,
        urgency: Urgency::High,
    });
    let perm_req = PermissionRequest {
        ticket_id: t.key.clone(),
        kind: PermissionKind::Network,
        command: None,
        domains: vec!["crates.io".into()],
        paths: vec![],
        reason: "cargo add oauth2".into(),
    };
    let (qc, pc) = s
        .commit(None, "agent", |w| -> Result<_, StoreError> {
            let qc = comments::insert(
                w.tx,
                CommentId::new(),
                t.id,
                Author::Agent(agent.id),
                None,
                &q,
                &now_iso(),
            )?;
            let body = CommentBody::Permission {
                request: perm_req.clone(),
                run: None,
                outcome: None,
            };
            let pc = comments::insert(
                w.tx,
                CommentId::new(),
                t.id,
                Author::Agent(agent.id),
                None,
                &body,
                &now_iso(),
            )?;
            Ok((qc, pc))
        })
        .unwrap()
        .value;
    assert!(qc.blocking && pc.blocking);
    assert_eq!(
        s.read(|c| comments::open_blocking(c, t.id)).unwrap().len(),
        2
    );
    assert_eq!(s.read(|c| comments::inbox(c, p.id)).unwrap().len(), 2);

    let decided = CommentBody::Permission {
        request: perm_req,
        run: None,
        outcome: Some(PermissionOutcome {
            decision: PermissionDecision::Approve,
            decided_by: "user".into(),
            scope: "run".into(),
            note: None,
        }),
    };
    s.commit(None, "user", |w| -> Result<_, StoreError> {
        comments::set_body(w.tx, pc.id, &decided)?;
        comments::set_resolved(w.tx, pc.id, true)?;
        comments::set_resolved(w.tx, qc.id, true)
    })
    .unwrap();
    assert!(s
        .read(|c| comments::open_blocking(c, t.id))
        .unwrap()
        .is_empty());
    let back = s.read(|c| comments::get(c, pc.id)).unwrap();
    assert!(!back.blocking && back.resolved);

    // Only the user grants always-allow (decision 4): enforced by the schema itself.
    let grant = |scope: &str, by: &str, run: Option<RunId>| runs::Grant {
        id: ulid::Ulid::new().to_string(),
        project: p.id,
        ticket: Some(t.id),
        run,
        scope: scope.into(),
        granted_by: by.into(),
        request: serde_json::json!({}),
    };
    let lead = format!("agent:{}", agent.id);
    let e = s
        .commit(None, "agent", |w| {
            runs::insert_grant(w.tx, &grant("project", &lead, None))
        })
        .unwrap_err();
    assert!(
        matches!(e, StoreError::Invalid(ref m) if m.contains("only be granted by the user")),
        "{e}"
    );
    assert!(s
        .commit(None, "agent", |w| runs::insert_grant(
            w.tx,
            &grant("run", &lead, None)
        ))
        .is_err());
    s.commit(None, "user", |w| {
        runs::insert_grant(w.tx, &grant("project", "user", None))
    })
    .unwrap();
}

/// core_002 (agent tombstones) and core_003 (agent pool: the agent table rebuilt with a nullable team) upgrade a
/// populated v1 database after a pre-migrate snapshot; references to the rebuilt table still hold.
#[test]
fn v1_database_with_data_migrates_to_the_latest() {
    let tmp = tempfile::tempdir().unwrap();
    let db = tmp.path().join("db/core.db");
    std::fs::create_dir_all(db.parent().unwrap()).unwrap();
    {
        let c = rusqlite::Connection::open(&db).unwrap();
        c.execute_batch(include_str!("../src/schema/core_001.sql"))
            .unwrap();
        c.execute_batch(
            "PRAGMA user_version = 1;
             INSERT INTO team(id, name, template) VALUES ('T1', 'solo', 'coding@1');
             INSERT INTO agent(id, name, team, role, role_kind, status, backend, model)
               VALUES ('agent_01', 'ada', 'T1', 'solo', 'lead', 'Idle', 'Api', '{}');
             INSERT INTO run(id, agent, provider, model, started_at)
               VALUES ('run_01', 'agent_01', 'p', 'm', '2026-10-24T00:00:00Z');",
        )
        .unwrap();
    }
    let s = open(tmp.path());
    assert_eq!(s.migration().from, 1);
    assert_eq!(s.migration().to, 4);
    s.commit(None, "user", |w| -> Result<(), StoreError> {
        let c = w.tx;
        let team: Option<String> = c.query_row("SELECT team FROM agent WHERE id = 'agent_01'", [], |r| r.get(0))?;
        assert_eq!(team.as_deref(), Some("T1"));
        let runs: i64 = c.query_row("SELECT count(*) FROM run WHERE agent = 'agent_01'", [], |r| r.get(0))?;
        assert_eq!(runs, 1);
        // In the pool: no team.
        c.execute(
            "INSERT INTO agent(id, name, role, role_kind, status, backend, model)
               VALUES ('agent_02', 'bea', 'r', 'worker', 'Idle', 'Api', '{}')",
            [],
        )?;
        // The run table still references the rebuilt agent table.
        assert!(c
            .execute(
                "INSERT INTO run(id, agent, provider, model, started_at) VALUES ('run_02', 'nobody', 'p', 'm', 'x')",
                [],
            )
            .is_err());
        Ok(())
    })
    .unwrap();
    let names: Vec<String> = s
        .read(|c| -> Result<Vec<String>, StoreError> {
            let mut st = c.prepare("SELECT name FROM agent WHERE deleted_at IS NULL")?;
            let v = st.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
            Ok(v)
        })
        .unwrap();
    assert_eq!(names, vec!["ada", "bea"]);
    let snaps: Vec<_> = std::fs::read_dir(tmp.path().join("backups"))
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    assert!(!snaps.is_empty(), "pre-migrate snapshot written");
}
