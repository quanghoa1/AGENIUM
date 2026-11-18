//! Improving a skill the agent created itself (ADR-146): `propose_skill_update` on a library or role skill writes the
//! agent's own copy (an overlay, decision 35); on a skill in the agent's own folder (ADR-91) there is no base to overlay,
//! so the skill is rewritten in place — under the gates of an overlay: a live, trusted lesson that held in two closed
//! tickets, a reason, no injected instructions or secrets, no new contradiction with another skill, a valid skill
//! before it is swapped in. Without this, an agent that made a skill could never improve it: the refusal of a second
//! skill on the same subject sent it to `propose_skill_update`, which refused its own skills (found on Codex).

use agenium_core::forms::ProposeSkillUpdate;
use agenium_core::*;
use agenium_skills::{Origin, Owner};

use crate::app::App;
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
        let lesson = self.lesson_gate(agent, u.lesson, &u.reason)?;
        let mem = self.agent_memory_store(agent)?;
        let own = self
            .skill_registry()
            .dir_of(&Owner::Agent(agent.to_string()))
            .join(name);
        let current = std::fs::read_to_string(own.join(agenium_skills::SKILL_FILE))?;
        let updated = crate::skill_overlay::revised_text(&current, u)?;
        self.text_gate(&[&current], &updated, "change")?;
        self.conflict_gate(&a, name, &current, &updated)?;
        // The shape and size gate a new skill passes (`lint_skill`: its sections, `## Proof`, the length cap, not a
        // copy of another): a revision skipped it and could drop the proof or grow past the cap (audit M4,
        // 2026-10-31; ADR-147: a revision passes the same gates). Only what the change brings counts, as above.
        let reg = self.skill_registry();
        let mut bodies: Vec<(String, String)> = Vec::new();
        for s in reg
            .for_agent(&agent.to_string(), &a.role, &a.skills)
            .skills
            .iter()
            .chain(reg.library().skills.iter())
        {
            if s.meta.name != name && !bodies.iter().any(|(n, _)| *n == s.meta.name) {
                bodies.push((s.meta.name.clone(), s.body.clone()));
            }
        }
        let lint = |text: &str| {
            crate::skill_shape::lint_skill(
                name,
                &crate::skill_shape::strip_frontmatter(text),
                &bodies,
            )
        };
        let had = lint(&current);
        let brought: Vec<String> = lint(&updated)
            .into_iter()
            .filter(|n| !had.contains(n))
            .collect();
        if !brought.is_empty() {
            return refuse(format!(
                "the revised skill is not ready: {}",
                brought.join("; ")
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
