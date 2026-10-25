//! `core.db`: projects, teams, agents, specs/plans, tickets, comments, runs, grants and the event log.
//!
//! Every mutation goes through [`CoreStore::commit`]: one `BEGIN IMMEDIATE` transaction holding the state
//! changes, the events they produce (gap-free `seq`) and, for intents, the idempotency record. After commit the
//! new events are pushed to the [`EventSink`] (realtime bus); a replayed intent returns the stored result and
//! emits nothing.

pub mod agents;
pub mod comments;
pub mod events;
pub mod ops;
pub mod plans;
pub mod projects;
pub mod roles;
pub mod runs;
pub mod tickets;
pub mod workspaces;

use std::path::Path;
use std::sync::{Arc, RwLock};

use agenium_core::{now_iso, DomainEvent, EventRecord, EventSink};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::backup;
use crate::db::{Db, DbOptions};
use crate::migrate::{migrate, MigrateReport, Migration};
use crate::StoreError;

pub const CORE_MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        sql: include_str!("../schema/core_001.sql"),
    },
    Migration {
        version: 2,
        sql: include_str!("../schema/core_002.sql"),
    },
    Migration {
        version: 3,
        sql: include_str!("../schema/core_003.sql"),
    },
    Migration {
        version: 4,
        sql: include_str!("../schema/core_004.sql"),
    },
];

/// Write context handed to [`CoreStore::commit`] closures.
pub struct WriteCtx<'a, 't> {
    pub tx: &'a Transaction<'t>,
    actor: String,
    intent: Option<String>,
    events: Vec<EventRecord>,
}

impl WriteCtx<'_, '_> {
    pub fn actor(&self) -> &str {
        &self.actor
    }

    /// Appends an event in the current transaction and returns its record.
    pub fn emit(&mut self, e: DomainEvent) -> Result<EventRecord, StoreError> {
        let rec = events::append(self.tx, self.intent.as_deref(), &self.actor, e)?;
        self.events.push(rec.clone());
        Ok(rec)
    }

    /// Appends an event attributed to another actor (e.g. a System consequence of a user action).
    pub fn emit_as(&mut self, actor: &str, e: DomainEvent) -> Result<EventRecord, StoreError> {
        let rec = events::append(self.tx, self.intent.as_deref(), actor, e)?;
        self.events.push(rec.clone());
        Ok(rec)
    }
}

/// Result of a commit.
#[derive(Debug, Clone, PartialEq)]
pub struct Committed<T> {
    pub value: T,
    /// `true` when the intent had already been committed; nothing changed this time.
    pub replayed: bool,
    pub events: Vec<EventRecord>,
}

pub struct CoreStore {
    db: Db,
    sink: RwLock<Option<Arc<dyn EventSink>>>,
    migration: MigrateReport,
}

impl std::fmt::Debug for CoreStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CoreStore").field("db", &self.db).finish()
    }
}

impl CoreStore {
    /// Opens `core.db`, migrating it. A pre-migrate backup goes to `pre_migrate_dir` when the database already
    /// has a schema.
    pub fn open(path: &Path, pre_migrate_dir: &Path) -> Result<CoreStore, StoreError> {
        Self::open_with(path, pre_migrate_dir, DbOptions::default())
    }

    pub fn open_with(
        path: &Path,
        pre_migrate_dir: &Path,
        opts: DbOptions,
    ) -> Result<CoreStore, StoreError> {
        let mut report = MigrateReport { from: 0, to: 0 };
        let db = Db::open(path, opts, |c| {
            report = migrate(c, CORE_MIGRATIONS, |c, from| {
                backup::pre_migrate_snapshot(c, pre_migrate_dir, "core", from).map(|_| ())
            })?;
            Ok(())
        })?;
        Ok(CoreStore {
            db,
            sink: RwLock::new(None),
            migration: report,
        })
    }

    pub fn migration(&self) -> MigrateReport {
        self.migration
    }

    pub fn db(&self) -> &Db {
        &self.db
    }

    pub fn set_sink(&self, sink: Option<Arc<dyn EventSink>>) {
        *self.sink.write().unwrap_or_else(|p| p.into_inner()) = sink;
    }

    /// Read-only access on a pooled connection.
    pub fn read<T>(
        &self,
        f: impl FnOnce(&Connection) -> Result<T, StoreError>,
    ) -> Result<T, StoreError> {
        self.db.read(f)
    }

    /// Commits a mutation (see module docs). With `intent = Some(id)`, a second commit of the same id returns the
    /// first result without running `f`.
    pub fn commit<T, E>(
        &self,
        intent: Option<&str>,
        actor: &str,
        f: impl FnOnce(&mut WriteCtx<'_, '_>) -> Result<T, E>,
    ) -> Result<Committed<T>, E>
    where
        T: Serialize + DeserializeOwned,
        E: From<StoreError>,
    {
        let out = self.db.write(|tx| -> Result<Committed<T>, E> {
            if let Some(id) = intent {
                let prev: Option<String> = tx
                    .query_row("SELECT result FROM intent WHERE id = ?1", [id], |r| r.get(0))
                    .optional()
                    .map_err(StoreError::from)?;
                if let Some(json) = prev {
                    let value = serde_json::from_str(&json).map_err(StoreError::from)?;
                    return Ok(Committed { value, replayed: true, events: vec![] });
                }
            }
            let mut ctx = WriteCtx { tx, actor: actor.to_owned(), intent: intent.map(str::to_owned), events: vec![] };
            let value = f(&mut ctx)?;
            if let Some(id) = intent {
                let result = serde_json::to_string(&value).map_err(StoreError::from)?;
                let first = ctx.events.first().map(|e| e.seq as i64);
                tx.execute(
                    "INSERT INTO intent(id, actor, first_seq, result, at) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![id, actor, first, result, now_iso()],
                )
                .map_err(StoreError::from)?;
            }
            Ok(Committed { value, replayed: false, events: ctx.events })
        })?;
        if !out.events.is_empty() {
            if let Some(sink) = self.sink.read().unwrap_or_else(|p| p.into_inner()).clone() {
                for e in &out.events {
                    sink.emit(e);
                }
            }
        }
        Ok(out)
    }

    /// Stored result of an intent, if it was committed.
    pub fn intent_result(&self, id: &str) -> Result<Option<serde_json::Value>, StoreError> {
        self.read(|c| {
            let s: Option<String> = c
                .query_row("SELECT result FROM intent WHERE id = ?1", [id], |r| {
                    r.get(0)
                })
                .optional()?;
            Ok(s.map(|s| serde_json::from_str(&s)).transpose()?)
        })
    }
}

// ---------------------------------------------------------------- JSON column helpers

pub(crate) fn to_json<T: Serialize + ?Sized>(v: &T) -> Result<String, StoreError> {
    Ok(serde_json::to_string(v)?)
}

pub(crate) fn from_json<T: DeserializeOwned>(s: &str) -> Result<T, StoreError> {
    serde_json::from_str(s).map_err(|e| StoreError::Corrupt(format!("bad JSON column: {e}")))
}

pub(crate) fn parse<T: std::str::FromStr>(s: &str, what: &str) -> Result<T, StoreError>
where
    T::Err: std::fmt::Display,
{
    s.parse()
        .map_err(|e| StoreError::Corrupt(format!("bad {what} {s:?}: {e}")))
}
