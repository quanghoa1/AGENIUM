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

/// Normalised name: lowercase, every run of non-alphanumeric characters becomes one space, trimmed. `+` and `#`
/// directly after a letter or digit stay part of the word: "C#", "C++" and "C" are three languages, and "DS920+" is
/// not "DS920" (they all became "c" / "ds920" before ADR-94; `AgentStore::open` re-keys older databases).
pub fn canonical(s: &str) -> String {
    let mut out = String::new();
    let mut gap = false;
    for c in s.trim().chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            if gap && !out.is_empty() {
                out.push(' ');
            }
            out.push(c);
            gap = false;
        } else if matches!(c, '+' | '#') && !gap && !out.is_empty() {
            out.push(c);
        } else {
            gap = true;
        }
    }
    out
}

/// Canonical scope object: keys and string values normalised, empty values dropped.
pub fn normalize_scope(scope: &Value) -> Value {
    let mut m = serde_json::Map::new();
    if let Some(o) = scope.as_object() {
        let mut keys: Vec<&String> = o.keys().collect();
        keys.sort();
        for k in keys {
            let v = match &o[k] {
                Value::String(s) => canonical(s),
                Value::Number(n) => n.to_string(),
                Value::Bool(b) => b.to_string(),
                _ => continue,
            };
            let key = canonical(k).replace(' ', "_");
            if !v.is_empty() && !key.is_empty() {
                m.insert(key, Value::String(v));
            }
        }
    }
    Value::Object(m)
}

/// `k=v;k=v` (sorted) of a normalised scope; the identity part of unique indexes.
pub fn scope_key(scope: &Value) -> String {
    normalize_scope(scope)
        .as_object()
        .map(|o| {
            o.iter()
                .map(|(k, v)| format!("{k}={}", v.as_str().unwrap_or("")))
                .collect::<Vec<_>>()
                .join(";")
        })
        .unwrap_or_default()
}

/// Scope test of ADR-51 §5 between an item's scope and the context scope (both normalised).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ScopeMatch {
    /// A key present on both sides differs: the item must not be used here.
    Conflict,
    /// Factor for the re-ranker: 1.0 per matching key, `0.6` per key the context does not know.
    Factor(f64),
}

pub fn scope_match(item: &Value, ctx: &Value) -> ScopeMatch {
    let (Some(i), c) = (item.as_object(), ctx.as_object()) else {
        return ScopeMatch::Factor(1.0);
    };
    let mut f = 1.0;
    for (k, v) in i {
        match c.and_then(|c| c.get(k)) {
            Some(cv) if cv == v => f *= 1.05,
            Some(_) => return ScopeMatch::Conflict,
            None => f *= 0.6,
        }
    }
    ScopeMatch::Factor(f)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entity {
    pub id: i64,
    pub kind: String,
    pub name: String,
    pub canonical_key: String,
    pub scope: Value,
    pub description: Option<String>,
    pub confidence: f64,
    pub mentions: i64,
    pub degree: i64,
    pub merged_into: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewEntity {
    pub kind: String,
    pub name: String,
    pub scope: Value,
    pub description: Option<String>,
    pub aliases: Vec<String>,
    pub confidence: f64,
    pub evidence: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Edge {
    pub id: i64,
    pub src: i64,
    pub dst: i64,
    pub kind: String,
    pub scope: Value,
    pub weight: f64,
    pub confidence: f64,
    pub evidence_fact: Option<i64>,
    pub valid_to: Option<String>,
}

/// An entity found by linking, with how it was found and a similarity in 0..1.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub entity: Entity,
    pub via: &'static str,
    pub similarity: f64,
}

/// A node reached by expansion.
#[derive(Debug, Clone, PartialEq)]
pub struct Reached {
    pub entity: i64,
    pub score: f64,
    pub hops: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct Expansion {
    pub max_depth: u32,
    pub hub_degree: i64,
    pub limit: usize,
    pub budget: Duration,
}

impl Default for Expansion {
    fn default() -> Self {
        Self {
            max_depth: 2,
            hub_degree: 200,
            limit: 50,
            budget: Duration::from_millis(25),
        }
    }
}

/// Result of an expansion: nodes plus whether the time budget cut it short.
#[derive(Debug, Clone, Default)]
pub struct ExpandResult {
    pub reached: Vec<Reached>,
    pub timed_out: bool,
}

const ENTITY_COLS: &str =
    "id, type, name, canonical_key, scope, description, confidence, mentions, degree, merged_into";

fn map_entity(r: &rusqlite::Row<'_>) -> rusqlite::Result<(Entity, String)> {
    Ok((
        Entity {
            id: r.get(0)?,
            kind: r.get(1)?,
            name: r.get(2)?,
            canonical_key: r.get(3)?,
            scope: Value::Null,
            description: r.get(5)?,
            confidence: r.get(6)?,
            mentions: r.get(7)?,
            degree: r.get(8)?,
            merged_into: r.get(9)?,
        },
        r.get(4)?,
    ))
}

fn decode_entity(v: (Entity, String)) -> Result<Entity, StoreError> {
    let (mut e, scope) = v;
    e.scope = crate::core::from_json(&scope)?;
    Ok(e)
}

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

impl AgentStore {
    pub fn entity(&self, id: i64) -> Result<Entity, StoreError> {
        self.db.read(|c| {
            c.query_row(
                &format!("SELECT {ENTITY_COLS} FROM entity WHERE id = ?1"),
                [id],
                map_entity,
            )
            .optional()?
            .map(decode_entity)
            .transpose()?
            .ok_or_else(|| StoreError::NotFound(format!("entity {id}")))
        })
    }

    /// Follows `merged_into` to the surviving entity.
    pub fn live_entity(&self, id: i64) -> Result<Entity, StoreError> {
        let mut e = self.entity(id)?;
        for _ in 0..32 {
            match e.merged_into {
                Some(n) => e = self.entity(n)?,
                None => return Ok(e),
            }
        }
        Err(StoreError::Corrupt(format!(
            "merge chain of entity {id} does not end"
        )))
    }

    /// Inserts an entity or, when one with the same type + canonical name + identity scope exists, records the new
    /// mention (aliases, `mentions`, `last_seen_at`). Returns `(id, created)`.
    pub fn upsert_entity(
        &self,
        e: &NewEntity,
        vector: Option<&[f32]>,
    ) -> Result<(i64, bool), StoreError> {
        if let Some(v) = vector {
            self.check_vec(v)?;
        }
        let key = canonical(&e.name);
        if key.is_empty() || e.kind.trim().is_empty() {
            return Err(StoreError::Invalid("entity needs a type and a name".into()));
        }
        let kind = canonical(&e.kind).replace(' ', "_");
        let scope = normalize_scope(&e.scope);
        let sk = scope_key(&scope);
        self.db.write(|tx| {
            let existing: Option<i64> = tx
                .query_row(
                    "SELECT id FROM entity WHERE type = ?1 AND canonical_key = ?2 AND scope_key = ?3 AND merged_into IS NULL",
                    params![kind, key, sk],
                    |r| r.get(0),
                )
                .optional()?;
            let (id, created) = match existing {
                Some(id) => {
                    tx.execute(
                        &format!(
                            "UPDATE entity SET mentions = mentions + 1, last_seen_at = {n}, updated_at = {n},
                             description = COALESCE(description, ?2) WHERE id = ?1",
                            n = now()
                        ),
                        params![id, e.description],
                    )?;
                    (id, false)
                }
                None => {
                    let evidence: Vec<&str> = e.evidence.iter().map(String::as_str).collect();
                    tx.execute(
                        &format!(
                            "INSERT INTO entity(type, name, canonical_key, scope, scope_key, description, confidence, evidence, last_seen_at)
                             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, {})",
                            now()
                        ),
                        params![
                            kind,
                            e.name.trim(),
                            key,
                            scope.to_string(),
                            sk,
                            e.description,
                            e.confidence.clamp(0.0, 1.0),
                            serde_json::to_string(&evidence).unwrap_or_else(|_| "[]".into())
                        ],
                    )?;
                    let id = tx.last_insert_rowid();
                    if let Some(v) = vector.filter(|v| usable(v)) {
                        tx.execute(
                            "INSERT INTO entity_vec(rowid, embedding) VALUES (?1, ?2)",
                            params![id, vec_blob(v)],
                        )?;
                    }
                    (id, true)
                }
            };
            add_alias(tx, id, &e.name, "extract")?;
            for a in &e.aliases {
                add_alias(tx, id, a, "extract")?;
            }
            refresh_fts(tx, id)?;
            Ok((id, created))
        })
    }

    /// Entity linking candidates for a mention of `kind` (ADR-51 §4): exact key, alias, FTS on name/aliases,
    /// vector KNN on name + description. Only live, non-merged entities of the same type.
    pub fn entity_candidates(
        &self,
        kind: &str,
        name: &str,
        vector: Option<&[f32]>,
        k: usize,
    ) -> Result<Vec<Candidate>, StoreError> {
        let kind = canonical(kind).replace(' ', "_");
        let key = canonical(name);
        let mut found: BTreeMap<i64, (&'static str, f64)> = BTreeMap::new();
        self.db.read(|c| -> Result<(), StoreError> {
            let mut st = c.prepare_cached(
                "SELECT id FROM entity WHERE type = ?1 AND canonical_key = ?2 AND merged_into IS NULL AND valid_to IS NULL",
            )?;
            for id in st.query_map(params![kind, key], |r| r.get::<_, i64>(0))? {
                found.insert(id?, ("exact", 1.0));
            }
            let mut st = c.prepare_cached(
                "SELECT e.id FROM entity_alias a JOIN entity e ON e.id = a.entity_id
                 WHERE a.alias_norm = ?1 AND e.type = ?2 AND e.merged_into IS NULL AND e.valid_to IS NULL",
            )?;
            for id in st.query_map(params![key, kind], |r| r.get::<_, i64>(0))? {
                found.entry(id?).or_insert(("alias", 0.99));
            }
            if let Some(q) = crate::agent::fts_query(name) {
                let mut st = c.prepare_cached(
                    "SELECT e.id FROM entity_fts JOIN entity e ON e.id = entity_fts.rowid
                     WHERE entity_fts MATCH ?1 AND e.type = ?2 AND e.merged_into IS NULL AND e.valid_to IS NULL
                     ORDER BY bm25(entity_fts) LIMIT ?3",
                )?;
                for id in st.query_map(params![q, kind, k as i64], |r| r.get::<_, i64>(0))? {
                    found.entry(id?).or_insert(("fts", 0.0));
                }
            }
            if let Some(v) = vector.filter(|v| usable(v)) {
                let mut st = c.prepare_cached(
                    "SELECT rowid, distance FROM entity_vec WHERE embedding MATCH ?1 AND k = ?2 ORDER BY distance",
                )?;
                let rows: Vec<(i64, Option<f64>)> = st
                    .query_map(params![vec_blob(v), (k * 4) as i64], |r| Ok((r.get(0)?, r.get(1)?)))?
                    .collect::<Result<_, _>>()?;
                for (id, d) in rows.into_iter().filter_map(|(id, d)| Some((id, d?))) {
                    let sim = 1.0 - d;
                    let e = found.entry(id).or_insert(("vector", sim));
                    if e.1 < sim {
                        e.1 = sim;
                    }
                }
            }
            Ok(())
        })?;
        let mut out = Vec::new();
        for (id, (via, sim)) in found {
            let e = self.entity(id)?;
            if e.kind == kind && e.merged_into.is_none() {
                out.push(Candidate {
                    entity: e,
                    via,
                    similarity: sim,
                });
            }
        }
        out.sort_by(|a, b| b.similarity.total_cmp(&a.similarity));
        out.truncate(k);
        Ok(out)
    }

    /// Entities whose name or alias occurs in `text` as whole words (query-side linking; longest names first).
    pub fn entities_in_text(&self, text: &str, limit: usize) -> Result<Vec<Entity>, StoreError> {
        let norm = format!(" {} ", canonical(text));
        let words: Vec<&str> = norm.split_whitespace().collect();
        if words.is_empty() {
            return Ok(vec![]);
        }
        // Candidate n-grams up to 5 words.
        let mut grams = Vec::new();
        for n in (1..=5).rev() {
            for w in words.windows(n) {
                grams.push(w.join(" "));
            }
        }
        grams.sort();
        grams.dedup();
        let ids: Vec<i64> = self.db.read(|c| -> Result<Vec<i64>, StoreError> {
            let mut st = c.prepare_cached(
                "SELECT DISTINCT e.id, length(a.alias_norm) FROM entity_alias a JOIN entity e ON e.id = a.entity_id
                 WHERE a.alias_norm = ?1 AND e.merged_into IS NULL AND e.valid_to IS NULL",
            )?;
            let mut hits: Vec<(i64, i64)> = Vec::new();
            for g in &grams {
                if g.chars().count() < 2 {
                    continue;
                }
                for r in st.query_map([g], |r| Ok((r.get(0)?, r.get(1)?)))? {
                    hits.push(r?);
                }
            }
            hits.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            let mut seen = std::collections::HashSet::new();
            Ok(hits.into_iter().filter(|(id, _)| seen.insert(*id)).map(|(id, _)| id).take(limit).collect())
        })?;
        ids.into_iter().map(|id| self.entity(id)).collect()
    }

    /// Adds a live edge or strengthens the identical live one (`weight`, `confidence` max). Returns its id.
    #[allow(clippy::too_many_arguments)]
    pub fn add_edge(
        &self,
        src: i64,
        dst: i64,
        kind: &str,
        scope: &Value,
        confidence: f64,
        evidence_fact: Option<i64>,
        source: &str,
    ) -> Result<i64, StoreError> {
        if src == dst {
            return Err(StoreError::Invalid(
                "an edge needs two different entities".into(),
            ));
        }
        let scope = normalize_scope(scope);
        let sk = scope_key(&scope);
        self.db.write(|tx| {
            let known: Option<i64> = tx
                .query_row("SELECT 1 FROM edge_type WHERE name = ?1", [kind], |r| r.get(0))
                .optional()?;
            if known.is_none() {
                return Err(StoreError::Invalid(format!("unknown edge type {kind:?}")));
            }
            let existing: Option<i64> = tx
                .query_row(
                    "SELECT id FROM edge WHERE src = ?1 AND dst = ?2 AND type = ?3 AND scope_key = ?4 AND valid_to IS NULL",
                    params![src, dst, kind, sk],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(id) = existing {
                tx.execute(
                    &format!(
                        "UPDATE edge SET weight = weight + 1, confidence = MAX(confidence, ?2),
                         evidence_fact = COALESCE(evidence_fact, ?3), updated_at = {} WHERE id = ?1",
                        now()
                    ),
                    params![id, confidence.clamp(0.0, 1.0), evidence_fact],
                )?;
                return Ok(id);
            }
            tx.execute(
                "INSERT INTO edge(src, dst, type, scope, scope_key, confidence, evidence_fact, source) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![src, dst, kind, scope.to_string(), sk, confidence.clamp(0.0, 1.0), evidence_fact, source],
            )?;
            let id = tx.last_insert_rowid();
            bump_degree(tx, &[src, dst], 1)?;
            Ok(id)
        })
    }

    /// Ends an edge (contradicted or replaced); it stays for audit.
    pub fn invalidate_edge(&self, id: i64, superseded_by: Option<i64>) -> Result<(), StoreError> {
        self.db.write(|tx| {
            let ends: Option<(i64, i64)> = tx
                .query_row("SELECT src, dst FROM edge WHERE id = ?1 AND valid_to IS NULL", [id], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })
                .optional()?;
            let Some((s, d)) = ends else {
                return Ok(());
            };
            tx.execute(
                &format!("UPDATE edge SET valid_to = {n}, superseded_by = ?2, updated_at = {n} WHERE id = ?1", n = now()),
                params![id, superseded_by],
            )?;
            bump_degree(tx, &[s, d], -1)?;
            Ok(())
        })
    }

    pub fn edges_of(&self, entity: i64) -> Result<Vec<Edge>, StoreError> {
        self.db.read(|c| {
            let mut st = c.prepare_cached(
                "SELECT id, src, dst, type, scope, weight, confidence, evidence_fact, valid_to FROM edge
                 WHERE src = ?1 OR dst = ?1 ORDER BY id",
            )?;
            let rows = st.query_map([entity], |r| {
                Ok((
                    Edge {
                        id: r.get(0)?,
                        src: r.get(1)?,
                        dst: r.get(2)?,
                        kind: r.get(3)?,
                        scope: Value::Null,
                        weight: r.get(5)?,
                        confidence: r.get(6)?,
                        evidence_fact: r.get(7)?,
                        valid_to: r.get(8)?,
                    },
                    r.get::<_, String>(4)?,
                ))
            })?;
            rows.map(|r| {
                let (mut e, s) = r?;
                e.scope = crate::core::from_json(&s)?;
                Ok(e)
            })
            .collect()
        })
    }

    pub fn link_fact(&self, fact: i64, entity: i64, role: &str) -> Result<(), StoreError> {
        let role = match role {
            "subject" | "object" | "scope" => role,
            _ => "mention",
        };
        self.db.write(|tx| {
            tx.execute(
                "INSERT OR IGNORE INTO fact_entity(fact_id, entity_id, role) VALUES (?1, ?2, ?3)",
                params![fact, entity, role],
            )?;
            Ok(())
        })
    }

    /// Entities a fact is about.
    pub fn fact_entities(&self, fact: i64) -> Result<Vec<i64>, StoreError> {
        self.db.read(|c| {
            let mut st = c.prepare_cached(
                "SELECT DISTINCT entity_id FROM fact_entity WHERE fact_id = ?1 ORDER BY entity_id",
            )?;
            let rows = st.query_map([fact], |r| r.get(0))?;
            rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
        })
    }

    /// Merges `from` into `into` (entity resolution): edges and fact links are rewired (an edge that becomes a
    /// duplicate or a self-loop is ended), aliases move, `from` becomes a tombstone. Logged for undo.
    pub fn merge_entities(
        &self,
        from: i64,
        into: i64,
        similarity: Option<f64>,
        decided_by: &str,
    ) -> Result<(), StoreError> {
        if from == into {
            return Err(StoreError::Invalid(
                "cannot merge an entity into itself".into(),
            ));
        }
        let (a, b) = (self.entity(from)?, self.entity(into)?);
        if a.kind != b.kind {
            return Err(StoreError::Invalid(format!(
                "will not merge a {} into a {}",
                a.kind, b.kind
            )));
        }
        if a.merged_into.is_some() || b.merged_into.is_some() {
            return Err(StoreError::Invalid("entity already merged".into()));
        }
        if matches!(scope_match(&a.scope, &b.scope), ScopeMatch::Conflict) {
            return Err(StoreError::Invalid(
                "identity scopes conflict; not the same entity".into(),
            ));
        }
        self.db.write(|tx| {
            let mut moved_edges = Vec::new();
            let mut ended = Vec::new();
            let mut st = tx.prepare("SELECT id, src, dst, type, scope_key FROM edge WHERE (src = ?1 OR dst = ?1) AND valid_to IS NULL")?;
            let edges: Vec<(i64, i64, i64, String, String)> = st
                .query_map([from], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?
                .collect::<Result<_, _>>()?;
            drop(st);
            for (id, s, d, t, sk) in edges {
                let (ns, nd) = (if s == from { into } else { s }, if d == from { into } else { d });
                let dup: Option<i64> = tx
                    .query_row(
                        "SELECT id FROM edge WHERE src = ?1 AND dst = ?2 AND type = ?3 AND scope_key = ?4 AND valid_to IS NULL AND id <> ?5",
                        params![ns, nd, t, sk, id],
                        |r| r.get(0),
                    )
                    .optional()?;
                if ns == nd || dup.is_some() {
                    tx.execute(
                        &format!("UPDATE edge SET valid_to = {n}, superseded_by = ?2, updated_at = {n} WHERE id = ?1", n = now()),
                        params![id, dup],
                    )?;
                    if let Some(k) = dup {
                        tx.execute("UPDATE edge SET weight = weight + 1 WHERE id = ?1", [k])?;
                    }
                    bump_degree(tx, &[s, d], -1)?;
                    ended.push(id);
                } else {
                    tx.execute("UPDATE edge SET src = ?2, dst = ?3 WHERE id = ?1", params![id, ns, nd])?;
                    bump_degree(tx, &[from], -1)?;
                    bump_degree(tx, &[into], 1)?;
                    moved_edges.push(id);
                }
            }
            let links = tx.execute(
                "INSERT OR IGNORE INTO fact_entity(fact_id, entity_id, role, confidence)
                 SELECT fact_id, ?2, role, confidence FROM fact_entity WHERE entity_id = ?1",
                params![from, into],
            )?;
            tx.execute(
                "INSERT OR IGNORE INTO entity_alias(entity_id, alias_norm, alias, source)
                 SELECT ?2, alias_norm, alias, 'merge' FROM entity_alias WHERE entity_id = ?1",
                params![from, into],
            )?;
            tx.execute(
                &format!(
                    "UPDATE entity SET mentions = mentions + (SELECT mentions FROM entity WHERE id = ?1), updated_at = {} WHERE id = ?2",
                    now()
                ),
                params![from, into],
            )?;
            tx.execute(
                &format!("UPDATE entity SET merged_into = ?2, updated_at = {} WHERE id = ?1", now()),
                params![from, into],
            )?;
            tx.execute("DELETE FROM entity_vec WHERE rowid = ?1", [from])?;
            tx.execute("DELETE FROM entity_fts WHERE rowid = ?1", [from])?;
            refresh_fts(tx, into)?;
            let moved = serde_json::json!({ "edges": moved_edges, "ended_edges": ended, "fact_links": links });
            tx.execute(
                "INSERT INTO entity_merge_log(from_id, into_id, similarity, decided_by, moved) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![from, into, similarity, decided_by, moved.to_string()],
            )?;
            Ok(())
        })
    }

    /// Bounded two-arm expansion from `seeds` (ADR-51 §6a): returns reached nodes (not the seeds) with the best
    /// hop-decayed score. Edges whose scope conflicts with `ctx` are not crossed; hubs are not expanded; the query
    /// is interrupted after `x.budget` (whatever the vector/keyword channels found is still used).
    pub fn expand(
        &self,
        seeds: &[i64],
        ctx: &Value,
        x: &Expansion,
    ) -> Result<ExpandResult, StoreError> {
        if seeds.is_empty() || x.max_depth == 0 {
            return Ok(ExpandResult::default());
        }
        let ctx = normalize_scope(ctx).to_string();
        let seeds_json = serde_json::to_string(seeds).unwrap_or_else(|_| "[]".into());
        let depth = x.max_depth.min(3) as i64;
        let sql = "
WITH RECURSIVE walk(node, depth, score, path) AS (
  SELECT value, 0, 1.0, ',' || value || ',' FROM json_each(?1)
  UNION ALL
  SELECT e.dst, w.depth + 1, w.score * t.hop_weight * e.confidence, w.path || e.dst || ','
  FROM walk w
  JOIN entity n ON n.id = w.node AND n.degree <= ?3
  JOIN edge e INDEXED BY edge_out ON e.src = w.node AND e.valid_to IS NULL
  JOIN edge_type t ON t.name = e.type
  WHERE w.depth < ?2 AND t.traverse = 1
    AND instr(w.path, ',' || e.dst || ',') = 0
    AND NOT EXISTS (SELECT 1 FROM json_each(e.scope) s
                    WHERE json_extract(?4, '$.' || s.key) IS NOT NULL AND json_extract(?4, '$.' || s.key) <> s.value)
  UNION ALL
  SELECT e.src, w.depth + 1, w.score * t.hop_weight * e.confidence, w.path || e.src || ','
  FROM walk w
  JOIN entity n ON n.id = w.node AND n.degree <= ?3
  JOIN edge e INDEXED BY edge_in ON e.dst = w.node AND e.valid_to IS NULL
  JOIN edge_type t ON t.name = e.type
  WHERE w.depth < ?2 AND t.traverse = 1
    AND instr(w.path, ',' || e.src || ',') = 0
    AND NOT EXISTS (SELECT 1 FROM json_each(e.scope) s
                    WHERE json_extract(?4, '$.' || s.key) IS NOT NULL AND json_extract(?4, '$.' || s.key) <> s.value)
)
SELECT node, MAX(score) AS score, MIN(depth) AS hops FROM walk
WHERE depth > 0 AND node NOT IN (SELECT value FROM json_each(?1))
GROUP BY node ORDER BY score DESC, node LIMIT ?5";
        self.db.read(|c| -> Result<ExpandResult, StoreError> {
            let start = Instant::now();
            let budget = x.budget;
            c.progress_handler(1000, Some(move || start.elapsed() > budget))?;
            let r = (|| -> Result<Vec<Reached>, rusqlite::Error> {
                let mut st = c.prepare_cached(sql)?;
                let rows = st.query_map(
                    params![seeds_json, depth, x.hub_degree, ctx, x.limit as i64],
                    |r| {
                        Ok(Reached {
                            entity: r.get(0)?,
                            score: r.get(1)?,
                            hops: r.get::<_, i64>(2)? as u32,
                        })
                    },
                )?;
                rows.collect()
            })();
            c.progress_handler(0, None::<fn() -> bool>)?;
            match r {
                Ok(reached) => Ok(ExpandResult {
                    reached,
                    timed_out: false,
                }),
                Err(rusqlite::Error::SqliteFailure(e, _))
                    if e.code == rusqlite::ErrorCode::OperationInterrupted =>
                {
                    Ok(ExpandResult {
                        reached: vec![],
                        timed_out: true,
                    })
                }
                Err(e) => Err(e.into()),
            }
        })
    }

    /// Live facts attached to entities (directly or as an edge's evidence) whose scope does not conflict with
    /// `ctx`, scored by the entity score times the fact's confidence (ADR-51 §6 "facts reachable from seeds").
    pub fn facts_of_entities(
        &self,
        scored: &[(i64, f64)],
        ctx: &Value,
        limit: usize,
    ) -> Result<Vec<(Fact, f64)>, StoreError> {
        if scored.is_empty() {
            return Ok(vec![]);
        }
        let ctx = normalize_scope(ctx);
        let mut best: HashMap<i64, f64> = HashMap::new();
        self.db.read(|c| -> Result<(), StoreError> {
            let mut st = c.prepare_cached(
                "SELECT fact_id FROM fact_entity WHERE entity_id = ?1
                 UNION SELECT evidence_fact FROM edge WHERE (src = ?1 OR dst = ?1) AND valid_to IS NULL AND evidence_fact IS NOT NULL",
            )?;
            for (id, score) in scored {
                for f in st.query_map([id], |r| r.get::<_, i64>(0))? {
                    let f = f?;
                    let e = best.entry(f).or_insert(0.0);
                    if *e < *score {
                        *e = *score;
                    }
                }
            }
            Ok(())
        })?;
        let ids: Vec<i64> = best.keys().copied().collect();
        let facts = self.db.read(|c| Self::facts_by_ids(c, &ids))?;
        let mut out: Vec<(Fact, f64)> = facts
            .into_values()
            .filter(|f| f.is_live() && f.weight >= crate::agent::HIDE_BELOW_WEIGHT && !f.untrusted)
            .filter(|f| {
                !matches!(
                    scope_match(&normalize_scope(&f.scope), &ctx),
                    ScopeMatch::Conflict
                )
            })
            .map(|f| {
                let s = best[&f.id] * f.confidence;
                (f, s)
            })
            .collect();
        out.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.id.cmp(&b.0.id)));
        out.truncate(limit);
        Ok(out)
    }

    /// Rebuilds `entity_fts` and `entity_vec` (after an import or an embedder change). Vectors are made from the name
    /// alone, as `agenium_memory::resolve::entity_text` makes them (ADR-94).
    pub fn reindex_entities(
        &self,
        embed: impl Fn(&[String]) -> Result<Vec<Vec<f32>>, String>,
    ) -> Result<u64, StoreError> {
        let rows: Vec<(i64, String)> = self.db.read(|c| -> Result<_, StoreError> {
            let mut st =
                c.prepare("SELECT id, name || '. ' FROM entity WHERE merged_into IS NULL")?;
            let v = st
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(v)
        })?;
        let texts: Vec<String> = rows.iter().map(|r| r.1.clone()).collect();
        let vecs = if texts.is_empty() {
            vec![]
        } else {
            embed(&texts).map_err(StoreError::Invalid)?
        };
        if vecs.len() != rows.len() {
            return Err(StoreError::Invalid(
                "embedder returned a different number of vectors".into(),
            ));
        }
        for v in &vecs {
            self.check_vec(v)?;
        }
        self.db.write(|tx| {
            tx.execute("DELETE FROM entity_vec", [])?;
            for ((id, _), v) in rows.iter().zip(&vecs) {
                if usable(v) {
                    tx.execute(
                        "INSERT INTO entity_vec(rowid, embedding) VALUES (?1, ?2)",
                        params![id, vec_blob(v)],
                    )?;
                }
                refresh_fts(tx, *id)?;
            }
            Ok(rows.len() as u64)
        })
    }

    /// Lowers (or raises) a fact's confidence within 0..1 (reuse signals, ADR-51 §8).
    pub fn adjust_confidence(&self, fact: i64, delta: f64) -> Result<(), StoreError> {
        self.db.write(|tx| {
            tx.execute(
                "UPDATE fact SET confidence = MIN(1.0, MAX(0.0, confidence + ?2)) WHERE id = ?1",
                params![fact, delta],
            )?;
            Ok(())
        })
    }

    /// Marks the usage rows of a run with an outcome signal (`reopened`, `closed`, ...).
    pub fn signal_usage(
        &self,
        run: &str,
        fact: Option<i64>,
        signal: &str,
        outcome: Option<&str>,
    ) -> Result<u64, StoreError> {
        self.db.write(|tx| {
            let n = tx.execute(
                &format!(
                    "UPDATE fact_usage SET signal = ?2, outcome = COALESCE(?3, outcome), outcome_at = {}
                     WHERE run = ?1 AND (?4 IS NULL OR fact = ?4)",
                    now()
                ),
                params![run, signal, outcome, fact],
            )?;
            Ok(n as u64)
        })
    }

    /// Facts shown to a run (for reopen attribution).
    pub fn facts_used_in(&self, run: &str) -> Result<Vec<Fact>, StoreError> {
        let ids: Vec<i64> = self
            .usage_for_run(run)?
            .into_iter()
            .map(|(f, _, _)| f)
            .collect();
        let m = self.db.read(|c| Self::facts_by_ids(c, &ids))?;
        Ok(ids.iter().filter_map(|i| m.get(i).cloned()).collect())
    }
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
