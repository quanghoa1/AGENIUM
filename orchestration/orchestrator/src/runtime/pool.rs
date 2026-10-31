//! The shared lesson pool (ADR-87): an agent's general lessons copied in, the copies following their lessons, and a
//! copy that misled other agents' work weighed down and dropped (audit M3, 2026-10-31).

use agenium_core::*;
use agenium_memory::pipeline::{Episode, HURT_DELTA};

use super::Runtime;
use crate::Result;

impl Runtime {
    /// Promotes `agent`'s generalized lessons into the shared pool (ADR-87): a live, trusted Lesson that held in two
    /// or more distinct contexts and is not scoped to one project. The pool copy drops the project, keeps scope and cue,
    /// and caps confidence; pool recall ranks it below the recalling agent's own memory. Deduped by text, so repeated
    /// runs do not pile up copies. No-op when `[memory] share` is off.
    ///
    /// The pool follows its sources (audit M3, 2026-10-31: copies were only ever added): a copy names the lesson it
    /// came from (`shared:<agent>:<fact id>`), and one whose lesson is no longer a live general lesson of the agent
    /// (retired as misleading, refined into a new one, scoped to a project) or whose confidence fell below the copy's
    /// is retired from the pool. Copies made before (`shared:<agent>`) are matched by their text.
    pub(super) fn share_lessons(
        &self,
        agent: AgentId,
        mem: &std::sync::Arc<agenium_store::AgentStore>,
    ) -> Result<usize> {
        if !self.app.config().memory.share {
            return Ok(0);
        }
        let pool = self.app.pool_memory()?;
        let general: Vec<agenium_store::Fact> = mem
            .live(10_000)?
            .into_iter()
            .filter(|f| {
                let project_scoped = f
                    .scope
                    .as_object()
                    .is_some_and(|o| o.contains_key("project"));
                f.kind == agenium_store::FactKind::Lesson
                    && !f.untrusted
                    && !project_scoped
                    && self.app.evidenced_contexts(f).len() >= crate::skill_overlay::MIN_CONTEXTS
            })
            .collect();
        // A text the pool dropped as misleading (audit M3) is not shared again, by this agent or another.
        let mut dropped = std::collections::HashSet::new();
        for f in &general {
            if pool.meta(&pool_dropped_key(&f.text))?.is_some() {
                dropped.insert(norm_text(&f.text));
            }
        }
        let mine = format!("shared:{agent}");
        let (retire, add) = pool_changes(&mine, &general, &pool.live(10_000)?, &dropped);
        for id in retire {
            pool.retire(id)?;
        }
        let mut n = 0;
        for f in add.into_iter().map(|i| &general[i]) {
            let nf = agenium_store::NewFact {
                text: f.text.clone(),
                kind: agenium_store::FactKind::Lesson,
                project: None,
                scope: f.scope.clone(),
                cue: f.cue.clone(),
                evidence: None,
                source: format!("{mine}:{}", f.id),
                untrusted: false,
                confidence: f.confidence.min(POOL_CONFIDENCE),
            };
            let vector =
                agenium_memory::cpu_bound(|| self.embedder.embed(std::slice::from_ref(&f.text)))
                    .ok()
                    .and_then(|mut v| v.pop());
            pool.add(&nf, vector.as_deref())?;
            n += 1;
        }
        Ok(n)
    }

    /// Audit M3 (2026-10-31): a shared lesson that misled another agent's work (`ids`, pool fact ids the learning named)
    /// weighs on the pool's copy only. It loses confidence ([`HURT_DELTA`]); once it misled work in
    /// [`MIN_CONTEXTS`](crate::skill_overlay::MIN_CONTEXTS) contexts it leaves the pool, and its text is not shared
    /// again by any agent. The lesson it was copied from stays as it is in its agent's memory, and nothing another agent
    /// does raises a copy. Returns the texts that left the pool.
    pub(super) fn pool_misled(&self, ep: &Episode, ids: &[i64]) -> Result<Vec<String>> {
        if ids.is_empty() {
            return Ok(vec![]);
        }
        let pool = self.app.pool_memory()?;
        // A context as the agent's own lessons count them (`validate_in`): the project and the ticket.
        let context = format!("{}/{}", ep.project.as_deref().unwrap_or("-"), ep.source);
        let mut dropped = Vec::new();
        for &id in ids {
            let Ok(f) = pool.get(id) else { continue };
            if !f.is_live() {
                continue;
            }
            pool.adjust_confidence(id, HURT_DELTA)?;
            if let Some(run) = &ep.run {
                pool.signal_usage(run, Some(id), "reopened", Some("hurt"))?;
            }
            if pool.misled_in(id, &context)? >= crate::skill_overlay::MIN_CONTEXTS {
                pool.retire(id)?;
                pool.set_meta(&pool_dropped_key(&f.text), &chrono::Utc::now().to_rfc3339())?;
                dropped.push(f.text);
            }
        }
        Ok(dropped)
    }
}

/// The most confidence a lesson copied into the shared pool keeps.
const POOL_CONFIDENCE: f64 = 0.6;

/// A lesson's text as the pool compares texts.
fn norm_text(s: &str) -> String {
    s.trim().to_lowercase()
}

/// The pool's mark on a text it dropped as misleading (audit M3).
fn pool_dropped_key(text: &str) -> String {
    format!("pool-dropped:{}", norm_text(text))
}

/// What sharing changes in the pool for one agent (`mine` = `shared:<agent>`): the copies to retire (their lesson is
/// no longer among `general`, or its confidence fell below the copy's) and the lessons of `general` (by index) to copy.
/// A copy names its lesson by id (`shared:<agent>:<id>`); one from before that (`shared:<agent>`) by its text. Text
/// already in the pool (another agent's copy) is not copied again, nor text in `dropped` (texts the pool dropped as
/// misleading, [`norm_text`]): a lesson of such a text counts as not general.
fn pool_changes(
    mine: &str,
    general: &[agenium_store::Fact],
    pool: &[agenium_store::Fact],
    dropped: &std::collections::HashSet<String>,
) -> (Vec<i64>, Vec<usize>) {
    let norm = norm_text;
    let general: Vec<(usize, &agenium_store::Fact)> = general
        .iter()
        .enumerate()
        .filter(|(_, f)| !dropped.contains(&norm(&f.text)))
        .collect();
    let with_id = format!("{mine}:");
    let mut retire = Vec::new();
    let mut have = std::collections::HashSet::new();
    for copy in pool {
        if copy.source != mine && !copy.source.starts_with(&with_id) {
            have.insert(norm(&copy.text));
            continue;
        }
        let source = match copy.source.strip_prefix(&with_id) {
            Some(id) => general.iter().find(|(_, f)| id.parse() == Ok(f.id)),
            None => general
                .iter()
                .find(|(_, f)| norm(&f.text) == norm(&copy.text)),
        }
        .map(|(_, f)| *f);
        match source {
            Some(f) if f.confidence.min(POOL_CONFIDENCE) + 1e-9 >= copy.confidence => {
                have.insert(norm(&copy.text));
            }
            _ => retire.push(copy.id),
        }
    }
    let add = general
        .iter()
        .filter(|(_, f)| have.insert(norm(&f.text)))
        .map(|(i, _)| *i)
        .collect();
    (retire, add)
}

#[cfg(test)]
mod tests {
    fn lesson(id: i64, text: &str, source: &str, confidence: f64) -> agenium_store::Fact {
        agenium_store::Fact {
            id,
            text: text.into(),
            kind: agenium_store::FactKind::Lesson,
            project: None,
            scope: serde_json::json!({}),
            cue: None,
            evidence: None,
            source: source.into(),
            untrusted: false,
            confidence,
            weight: 1.0,
            model_id: None,
            superseded_by: None,
            created_at: String::new(),
            updated_at: String::new(),
            used_at: None,
            validated: vec![],
        }
    }

    /// Audit M3 (2026-10-31): the pool only ever grew. A copy whose lesson was retired, refined into another or
    /// lost confidence goes; a new lesson is copied once; another agent's copy of the same text is kept.
    #[test]
    fn the_pool_follows_the_lessons_it_was_copied_from() {
        let mine = "shared:A";
        let general = vec![
            lesson(10, "Run the migrations before the tests", "chat", 0.9),
            lesson(12, "Pin the toolchain (refined)", "chat", 0.8),
            lesson(13, "Prefer small commits", "chat", 0.3),
            lesson(14, "Read the README first", "chat", 0.9),
        ];
        let pool = vec![
            lesson(1, "Run the migrations before the tests", "shared:A:10", 0.6),
            // Lesson 11 was refined into 12: its copy goes, 12 is copied.
            lesson(2, "Pin the toolchain", "shared:A:11", 0.6),
            // Lesson 13's confidence fell to 0.3: the copy at 0.6 goes and a capped one comes.
            lesson(3, "Prefer small commits", "shared:A:13", 0.6),
            // A copy from before ids: kept while its text is a general lesson, else retired.
            lesson(4, "Retired as misleading", "shared:A", 0.6),
            // Another agent's copy of the same text: kept, and not copied again.
            lesson(5, "Read the README first", "shared:B:7", 0.6),
        ];
        let none = std::collections::HashSet::new();
        let (retire, add) = super::pool_changes(mine, &general, &pool, &none);
        assert_eq!(retire, vec![2, 3, 4]);
        let added: Vec<i64> = add.iter().map(|i| general[*i].id).collect();
        assert_eq!(added, vec![12, 13]);

        // Audit M3: a text the pool dropped as misleading is not copied again, and a copy of it still there goes
        // (compared as the pool compares texts: case and outer spaces aside).
        let dropped: std::collections::HashSet<String> = [
            "  run the MIGRATIONS before the tests",
            "pin the toolchain (refined)",
        ]
        .iter()
        .map(|t| super::norm_text(t))
        .collect();
        let (retire, add) = super::pool_changes(mine, &general, &pool, &dropped);
        assert_eq!(retire, vec![1, 2, 3, 4]);
        let added: Vec<i64> = add.iter().map(|i| general[*i].id).collect();
        assert_eq!(added, vec![13], "neither dropped text comes back");
    }
}
