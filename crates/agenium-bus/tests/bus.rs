use std::sync::Arc;

use agenium_bus::{Bus, EventSource};
use agenium_core::{DomainEvent, EventRecord, ProjectId};
use agenium_store::core::events;
use agenium_store::{CoreStore, StoreError};

struct Log(Arc<CoreStore>);
impl EventSource for Log {
    fn since(&self, after: u64, limit: usize) -> Result<Vec<EventRecord>, String> {
        self.0
            .read(|c| events::since(c, after, limit))
            .map_err(|e| e.to_string())
    }
}

fn store(dir: &std::path::Path) -> Arc<CoreStore> {
    Arc::new(CoreStore::open(&dir.join("core.db"), &dir.join("backups")).unwrap())
}

fn emit(s: &CoreStore, n: usize) {
    let p = ProjectId::new();
    for i in 0..n {
        s.commit(None, "system", |w| -> Result<(), StoreError> {
            w.emit(DomainEvent::ProjectStatusChanged {
                project: p,
                status: i.to_string(),
            })?;
            Ok(())
        })
        .unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn late_subscriber_backfills_then_follows_live() {
    let tmp = tempfile::tempdir().unwrap();
    let s = store(tmp.path());
    let bus = Bus::new(64);
    s.set_sink(Some(Arc::new(bus.clone())));
    emit(&s, 10);
    let mut sub = bus.subscribe(Arc::new(Log(s.clone())), 0);
    let writer = {
        let s = s.clone();
        tokio::task::spawn_blocking(move || emit(&s, 10))
    };
    for want in 1..=20u64 {
        assert_eq!(sub.recv().await.unwrap().seq, want);
    }
    writer.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn burst_with_tiny_channel_is_exactly_once_in_order() {
    let tmp = tempfile::tempdir().unwrap();
    let s = store(tmp.path());
    let bus = Bus::new(2);
    s.set_sink(Some(Arc::new(bus.clone())));
    let mut subs: Vec<_> = (0..3)
        .map(|_| bus.subscribe(Arc::new(Log(s.clone())), 0))
        .collect();
    let writer = {
        let s = s.clone();
        tokio::task::spawn_blocking(move || emit(&s, 300))
    };
    for sub in &mut subs {
        let mut got = Vec::new();
        while got.len() < 300 {
            got.push(sub.recv().await.unwrap().seq);
        }
        assert_eq!(got, (1..=300).collect::<Vec<u64>>());
    }
    writer.await.unwrap();
}

#[tokio::test]
async fn restart_resumes_from_persisted_cursor() {
    let tmp = tempfile::tempdir().unwrap();
    let cursor;
    {
        let s = store(tmp.path());
        let bus = Bus::default();
        s.set_sink(Some(Arc::new(bus.clone())));
        let mut sub = bus.subscribe(Arc::new(Log(s.clone())), 0);
        emit(&s, 5);
        for _ in 0..3 {
            sub.recv().await.unwrap();
        }
        cursor = sub.last_seq();
        emit(&s, 5); // committed while the "UI" is gone
    }
    let s = store(tmp.path());
    let bus = Bus::default();
    s.set_sink(Some(Arc::new(bus.clone())));
    let mut sub = bus.subscribe(Arc::new(Log(s.clone())), cursor);
    let caught: Vec<u64> = sub.catch_up().unwrap().iter().map(|e| e.seq).collect();
    assert_eq!(caught, (4..=10).collect::<Vec<u64>>());
    assert!(sub.catch_up().unwrap().is_empty());
}
