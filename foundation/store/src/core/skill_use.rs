//! The skills the runs of each closed ticket loaded (ADR-152), for a skill's effectiveness: the tickets it was loaded
//! in, how many were reopened, against the same agents' other tickets. Read from the run log (`load_skill` calls that
//! succeeded), so it covers the days the log keeps.

use std::collections::{BTreeSet, HashMap};

use rusqlite::Connection;

use crate::StoreError;

/// A closed ticket, who worked it, whether it was reopened, and the skills its runs loaded.
#[derive(Debug, Clone, PartialEq)]
pub struct ClosedTicket {
    pub ticket: String,
    pub assignee: Option<String>,
    pub reopened: bool,
    pub skills: BTreeSet<String>,
}

pub fn closed_tickets(c: &Connection) -> Result<Vec<ClosedTicket>, StoreError> {
    let mut loads: HashMap<String, BTreeSet<String>> = HashMap::new();
    let mut st = c.prepare_cached(
        "SELECT DISTINCT r.ticket, json_extract(l.data, '$.args.name')
         FROM run_log l JOIN run r ON r.id = l.run
         WHERE l.kind = 'tool' AND r.ticket IS NOT NULL
           AND json_extract(l.data, '$.tool') = 'load_skill'
           AND COALESCE(json_extract(l.data, '$.ok'), 1) = 1",
    )?;
    let rows = st.query_map([], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
    })?;
    for row in rows {
        if let (t, Some(skill)) = row? {
            loads.entry(t).or_default().insert(skill);
        }
    }
    let mut st = c.prepare_cached(
        "SELECT id, assignee, reopen_count FROM ticket WHERE status = 'Closed' ORDER BY id",
    )?;
    let rows = st.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, Option<String>>(1)?,
            r.get::<_, i64>(2)?,
        ))
    })?;
    rows.map(|row| {
        let (ticket, assignee, reopens) = row?;
        let skills = loads.remove(&ticket).unwrap_or_default();
        Ok(ClosedTicket {
            ticket,
            assignee,
            reopened: reopens > 0,
            skills,
        })
    })
    .collect()
}
