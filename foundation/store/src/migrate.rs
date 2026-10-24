//! Schema migrations: version in `PRAGMA user_version`, each migration in one transaction (SQLite DDL is
//! transactional, so a failed migration rolls back by itself). A pre-migrate backup is still taken first when the
//! database already holds data (decision 23 / ADR-46). A database newer than this build is refused.
//! Foreign keys are off while migrations run (SQLite's table-rebuild procedure: a migration may recreate a table
//! that others reference) and every migration must pass `PRAGMA foreign_key_check` before it commits.

use rusqlite::{Connection, TransactionBehavior};

use crate::StoreError;

#[derive(Debug, Clone, Copy)]
pub struct Migration {
    pub version: u32,
    pub sql: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MigrateReport {
    pub from: u32,
    pub to: u32,
}

pub fn user_version(c: &Connection) -> Result<u32, StoreError> {
    let v: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    Ok(v as u32)
}

/// Applies the pending `migrations` (sorted by version, starting at 1). `pre_backup(from)` is called once before
/// the first pending migration when `from > 0`.
pub fn migrate(
    c: &mut Connection,
    migrations: &[Migration],
    pre_backup: impl FnOnce(&Connection, u32) -> Result<(), StoreError>,
) -> Result<MigrateReport, StoreError> {
    debug_assert!(migrations
        .windows(2)
        .all(|w| w[1].version == w[0].version + 1));
    let latest = migrations.last().map_or(0, |m| m.version);
    let from = user_version(c)?;
    if from > latest {
        return Err(StoreError::TooNew {
            found: from,
            supported: latest,
        });
    }
    if from == latest {
        return Ok(MigrateReport { from, to: from });
    }
    if from > 0 {
        pre_backup(c, from)?;
    }
    let fk: bool = c.query_row("PRAGMA foreign_keys", [], |r| r.get(0))?;
    c.pragma_update(None, "foreign_keys", "OFF")?;
    let result = apply(c, migrations, from);
    if fk {
        c.pragma_update(None, "foreign_keys", "ON")?;
    }
    result?;
    Ok(MigrateReport { from, to: latest })
}

fn apply(c: &mut Connection, migrations: &[Migration], from: u32) -> Result<(), StoreError> {
    for m in migrations.iter().filter(|m| m.version > from) {
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let failed = |message: String| StoreError::Migration {
            version: m.version,
            message,
        };
        tx.execute_batch(m.sql).map_err(|e| failed(e.to_string()))?;
        let broken: Vec<String> = {
            let mut st = tx.prepare("PRAGMA foreign_key_check")?;
            let rows = st.query_map([], |r| {
                Ok(format!(
                    "{} -> {}",
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(2)?
                ))
            })?;
            rows.collect::<Result<_, _>>()?
        };
        if !broken.is_empty() {
            return Err(failed(format!(
                "foreign keys broken: {}",
                broken.join(", ")
            )));
        }
        tx.pragma_update(None, "user_version", m.version as i64)?;
        tx.commit()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const M: &[Migration] = &[
        Migration {
            version: 1,
            sql: "CREATE TABLE a (x INTEGER) STRICT; INSERT INTO a VALUES (1);",
        },
        Migration {
            version: 2,
            sql: "ALTER TABLE a ADD COLUMN y TEXT;",
        },
    ];

    #[test]
    fn applies_in_order_and_is_idempotent() {
        let mut c = Connection::open_in_memory().unwrap();
        let mut backups = 0;
        assert_eq!(
            migrate(&mut c, &M[..1], |_, _| Ok(())).unwrap(),
            MigrateReport { from: 0, to: 1 }
        );
        assert_eq!(
            migrate(&mut c, M, |_, from| {
                assert_eq!(from, 1);
                backups += 1;
                Ok(())
            })
            .unwrap(),
            MigrateReport { from: 1, to: 2 }
        );
        assert_eq!(backups, 1);
        assert_eq!(
            migrate(&mut c, M, |_, _| panic!("no backup when up to date")).unwrap(),
            MigrateReport { from: 2, to: 2 }
        );
    }

    #[test]
    fn failed_migration_rolls_back_and_newer_db_is_refused() {
        let mut c = Connection::open_in_memory().unwrap();
        migrate(&mut c, &M[..1], |_, _| Ok(())).unwrap();
        let bad = [
            M[0],
            Migration {
                version: 2,
                sql: "ALTER TABLE a ADD COLUMN z TEXT; CREATE TABLE a (dup INTEGER);",
            },
        ];
        let e = migrate(&mut c, &bad, |_, _| Ok(())).unwrap_err();
        assert!(matches!(e, StoreError::Migration { version: 2, .. }), "{e}");
        assert_eq!(user_version(&c).unwrap(), 1);
        let cols: i64 = c
            .query_row("SELECT count(*) FROM pragma_table_info('a')", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(
            cols, 1,
            "partial DDL of the failed migration must be rolled back"
        );
        c.pragma_update(None, "user_version", 9).unwrap();
        assert!(matches!(
            migrate(&mut c, M, |_, _| Ok(())),
            Err(StoreError::TooNew {
                found: 9,
                supported: 2
            })
        ));
    }

    #[test]
    fn a_table_rebuild_keeps_references_and_broken_keys_are_refused() {
        let mut c = Connection::open_in_memory().unwrap();
        c.pragma_update(None, "foreign_keys", "ON").unwrap();
        let base = Migration {
            version: 1,
            sql: "CREATE TABLE p (id TEXT PRIMARY KEY, n TEXT NOT NULL) STRICT;
                  CREATE TABLE ch (p TEXT NOT NULL REFERENCES p(id)) STRICT;
                  INSERT INTO p VALUES ('a', 'x'); INSERT INTO ch VALUES ('a');",
        };
        migrate(&mut c, &[base], |_, _| Ok(())).unwrap();
        let rebuild = Migration {
            version: 2,
            sql: "CREATE TABLE p_new (id TEXT PRIMARY KEY, n TEXT) STRICT;
                  INSERT INTO p_new SELECT id, n FROM p; DROP TABLE p; ALTER TABLE p_new RENAME TO p;",
        };
        migrate(&mut c, &[base, rebuild], |_, _| Ok(())).unwrap();
        assert!(c
            .query_row("PRAGMA foreign_keys", [], |r| r.get::<_, bool>(0))
            .unwrap());
        assert!(
            c.execute("INSERT INTO ch VALUES ('missing')", []).is_err(),
            "the child still references the rebuilt table"
        );
        let breaking = Migration {
            version: 3,
            sql: "DELETE FROM p;",
        };
        let e = migrate(&mut c, &[base, rebuild, breaking], |_, _| Ok(())).unwrap_err();
        assert!(matches!(e, StoreError::Migration { version: 3, .. }), "{e}");
        assert_eq!(user_version(&c).unwrap(), 2);
        assert!(c
            .query_row("PRAGMA foreign_keys", [], |r| r.get::<_, bool>(0))
            .unwrap());
    }
}
