mod common;

use agenium_orchestrator::maintenance::{restore_into_new_root, BackupRequest};
use agenium_store::backup::BackupKind;
use agenium_store::NewFact;
use chrono::{Duration, TimeZone, Utc};
use common::*;

#[test]
fn scheduled_backups_and_restore_into_a_new_root() {
    let f = fixture();
    let a = solo_with_plan(&f, &[("PI-1", &[])]);
    started(&f, a.id, "PI-1", "first", &[], &[]);
    let mem = f.app.agent_store(a.id, 4, "toy").unwrap();
    mem.add(
        &NewFact::fact("The project greets in English", "ticket:AGN-1"),
        Some(&[1.0, 0.0, 0.0, 0.0]),
    )
    .unwrap();

    let t0 = Utc.with_ymd_and_hms(2026, 10, 23, 8, 0, 0).unwrap();
    let r = f.app.maintenance_tick(t0).unwrap();
    assert!(
        r.agent_stores >= 1,
        "the tick also maintains the open agent memory databases"
    );
    let full = r.backup.expect("first tick takes a full backup");
    assert!(full.ends_with("-full"));
    assert!(
        f.app
            .maintenance_tick(t0 + Duration::minutes(30))
            .unwrap()
            .backup
            .is_none(),
        "nothing due"
    );
    // More work, then an hourly incremental.
    started(&f, a.id, "PI-1", "second", &[], &[]);
    let r = f.app.maintenance_tick(t0 + Duration::minutes(70)).unwrap();
    let incr = r.backup.unwrap();
    assert!(incr.ends_with("-incr"));
    // A day later: full again.
    let r = f.app.maintenance_tick(t0 + Duration::hours(25)).unwrap();
    assert!(r.backup.unwrap().ends_with("-full"));
    let sets = agenium_store::backup::list_sets(&f.app.backups_dir()).unwrap();
    assert_eq!(
        sets.iter().filter(|m| m.kind == BackupKind::Full).count(),
        2
    );
    assert!(
        sets[0]
            .dbs
            .iter()
            .any(|d| d.name == format!("agents/{}", a.id)),
        "agent memory is backed up"
    );

    // Restore the incremental into a brand-new root and open the app there.
    let new_root = f.root.parent().unwrap().join("restored");
    restore_into_new_root(&f.root, &f.app.backups_dir(), &incr, &new_root).unwrap();
    let app2 = open_app(&new_root, f.keyring.clone(), None);
    let ts = app2.tickets(f.project.id).unwrap();
    assert_eq!(
        ts.iter().map(|t| t.key.as_str()).collect::<Vec<_>>(),
        vec!["AGN-1", "AGN-2"]
    );
    let mem2 = app2.agent_store(a.id, 4, "toy").unwrap();
    assert_eq!(mem2.count_live().unwrap(), 1);
    assert!(
        restore_into_new_root(&f.root, &f.app.backups_dir(), &incr, &new_root).is_err(),
        "never overwrite"
    );
    let _ = f.app.backup(BackupRequest::Full, Utc::now()).unwrap();
}
