//! What the settling pass needs of the store (ADR-145): the facts' vectors, the verdicts on pairs of facts, folding one
//! fact into another, and the conflicts the user decides.

use super::*;

/// A verdict on a pair of facts (`fact_pair.verdict`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PairVerdict {
    /// They say the same: one was folded into the other.
    Same,
    /// Both hold: kept.
    Distinct,
    /// Both cannot hold: kept until the user decides.
    Conflict,
}

impl PairVerdict {
    pub fn as_str(self) -> &'static str {
        match self {
            PairVerdict::Same => "same",
            PairVerdict::Distinct => "distinct",
            PairVerdict::Conflict => "conflict",
        }
    }
}

/// Two live facts that may contradict each other, for the user.
#[derive(Debug, Clone, Serialize)]
pub struct FactConflict {
    pub a: Fact,
    pub b: Fact,
    pub similarity: f64,
    /// Who found it: `figures` (the same words, other numbers), `judge`.
    pub by: String,
}

/// What the user decides of a conflict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConflictChoice {
    /// The first holds: the second is folded into it.
    A,
    B,
    /// Both hold (different cases).
    Both,
}

fn ordered(a: i64, b: i64) -> (i64, i64) {
    (a.min(b), a.max(b))
}

impl AgentStore {
    /// The stored vectors of `ids` (facts without one are absent).
    pub fn vectors_of(&self, ids: &[i64]) -> Result<HashMap<i64, Vec<f32>>, StoreError> {
        self.db.read(|c| {
            let mut st = c.prepare_cached("SELECT embedding FROM fact_vec WHERE rowid = ?1")?;
            let mut out = HashMap::new();
            for id in ids {
                let blob: Option<Vec<u8>> = st.query_row([id], |r| r.get(0)).optional()?;
                if let Some(b) = blob {
                    out.insert(
                        *id,
                        b.as_chunks::<4>()
                            .0
                            .iter()
                            .map(|x| f32::from_le_bytes(*x))
                            .collect(),
                    );
                }
            }
            Ok(out)
        })
    }

    /// The verdict already given on the pair, if any.
    pub fn pair_verdict(&self, a: i64, b: i64) -> Result<Option<PairVerdict>, StoreError> {
        let (a, b) = ordered(a, b);
        self.db.read(|c| {
            let v: Option<String> = c
                .query_row(
                    "SELECT verdict FROM fact_pair WHERE a = ?1 AND b = ?2",
                    params![a, b],
                    |r| r.get(0),
                )
                .optional()?;
            Ok(v.and_then(|v| serde_json::from_value(serde_json::Value::String(v)).ok()))
        })
    }

    /// The earlier versions of a fact (the facts an UPDATE replaced by it, and theirs), newest first.
    pub fn earlier_versions(&self, id: i64) -> Result<Vec<i64>, StoreError> {
        self.db.read(|c| {
            let mut st =
                c.prepare_cached("SELECT id FROM fact WHERE superseded_by = ?1 AND id != ?1")?;
            let (mut out, mut next) = (Vec::new(), vec![id]);
            while let Some(f) = next.pop() {
                let older = st
                    .query_map([f], |r| r.get::<_, i64>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                for o in older {
                    if !out.contains(&o) && o != id {
                        out.push(o);
                        next.push(o);
                    }
                }
            }
            out.sort_by(|a, b| b.cmp(a));
            Ok(out)
        })
    }

    /// Every pair a verdict was given on, `(smaller id, larger id)`.
    pub fn judged_pairs(&self) -> Result<std::collections::HashSet<(i64, i64)>, StoreError> {
        self.db.read(|c| {
            let mut st = c.prepare_cached("SELECT a, b FROM fact_pair")?;
            let rows = st
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<rusqlite::Result<std::collections::HashSet<(i64, i64)>>>()?;
            Ok(rows)
        })
    }

    /// Records a verdict on a pair (`by`: who gave it).
    pub fn set_pair(
        &self,
        a: i64,
        b: i64,
        verdict: PairVerdict,
        similarity: f64,
        by: &str,
    ) -> Result<(), StoreError> {
        let (a, b) = ordered(a, b);
        self.db.write(|tx| {
            tx.execute(
                "INSERT INTO fact_pair(a, b, verdict, similarity, by) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(a, b) DO UPDATE SET verdict = excluded.verdict, similarity = excluded.similarity,
                 by = excluded.by, at = strftime('%Y-%m-%dT%H:%M:%fZ','now'), resolved = NULL",
                params![a, b, verdict.as_str(), similarity, by],
            )?;
            Ok(())
        })
    }

    /// Folds `other` into `keep` (they say the same): `other` is superseded by `keep` and keeps its history, the contexts
    /// it was validated in move to `keep`, the higher confidence stays. Both must be live and of the same trust.
    pub fn fold(&self, keep: i64, other: i64) -> Result<(), StoreError> {
        let (k, o) = (self.get(keep)?, self.get(other)?);
        if !k.is_live() || !o.is_live() {
            return Err(StoreError::Conflict(format!(
                "fact {keep} or {other} is no longer live"
            )));
        }
        if k.untrusted != o.untrusted {
            return Err(StoreError::Conflict(
                "a trusted fact and an untrusted one are not folded together".into(),
            ));
        }
        let mut validated = k.validated.clone();
        for c in o.validated {
            if !validated.contains(&c) {
                validated.push(c);
            }
        }
        self.db.write(|tx| {
            tx.execute(
                "UPDATE fact SET superseded_by = ?2, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id = ?1",
                params![other, keep],
            )?;
            tx.execute(
                "UPDATE fact SET validated = ?2, confidence = max(confidence, ?3) WHERE id = ?1",
                params![keep, serde_json::to_string(&validated)?, o.confidence],
            )?;
            Self::drop_vector(tx, other)
        })?;
        self.vectors.after_write();
        Ok(())
    }

    /// Conflicts the user has not decided whose facts are both still live, the closest first.
    pub fn open_conflicts(&self) -> Result<Vec<FactConflict>, StoreError> {
        let rows: Vec<(i64, i64, f64, String)> = self.db.read(|c| {
            let mut st = c.prepare_cached(
                "SELECT a, b, similarity, by FROM fact_pair WHERE verdict = 'conflict' AND resolved IS NULL
                 ORDER BY similarity DESC",
            )?;
            let rows = st
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok::<_, StoreError>(rows)
        })?;
        let mut out = Vec::new();
        for (a, b, similarity, by) in rows {
            let (fa, fb) = (self.get(a)?, self.get(b)?);
            if fa.is_live() && fb.is_live() {
                out.push(FactConflict {
                    a: fa,
                    b: fb,
                    similarity,
                    by,
                });
            }
        }
        Ok(out)
    }

    /// The user's decision on a conflict: the fact that holds stays and the other is folded into it, or both stay.
    pub fn resolve_conflict(
        &self,
        a: i64,
        b: i64,
        choice: ConflictChoice,
    ) -> Result<(), StoreError> {
        match choice {
            ConflictChoice::A => self.fold(a, b)?,
            ConflictChoice::B => self.fold(b, a)?,
            ConflictChoice::Both => {}
        }
        let (x, y) = ordered(a, b);
        let resolved = match choice {
            ConflictChoice::A => format!("kept {a}"),
            ConflictChoice::B => format!("kept {b}"),
            ConflictChoice::Both => "both".to_owned(),
        };
        self.db.write(|tx| {
            let n = tx.execute(
                "UPDATE fact_pair SET resolved = ?3, verdict = CASE WHEN ?4 THEN 'distinct' ELSE verdict END
                 WHERE a = ?1 AND b = ?2 AND verdict = 'conflict'",
                params![x, y, resolved, choice == ConflictChoice::Both],
            )?;
            if n == 0 {
                return Err(StoreError::NotFound(format!("conflict between facts {a} and {b}")));
            }
            Ok(())
        })
    }
}
