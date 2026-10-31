use agenium_core::{DomainEvent, ProjectId};
use agenium_store::backup::{self, BackupKind, Retention};
use agenium_store::core::{events, projects};
use agenium_store::{AgentStore, CoreStore, NewFact, StoreError};
use chrono::{Duration, TimeZone, Utc};

fn write_events(s: &CoreStore, project: ProjectId, n: usize, tag: &str) {
    for i in 0..n {
        s.commit(
            Some(&format!("{tag}-{i}")),
            "user",
            |w| -> Result<(), StoreError> {
                w.emit(DomainEvent::ProjectStatusChanged {
                    project,
                    status: format!("{tag}{i}"),
                })?;
                Ok(())
            },
        )
        .unwrap();
    }
}

#[test]
fn full_and_incremental_restore_match_content_hashes() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let backups = root.join("backups");
    let core = CoreStore::open(&root.join("db/core.db"), &backups).unwrap();
    let agent = AgentStore::open(&root.join("db/agents/agent_x.db"), 4, "m", &backups).unwrap();
    let np = projects::NewProject {
        id: ProjectId::new(),
        name: "p".into(),
        path: "X:/p".into(),
        key_prefix: "AGN".into(),
        main_branch: "main".into(),
    };
    let p = core
        .commit(None, "user", |w| projects::insert(w.tx, &np))
        .unwrap()
        .value;
    write_events(&core, p.id, 50, "a");
    agent
        .add(
            &NewFact::fact("first fact", "chat"),
            Some(&[1.0, 0.0, 0.0, 0.0]),
        )
        .unwrap();
    let ent = |n: &str| agenium_store::graph::NewEntity {
        kind: "service".into(),
        name: n.into(),
        scope: serde_json::json!({}),
        description: None,
        aliases: vec![],
        confidence: 0.5,
        evidence: None,
    };
    let (a, _) = agent
        .upsert_entity(&ent("api"), Some(&[0.0, 1.0, 0.0, 0.0]))
        .unwrap();
    let (b, _) = agent.upsert_entity(&ent("db"), None).unwrap();
    agent
        .add_edge(
            a,
            b,
            "depends_on",
            &serde_json::json!({}),
            0.7,
            None,
            "extract",
        )
        .unwrap();

    let t0 = Utc.with_ymd_and_hms(2026, 10, 23, 10, 0, 0).unwrap();
    let (_, full) = backup::create_full(
        &backups,
        &[("core", core.db()), ("agents/agent_x", agent.db())],
        t0,
    )
    .unwrap();
    assert_eq!(full.cuts.get("event"), Some(&50));

    // Changes after the full: new events, a mutated project row, a fact the agent learned.
    write_events(&core, p.id, 20, "b");
    core.commit(None, "user", |w| {
        projects::set_status(w.tx, p.id, projects::ProjectStatus::Paused)
    })
    .unwrap();
    agent
        .add(
            &NewFact::fact("learned after the full set", "chat"),
            Some(&[0.9, 0.1, 0.0, 0.0]),
        )
        .unwrap();
    let (_, incr) = backup::create_incremental(
        &backups,
        &backups.join(&full.id),
        core.db(),
        &[("agents/agent_x".into(), backup::Source::Open(agent.db()))],
        t0 + Duration::hours(1),
    )
    .unwrap();
    assert_eq!(incr.kind, BackupKind::Incremental);
    assert_eq!(incr.base.as_deref(), Some(full.id.as_str()));
    let live_hash = core.read(backup::content_hash).unwrap();
    assert_eq!(incr.dbs[0].tables, live_hash);

    // Restore the full into a fresh root.
    let r1 = root.join("restore-full/db");
    backup::restore(&backups, &full.id, &r1).unwrap();
    let c1 = CoreStore::open(&r1.join("core.db"), &root.join("restore-full/backups")).unwrap();
    assert_eq!(c1.read(events::last_seq).unwrap(), 50);
    let a1 = AgentStore::open(
        &r1.join("agents/agent_x.db"),
        4,
        "m",
        &root.join("restore-full/backups"),
    )
    .unwrap();
    assert_eq!(
        a1.knn(&[1.0, 0.0, 0.0, 0.0], 1).unwrap().len(),
        1,
        "vectors are restored"
    );
    let reached = a1
        .expand(&[a], &serde_json::json!({}), &Default::default())
        .unwrap();
    assert_eq!(reached.reached.len(), 1, "graph tables are restored");
    assert_eq!(a1.entities_in_text("the api", 3).unwrap().len(), 1);

    // Restore the incremental: base + delta, content hash equals the live database.
    let r2 = root.join("restore-incr/db");
    backup::restore(&backups, &incr.id, &r2).unwrap();
    let c2 = CoreStore::open(&r2.join("core.db"), &root.join("restore-incr/backups")).unwrap();
    assert_eq!(c2.read(events::last_seq).unwrap(), 70);
    let h2 = c2.read(backup::content_hash).unwrap();
    for (t, v) in &live_hash {
        assert_eq!(h2.get(t), Some(v), "table {t}");
    }
    assert_eq!(h2, live_hash);
    // Audit H3 (2026-10-31): the agent's memory is the incremental set's own, not the base's of up to a day before.
    let a2 = AgentStore::open(
        &r2.join("agents/agent_x.db"),
        4,
        "m",
        &root.join("restore-incr/backups"),
    )
    .unwrap();
    assert_eq!(
        a2.knn(&[1.0, 0.0, 0.0, 0.0], 5).unwrap().len(),
        2,
        "the fact learned after the full set is restored"
    );
    assert_eq!(
        c2.read(|c| projects::get(c, p.id)).unwrap().status,
        projects::ProjectStatus::Paused
    );
    // The restored log keeps working (seq continues gap-free).
    write_events(&c2, p.id, 1, "c");
    assert_eq!(c2.read(events::last_seq).unwrap(), 71);

    // Restoring onto existing files is refused; a tampered file is detected.
    assert!(matches!(
        backup::restore(&backups, &full.id, &r1),
        Err(StoreError::Conflict(_))
    ));
    let f = backups.join(&full.id).join(&full.dbs[0].file);
    let mut bytes = std::fs::read(&f).unwrap();
    let mid = bytes.len() / 2;
    bytes[mid] ^= 0xff;
    std::fs::write(&f, bytes).unwrap();
    assert!(matches!(
        backup::restore(&backups, &full.id, &root.join("r3/db")),
        Err(StoreError::Corrupt(_))
    ));
}

#[test]
fn retention_keeps_24_hourly_7_daily_4_weekly_and_bases() {
    let tmp = tempfile::tempdir().unwrap();
    let backups = tmp.path().join("backups");
    let core = CoreStore::open(&tmp.path().join("db/core.db"), &backups).unwrap();
    // 40 days of daily fulls at 00:00, plus hourly incrementals for the last 30 hours.
    let start = Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap();
    let mut last_full = None;
    for d in 0..40 {
        let (dir, _) =
            backup::create_full(&backups, &[("core", core.db())], start + Duration::days(d))
                .unwrap();
        last_full = Some(dir);
    }
    let base = last_full.unwrap();
    let last_day = start + Duration::days(39);
    for h in 1..=30 {
        backup::create_incremental(
            &backups,
            &base,
            core.db(),
            &[],
            last_day + Duration::hours(h),
        )
        .unwrap();
    }
    let removed = backup::prune(&backups, Retention::default()).unwrap();
    assert!(!removed.is_empty());
    let kept = backup::list_sets(&backups).unwrap();
    let incr = kept
        .iter()
        .filter(|m| m.kind == BackupKind::Incremental)
        .count();
    assert_eq!(incr, 24, "24 newest hours");
    assert!(
        kept.iter().any(|m| base.ends_with(&m.id)),
        "the base of kept incrementals survives"
    );
    let fulls = kept.iter().filter(|m| m.kind == BackupKind::Full).count();
    // 7 daily (days 33..=39) + the newest of up to 4 ISO weeks; some overlap with the dailies.
    assert!((7..=11).contains(&fulls), "fulls kept: {fulls}");
    // Idempotent.
    assert!(backup::prune(&backups, Retention::default())
        .unwrap()
        .is_empty());
}

/// Run-log compaction (ADR-76) deletes rows: an incremental made after it restores without them. It carried only the
/// rows past the full's cut, so the restore kept the deleted rows and failed its content-hash check.
#[test]
fn an_incremental_after_run_log_compaction_restores() {
    use agenium_core::{
        now_iso, AgentId, AgentStatus, BackendKind, Effort, ModelBinding, RoleKind, RunId, TeamId,
    };
    use agenium_store::core::{agents, runs};

    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let backups = root.join("backups");
    let core = CoreStore::open(&root.join("db/core.db"), &backups).unwrap();
    let team = agents::Team {
        id: TeamId::new(),
        name: "solo".into(),
        template: "coding@1".into(),
        project: None,
        lead: None,
        solo: true,
        created_at: now_iso(),
    };
    let agent = agents::Agent {
        id: AgentId::new(),
        name: "ada".into(),
        team: Some(team.id),
        role: "lead".into(),
        role_kind: RoleKind::Lead,
        tags: vec![],
        status: AgentStatus::Idle,
        backend: BackendKind::Api,
        model: ModelBinding {
            provider: "mock".into(),
            model: "m".into(),
            effort: Effort::HIGH,
            max_tokens: None,
        },
        mcp: serde_json::json!([]),
        skills: vec![],
        created_at: now_iso(),
    };
    let run = RunId::new();
    core.commit(None, "system", |w| -> Result<(), StoreError> {
        agents::insert_team(w.tx, &team)?;
        agents::insert_agent(w.tx, &agent)?;
        runs::insert(
            w.tx,
            &runs::Run {
                id: run,
                ticket: None,
                agent: agent.id,
                provider: "mock".into(),
                model: "m".into(),
                started_at: "2026-01-01T00:00:00.000Z".into(),
                ended_at: Some("2026-01-01T00:01:00.000Z".into()),
                outcome: Some("Completed".into()),
                steps: 3,
                usage: Default::default(),
                heartbeat: None,
            },
        )?;
        for i in 0..5 {
            runs::log(w.tx, run, "llm", &serde_json::json!({ "call": i }))?;
        }
        Ok(())
    })
    .unwrap();
    let t0 = Utc.with_ymd_and_hms(2026, 10, 23, 10, 0, 0).unwrap();
    let (_, full) = backup::create_full(&backups, &[("core", core.db())], t0).unwrap();

    let (compacted, removed) = core
        .commit(None, "system", |w| {
            runs::compact_logs(w.tx, "2026-06-01T00:00:00Z")
        })
        .unwrap()
        .value;
    assert_eq!((compacted, removed), (1, 5));
    let (_, incr) = backup::create_incremental(
        &backups,
        &backups.join(&full.id),
        core.db(),
        &[],
        t0 + Duration::hours(1),
    )
    .unwrap();
    let live = core.read(backup::content_hash).unwrap();
    assert_eq!(incr.dbs[0].tables, live);

    let r = root.join("restore/db");
    backup::restore(&backups, &incr.id, &r).unwrap();
    let c = CoreStore::open(&r.join("core.db"), &root.join("restore/backups")).unwrap();
    assert_eq!(c.read(backup::content_hash).unwrap(), live);
    let kinds: Vec<String> = c
        .read(|c| runs::log_entries(c, run))
        .unwrap()
        .into_iter()
        .map(|e| e.kind)
        .collect();
    assert_eq!(kinds, ["compacted"]);
}

/// Audit M8 (2026-10-31): an incremental set taken after `core` was migrated cannot be applied to its base's image;
/// a restore says so plainly (maintenance takes a full set instead as soon as the schema moved).
#[test]
fn an_incremental_set_across_a_migration_is_refused_plainly() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let backups = root.join("backups");
    let core = CoreStore::open(&root.join("db/core.db"), &backups).unwrap();
    let t0 = Utc.with_ymd_and_hms(2026, 10, 23, 10, 0, 0).unwrap();
    let (_, full) = backup::create_full(&backups, &[("core", core.db())], t0).unwrap();
    let v = backup::schema_version_of(core.db()).unwrap();
    core.commit(None, "system", |w| -> Result<(), StoreError> {
        w.tx.pragma_update(None, "user_version", v + 1)?;
        Ok(())
    })
    .unwrap();
    assert_eq!(backup::schema_version_of(core.db()).unwrap(), v + 1);
    let (_, incr) = backup::create_incremental(
        &backups,
        &backups.join(&full.id),
        core.db(),
        &[],
        t0 + Duration::hours(1),
    )
    .unwrap();
    let e = backup::restore(&backups, &incr.id, &root.join("r/db")).unwrap_err();
    assert!(
        matches!(e, StoreError::Invalid(ref m) if m.contains("cannot be applied")),
        "{e}"
    );
}

/// Audit L6 (2026-10-31): a set half-written (its manifest is written before the folder is renamed) was listed under its
/// final id, whose folder did not exist: prune failed and maintenance stopped there.
#[test]
fn a_half_written_set_is_not_a_set() {
    let tmp = tempfile::tempdir().unwrap();
    let backups = tmp.path().join("backups");
    let core = CoreStore::open(&tmp.path().join("db/core.db"), &backups).unwrap();
    let t0 = Utc.with_ymd_and_hms(2026, 10, 23, 10, 0, 0).unwrap();
    let (dir, full) = backup::create_full(&backups, &[("core", core.db())], t0).unwrap();
    let partial = backups.join(format!(".{}.partial", "20261024T100000Z-full"));
    std::fs::create_dir_all(&partial).unwrap();
    let mut m = full.clone();
    m.id = "20261024T100000Z-full".into();
    std::fs::write(
        partial.join("manifest.json"),
        serde_json::to_vec(&m).unwrap(),
    )
    .unwrap();
    let sets = backup::list_sets(&backups).unwrap();
    assert_eq!(sets.len(), 1, "only the finished set");
    assert_eq!(sets[0].id, full.id);
    backup::prune(&backups, Retention::default()).unwrap();
    assert!(dir.exists());
}

/// Audit L7 (2026-10-31): pre-migrate snapshots were ordered by name, `v10` before `v8`, so the newest was deleted.
#[test]
fn pre_migrate_snapshots_keep_the_newest_versions() {
    let tmp = tempfile::tempdir().unwrap();
    let c = rusqlite::Connection::open(tmp.path().join("x.db")).unwrap();
    c.execute_batch("CREATE TABLE t(a); INSERT INTO t VALUES (1);")
        .unwrap();
    for v in [8, 9, 10, 11] {
        backup::pre_migrate_snapshot(&c, tmp.path(), "agent", v).unwrap();
    }
    let mut kept: Vec<String> = std::fs::read_dir(tmp.path().join("pre-migrate"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    kept.sort();
    assert_eq!(kept.len(), 3, "{kept:?}");
    assert!(!kept.iter().any(|n| n.starts_with("agent-v8-")), "{kept:?}");
    for v in [9, 10, 11] {
        assert!(
            kept.iter().any(|n| n.starts_with(&format!("agent-v{v}-"))),
            "{kept:?}"
        );
    }
}
