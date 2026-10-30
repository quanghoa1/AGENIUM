//! Improving a skill the agent created itself (ADR-146): `propose_skill_update` on a library or role skill writes the
//! agent's own copy (an overlay, decision 35); on a skill in the agent's own folder (ADR-91) there is no base to overlay,
//! so the skill is rewritten in place — under the gates of an overlay: a live, trusted lesson that held in two closed
//! tickets, a reason, no injected instructions or secrets, no new contradiction with another skill, a valid skill
//! before it is swapped in. Without this, an agent that made a skill could never improve it: the refusal of a second
//! skill on the same subject sent it to `propose_skill_update`, which refused its own skills (found on Codex).

use agenium_core::forms::ProposeSkillUpdate;
use agenium_core::*;
use agenium_skills::{Origin, Owner};
use agenium_store::FactKind;

use crate::app::App;
use crate::skill_overlay::{INJECTION, MIN_CONTEXTS};
use crate::{OrchError, Result};

fn refuse<T>(m: impl Into<String>) -> Result<T> {
    Err(OrchError::Invalid(m.into()))
}

impl App {
    /// Whether `name` is a skill `agent` created itself.
    pub(crate) fn is_created_skill(&self, agent: AgentId, name: &str) -> bool {
        self.created_skills(agent)
            .is_ok_and(|c| c.contains_key(name.trim()))
    }

    /// Rewrites the skill `agent` created, from a validated lesson (see the module docs).
    pub(crate) fn revise_created_skill(
        &self,
        agent: AgentId,
        u: &ProposeSkillUpdate,
    ) -> Result<serde_json::Value> {
        let a = self.agent(agent)?;
        let name = u.skill.trim();
        let mut created = self.created_skills(agent)?;
        if !created.contains_key(name) {
            return refuse(format!("{name:?} is not a skill you created"));
        }
        let mem = self.agent_memory_store(agent)?;
        let lesson = mem.get(u.lesson).map_err(|_| {
            OrchError::Invalid(format!("lesson #{} is not in your memory", u.lesson))
        })?;
        if lesson.kind != FactKind::Lesson || !lesson.is_live() {
            return refuse(format!("#{} is not a live lesson", u.lesson));
        }
        if lesson.untrusted {
            return refuse(format!(
                "lesson #{} came from external content (a page, a file, a tool output): it cannot change a skill",
                u.lesson
            ));
        }
        let held = self.evidenced_contexts(&lesson).len();
        if held < MIN_CONTEXTS {
            return refuse(format!(
                "lesson #{} held in {held} context(s) on record (closed tickets); a skill changes only after a lesson \
                 held in {MIN_CONTEXTS}",
                u.lesson
            ));
        }
        if u.reason.trim().is_empty() {
            return refuse("give the reason for the change");
        }
        let own = self
            .skill_registry()
            .dir_of(&Owner::Agent(agent.to_string()))
            .join(name);
        let current = std::fs::read_to_string(own.join(agenium_skills::SKILL_FILE))?;
        let updated = crate::skill_overlay::revised_text(&current, u)?;
        let lower = updated.to_lowercase();
        if let Some(p) = INJECTION
            .iter()
            .find(|p| lower.contains(*p) && !current.to_lowercase().contains(*p))
        {
            return refuse(format!(
                "the change adds {p:?}, which reads as injected instructions: refused"
            ));
        }
        if self.redactor().redact(&updated) != updated
            || !agenium_secrets::scan_text(&updated).is_empty()
        {
            return refuse("the change carries something that looks like a secret: refused");
        }
        // No new contradiction with another skill the agent uses (one already there does not block it, ADR-92).
        let others: Vec<(String, String)> = self
            .known_skills(Some(&a), name)
            .into_iter()
            .map(|k| (k.name, k.guidance))
            .collect();
        let strip = crate::skill_shape::strip_frontmatter;
        let before = crate::skill_conflict::directive_conflicts(name, &strip(&current), &others);
        let introduced: Vec<String> =
            crate::skill_conflict::directive_conflicts(name, &strip(&updated), &others)
                .into_iter()
                .filter(|c| !before.contains(c))
                .collect();
        if !introduced.is_empty() {
            return refuse(format!(
                "the change {}; reconcile the two skills first (or improve the other one)",
                introduced.join("; ")
            ));
        }
        // Write a staging copy, validate it as a skill, then swap it in.
        let staging_root = own.with_file_name(format!(".revise-{}", ulid::Ulid::new()));
        let staging = staging_root.join(name);
        std::fs::create_dir_all(&staging)?;
        std::fs::write(staging.join(agenium_skills::SKILL_FILE), &updated)?;
        if let Err(e) = agenium_skills::validate(&staging, Origin::Agent) {
            let _ = std::fs::remove_dir_all(&staging_root);
            return refuse(format!("the revised skill is not valid: {e}"));
        }
        std::fs::write(own.join(agenium_skills::SKILL_FILE), &updated)?;
        let _ = std::fs::remove_dir_all(&staging_root);
        if let Some(c) = created.get_mut(name) {
            c.revisions.push(crate::skill_create::Revision {
                lesson: u.lesson,
                lesson_text: lesson.text.clone(),
                reason: u.reason.trim().to_owned(),
                at: now_iso(),
            });
        }
        crate::skill_create::save_created(&mem, &created)?;
        self.store().commit(None, &format!("agent:{agent}"), |w| {
            w.emit(DomainEvent::AgentUpdated {
                agent,
                field: format!("skill_revised:{name}"),
            })
            .map_err(OrchError::from)
        })?;
        Ok(serde_json::json!({
            "status": "revised",
            "skill": name,
            "note": "your own skill is rewritten; the user can review or delete it",
        }))
    }
}
