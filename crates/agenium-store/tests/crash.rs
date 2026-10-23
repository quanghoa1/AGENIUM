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
