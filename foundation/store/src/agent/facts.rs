//! What the store holds: facts (new, stored, how they were found), skill overlays, and their row mapping.

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FactKind {
    Fact,
    Preference,
    Lesson,
}

impl FactKind {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Fact => "fact",
            Self::Preference => "preference",
            Self::Lesson => "lesson",
        }
    }
    pub(super) fn parse(s: &str) -> Result<Self, StoreError> {
        match s {
            "fact" => Ok(Self::Fact),
            "preference" => Ok(Self::Preference),
            "lesson" => Ok(Self::Lesson),
            o => Err(StoreError::Corrupt(format!("fact kind {o:?}"))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewFact {
    pub text: String,
    pub kind: FactKind,
    pub project: Option<String>,
    pub scope: serde_json::Value,
    pub cue: Option<String>,
    pub evidence: Option<String>,
    pub source: String,
    pub untrusted: bool,
    pub confidence: f64,
}

impl NewFact {
    pub fn fact(text: impl Into<String>, source: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind: FactKind::Fact,
            project: None,
            scope: serde_json::json!({}),
            cue: None,
            evidence: None,
            source: source.into(),
            untrusted: false,
            confidence: 0.5,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Fact {
    pub id: i64,
    pub text: String,
    pub kind: FactKind,
    pub project: Option<String>,
    pub scope: serde_json::Value,
    pub cue: Option<String>,
    pub evidence: Option<String>,
    pub source: String,
    pub untrusted: bool,
    pub confidence: f64,
    pub weight: f64,
    pub model_id: Option<String>,
    pub superseded_by: Option<i64>,
    pub created_at: String,
    pub updated_at: String,
    pub used_at: Option<String>,
    /// Contexts in which the fact (a lesson) was used and the work held (decision 35).
    #[serde(default)]
    pub validated: Vec<String>,
}

impl Fact {
    pub fn is_live(&self) -> bool {
        self.superseded_by.is_none()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Via {
    Vector,
    Keyword,
    Hybrid,
    Graph,
    Recent,
    /// A rule of the work's context: a proven lesson whose scope matches it (decision 35), shown whatever the
    /// text channels find.
    Rule,
}

impl Via {
    pub fn as_str(self) -> &'static str {
        match self {
            Via::Vector => "vector",
            Via::Keyword => "keyword",
            Via::Hybrid => "hybrid",
            Via::Graph => "graph",
            Via::Recent => "recent",
            Via::Rule => "rule",
        }
    }
}

/// An agent's copy-on-write version of a skill (decision 35).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OverlayRow {
    pub skill: String,
    pub base_version: String,
    /// Unified diff from the base SKILL.md to the overlay's.
    pub patch: String,
    /// One entry per update: when, why, the lesson behind it.
    pub changelog: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Hit {
    pub fact: Fact,
    pub score: f64,
    pub via: Via,
}

pub(super) const FACT_COLS: &str = "id, text, kind, project, scope, cue, evidence, source, untrusted, confidence, weight, model_id, \
    superseded_by, created_at, updated_at, used_at, validated";

pub(super) fn map_fact(r: &rusqlite::Row<'_>) -> rusqlite::Result<(Fact, String, String)> {
    let validated: String = r.get(16)?;
    Ok((
        Fact {
            id: r.get(0)?,
            text: r.get(1)?,
            kind: FactKind::Fact,
            project: r.get(3)?,
            scope: serde_json::Value::Null,
            cue: r.get(5)?,
            evidence: r.get(6)?,
            source: r.get(7)?,
            untrusted: r.get(8)?,
            confidence: r.get(9)?,
            weight: r.get(10)?,
            model_id: r.get(11)?,
            superseded_by: r.get(12)?,
            created_at: r.get(13)?,
            updated_at: r.get(14)?,
            used_at: r.get(15)?,
            validated: serde_json::from_str(&validated).unwrap_or_default(),
        },
        r.get(2)?,
        r.get(4)?,
    ))
}

pub(super) fn decode_fact(v: (Fact, String, String)) -> Result<Fact, StoreError> {
    let (mut f, kind, scope) = v;
    f.kind = FactKind::parse(&kind)?;
    f.scope = crate::core::from_json(&scope)?;
    Ok(f)
}
