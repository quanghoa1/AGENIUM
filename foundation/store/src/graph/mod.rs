//! Graph memory in the agent database (ADR-51, M15-7/9): entities with identity scope and aliases, typed scoped
//! edges stored once and traversed in both directions through the partial indexes `edge_out` / `edge_in`, facts
//! linked to entities. Traversal is a two-arm `WITH RECURSIVE` with a path cycle guard, a scope guard per edge, a
//! depth limit, a hub cap (nodes with more live edges than `hub_degree` are not expanded) and a time budget
//! (`progress_handler`). Nothing is deleted: edges get `valid_to`, merged entities `merged_into`.

use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, Instant};

use rusqlite::{params, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::agent::{usable, vec_blob, AgentStore, Fact};
use crate::StoreError;

mod scope;
mod types;

pub use scope::*;
pub use types::*;
mod edges;
mod entities;
mod expand;

fn now() -> &'static str {
    "strftime('%Y-%m-%dT%H:%M:%fZ','now')"
}

fn refresh_fts(tx: &Transaction<'_>, id: i64) -> Result<(), StoreError> {
    let row: Option<(String, Option<String>)> = tx
        .query_row(
            "SELECT name, description FROM entity WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    tx.execute("DELETE FROM entity_fts WHERE rowid = ?1", [id])?;
    if let Some((name, desc)) = row {
        let mut st = tx.prepare_cached("SELECT alias FROM entity_alias WHERE entity_id = ?1")?;
        let aliases: Vec<String> = st
            .query_map([id], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        tx.execute(
            "INSERT INTO entity_fts(rowid, name, aliases, description) VALUES (?1, ?2, ?3, ?4)",
            params![id, name, aliases.join(" | "), desc.unwrap_or_default()],
        )?;
    }
    Ok(())
}

fn add_alias(tx: &Transaction<'_>, id: i64, alias: &str, source: &str) -> Result<bool, StoreError> {
    let norm = canonical(alias);
    if norm.is_empty() {
        return Ok(false);
    }
    Ok(tx.execute(
        "INSERT OR IGNORE INTO entity_alias(entity_id, alias_norm, alias, source) VALUES (?1, ?2, ?3, ?4)",
        params![id, norm, alias.trim(), source],
    )? > 0)
}

fn bump_degree(tx: &Transaction<'_>, ids: &[i64], by: i64) -> Result<(), StoreError> {
    for id in ids {
        tx.execute(
            "UPDATE entity SET degree = MAX(0, degree + ?2) WHERE id = ?1",
            params![id, by],
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn normalisation() {
        assert_eq!(
            canonical("  TP-Link  Archer C7 (v2) "),
            "tp link archer c7 v2"
        );
        assert_eq!(
            scope_key(&json!({"Vendor": "TP-Link", "version": 2, "x": ""})),
            "vendor=tp link;version=2"
        );
    }

    #[test]
    fn scope_matrix() {
        let ctx = normalize_scope(&json!({"vendor": "netgear", "project": "net-ops"}));
        let s = |v: Value| scope_match(&normalize_scope(&v), &ctx);
        assert_eq!(s(json!({"vendor": "tp-link"})), ScopeMatch::Conflict);
        assert!(matches!(s(json!({"vendor": "Netgear"})), ScopeMatch::Factor(f) if f > 1.0));
        assert!(
            matches!(s(json!({"os": "linux"})), ScopeMatch::Factor(f) if (f - 0.6).abs() < 1e-9)
        );
        assert_eq!(s(json!({})), ScopeMatch::Factor(1.0));
    }
}
