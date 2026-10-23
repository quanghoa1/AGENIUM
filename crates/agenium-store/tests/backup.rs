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

    let t0 = Utc.with_ymd_and_hms(2026, 10, 23, 10, 0, 0).unwrap();
    let (_, full) = backup::create_full(
        &backups,
        &[("core", core.db()), ("agents/agent_x", agent.db())],
        t0,
    )
    .unwrap();
    assert_eq!(full.cuts.get("event"), Some(&50));

    // Changes after the full: new events, a mutated project row.
    write_events(&core, p.id, 20, "b");
    core.commit(None, "user", |w| {
        projects::set_status(w.tx, p.id, projects::ProjectStatus::Paused)
    })
    .unwrap();
    let (_, incr) = backup::create_incremental(
        &backups,
        &backups.join(&full.id),
        core.db(),
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
        backup::create_incremental(&backups, &base, core.db(), last_day + Duration::hours(h))
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
