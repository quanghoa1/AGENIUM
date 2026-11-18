//! The gates a change to an agent's skill passes, wherever it comes from (decision 35, ADR-91/92/146): the lesson
//! behind it, the text it brings (no injected instructions, no secrets) and the contradictions it adds.

use agenium_core::AgentId;
use agenium_store::FactKind;

use crate::app::App;
use crate::skill_overlay::{INJECTION, MIN_CONTEXTS};
use crate::{OrchError, Result};

fn refuse<T>(m: impl Into<String>) -> Result<T> {
    Err(OrchError::Invalid(m.into()))
}

impl App {
    /// The lesson `lesson` of `agent` may change a skill: live, trusted (not from external content), held in
    /// [`MIN_CONTEXTS`] closed tickets, with a reason given. The lesson.
    pub(crate) fn lesson_gate(
        &self,
        agent: AgentId,
        lesson: i64,
        reason: &str,
    ) -> Result<agenium_store::Fact> {
        let mem = self.agent_memory_store(agent)?;
        let l = mem
            .get(lesson)
            .map_err(|_| OrchError::Invalid(format!("lesson #{lesson} is not in your memory")))?;
        if l.kind != FactKind::Lesson || !l.is_live() {
            return refuse(format!("#{lesson} is not a live lesson"));
        }
        // ADR-92: like a new skill, a lesson that came from external content never changes a skill.
        if l.untrusted {
            return refuse(format!(
                "lesson #{lesson} came from external content (a page, a file, a tool output): it cannot change a skill"
            ));
        }
        let held = self.evidenced_contexts(&l).len();
        if held < MIN_CONTEXTS {
            return refuse(format!(
                "lesson #{lesson} held in {held} context(s) on record (closed tickets); a skill changes only after a \
                 lesson held in {MIN_CONTEXTS}"
            ));
        }
        if reason.trim().is_empty() {
            return refuse("give the reason for the change");
        }
        Ok(l)
    }

    /// `after` adds no injected instruction that none of `before` had, and carries nothing that looks like a secret.
    /// `what` names it in the refusal ("change", "merge").
    pub(crate) fn text_gate(&self, before: &[&str], after: &str, what: &str) -> Result<()> {
        let lower = after.to_lowercase();
        let had: Vec<String> = before.iter().map(|b| b.to_lowercase()).collect();
        if let Some(p) = INJECTION
            .iter()
            .find(|p| lower.contains(*p) && !had.iter().any(|b| b.contains(*p)))
        {
            return refuse(format!(
                "the {what} adds {p:?}, which reads as injected instructions: refused"
            ));
        }
        if self.redactor().redact(after) != after || !agenium_secrets::scan_text(after).is_empty() {
            return refuse(format!(
                "the {what} carries something that looks like a secret: refused"
            ));
        }
        Ok(())
    }

    /// The change of skill `name` from `current` to `updated` adds no explicit contradiction with another skill
    /// `agent` uses (one already there does not block it, ADR-92).
    pub(crate) fn conflict_gate(
        &self,
        a: &agenium_store::core::agents::Agent,
        name: &str,
        current: &str,
        updated: &str,
    ) -> Result<()> {
        let others: Vec<(String, String)> = self
            .known_skills(Some(a), name)
            .into_iter()
            .map(|k| (k.name, k.guidance))
            .collect();
        let strip = crate::skill_shape::strip_frontmatter;
        let before = crate::skill_conflict::directive_conflicts(name, &strip(current), &others);
        let introduced: Vec<String> =
            crate::skill_conflict::directive_conflicts(name, &strip(updated), &others)
                .into_iter()
                .filter(|c| !before.contains(c))
                .collect();
        if !introduced.is_empty() {
            return refuse(format!(
                "the change {}; reconcile the two skills first (or improve the other one)",
                introduced.join("; ")
            ));
        }
        Ok(())
    }
}
