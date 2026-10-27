//! Agent-created skills (ADR-91): an agent may distil a NEW skill into its own skills folder (`propose_new_skill`), the
//! way it updates an existing one through an overlay (ADR-73) — but under stricter gates, because a new skill is new
//! knowledge that steers every later run. The ideas come from hermes-agent (skills created after complex work) and
//! harness2 (keep only evidence-supported lessons, check a change before adopting it); the mechanism is our own:
//!
//! - **evidence**: the lesson behind the skill is a live, trusted Lesson (not from web pages or tool output) that held
//!   in at least two distinct contexts; one lesson yields at most one new skill;
//! - **no sprawl**: the name is not a skill the agent already has (library, role, own), the body is not a
//!   near-duplicate of any of them (then it must improve that one with `propose_skill_update`), and an agent holds at
//!   most [`MAX_CREATED_SKILLS`] created skills;
//! - **well-formed**: the full skill shape (Job, When to use, Steps, Tools, Proof) is filled and the body stays short
//!   (ADR-88); only a SKILL.md — an agent never writes scripts or reference files into a skill;
//! - **safe**: no injected instructions, no secrets; it is validated as a skill before it is swapped in;
//! - **contained**: it lives only in the agent's own folder, so it never changes the library, a role, or another agent;
//! - **accountable, anti-drift**: every creation is audited and listed for the user, who can delete it; when the
//!   lesson stops holding (deleted, retired, replaced, or its confidence falls after reopens) the skill is flagged and
//!   the user is asked to review it.

use std::collections::BTreeMap;

use agenium_core::forms::ProposeNewSkill;
use agenium_core::*;
use agenium_skills::{Origin, Owner};
use agenium_store::{AgentStore, FactKind};
use serde::{Deserialize, Serialize};

use crate::app::App;
use crate::skill_overlay::{INJECTION, MIN_CONTEXTS};
use crate::{OrchError, Result};

/// Most skills one agent may create (overlays of existing skills do not count).
pub const MAX_CREATED_SKILLS: usize = 12;

/// Below this confidence a lesson no longer supports the skill built on it.
pub const DOUBTFUL_CONFIDENCE: f64 = 0.3;

/// Agent-memory meta key holding the created skills (name -> record).
const META_KEY: &str = "created_skills";

/// What the agent created and why (kept in its memory database, shown to the user).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreatedSkill {
    pub lesson: i64,
    pub lesson_text: String,
    pub reason: String,
    pub at: String,
    /// Set when the lesson stopped holding: why, and when it was noticed.
    #[serde(default)]
    pub flagged: Option<String>,
}

fn refuse<T>(m: impl Into<String>) -> Result<T> {
    Err(OrchError::Invalid(m.into()))
}

fn load(mem: &AgentStore) -> Result<BTreeMap<String, CreatedSkill>> {
    Ok(mem
        .meta(META_KEY)?
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default())
}

fn save(mem: &AgentStore, map: &BTreeMap<String, CreatedSkill>) -> Result<()> {
    let s = serde_json::to_string(map).map_err(|e| OrchError::Invalid(e.to_string()))?;
    mem.set_meta(META_KEY, &s)?;
    Ok(())
}

impl App {
    /// The skills `agent` created, for the user (name -> record).
    pub fn created_skills(&self, agent: AgentId) -> Result<BTreeMap<String, CreatedSkill>> {
        load(self.agent_memory_store(agent)?.as_ref())
    }

    /// Creates a new skill in the agent's own folder after every gate passes (see the module docs). Refusals name the
    /// failed gate so the agent can fix the proposal or use `propose_skill_update` instead.
    pub fn create_agent_skill(
        &self,
        agent: AgentId,
        u: &ProposeNewSkill,
    ) -> Result<serde_json::Value> {
        let a = self.agent(agent)?;
        let name = u.name.trim();
        if !agenium_skills::valid_name(name) {
            return refuse(format!(
                "name {name:?} must be 1-{} characters of a-z, 0-9 and single hyphens",
                agenium_skills::MAX_NAME
            ));
        }
        let description = u.description.trim();
        if description.is_empty()
            || description.contains('\n')
            || description.chars().count() > agenium_skills::MAX_DESCRIPTION
        {
            return refuse("give a one-line description (when the skill applies)");
        }
        if u.reason.trim().is_empty() {
            return refuse("give the reason for the new skill");
        }

        // Evidence: a live, trusted lesson that held in two contexts, not yet turned into a skill.
        let mem = self.agent_memory_store(agent)?;
        let lesson = mem.get(u.lesson).map_err(|_| {
            OrchError::Invalid(format!("lesson #{} is not in your memory", u.lesson))
        })?;
        if lesson.kind != FactKind::Lesson || !lesson.is_live() {
            return refuse(format!("#{} is not a live lesson", u.lesson));
        }
        if lesson.untrusted {
            return refuse(format!(
                "lesson #{} came from external content (a page, a file, a tool output): it cannot become a skill",
                u.lesson
            ));
        }
        let held = self.evidenced_contexts(&lesson).len();
        if held < MIN_CONTEXTS {
            return refuse(format!(
                "lesson #{} held in {held} context(s) on record (closed tickets); a new skill needs a lesson that \
                 held in {MIN_CONTEXTS}",
                u.lesson
            ));
        }
        let mut created = load(&mem)?;
        if let Some((n, _)) = created.iter().find(|(_, c)| c.lesson == u.lesson) {
            return refuse(format!(
                "lesson #{} already became skill {n}: improve that one with propose_skill_update",
                u.lesson
            ));
        }

        // No sprawl: not a name the agent has, under the cap.
        let reg = self.skill_registry();
        let mine = reg.for_agent(&agent.to_string(), &a.role, &a.skills);
        let lib = reg.library();
        let own_dir = reg.dir_of(&Owner::Agent(agent.to_string()));
        if mine.get(name).is_some() || lib.get(name).is_some() || own_dir.join(name).exists() {
            return refuse(format!(
                "a skill named {name} already exists for you: improve it with propose_skill_update instead"
            ));
        }
        if created.len() >= MAX_CREATED_SKILLS {
            return refuse(format!(
                "you already created {MAX_CREATED_SKILLS} skills: improve one of them, or ask the user to remove one"
            ));
        }

        // Safe: no injected instructions, no secrets.
        let body = u.body.trim();
        let lower = format!("{description}\n{body}").to_lowercase();
        if let Some(p) = INJECTION.iter().find(|p| lower.contains(*p)) {
            return refuse(format!(
                "the skill contains {p:?}, which reads as injected instructions: refused"
            ));
        }
        let text = format!("---\nname: {name}\ndescription: {description}\n---\n\n{body}\n");
        if self.redactor().redact(&text) != text || !agenium_secrets::scan_text(&text).is_empty() {
            return refuse("the skill carries something that looks like a secret: refused");
        }

        // Well-formed and not a duplicate: the full shape check must come back clean.
        let mut others: Vec<(String, String)> = mine
            .skills
            .iter()
            .map(|s| (s.meta.name.clone(), s.body.clone()))
            .collect();
        for s in &lib.skills {
            if !others.iter().any(|(n, _)| *n == s.meta.name) {
                others.push((s.meta.name.clone(), s.body.clone()));
            }
        }
        let notes = crate::skill_shape::lint_skill(name, body, &others);
        if !notes.is_empty() {
            return refuse(format!("the new skill is not ready: {}", notes.join("; ")));
        }

        // No contradiction (ADR-92). One subject, one skill: a subject already covered must be improved in place, so
        // two skills about the same thing can never drift apart. Then no explicit opposite directive against any skill.
        let known = self.known_skills(Some(&a), name);
        let subjects: Vec<(String, String)> = known
            .iter()
            .map(|k| (k.name.clone(), k.subject.clone()))
            .collect();
        let me = crate::skill_conflict::subject(description, body);
        if let Some((other, score, how)) =
            crate::skill_conflict::same_subject(&me, &subjects, Some(self.embedder().as_ref()))
        {
            return refuse(format!(
                "skill {other} already covers this subject (same {how}, {:.0}%): improve it with propose_skill_update \
                 instead of adding a second skill about the same thing",
                score * 100.0
            ));
        }
        let guidance: Vec<(String, String)> = known
            .iter()
            .map(|k| (k.name.clone(), k.guidance.clone()))
            .collect();
        let conflicts = crate::skill_conflict::directive_conflicts(name, body, &guidance);
        if !conflicts.is_empty() {
            return refuse(format!(
                "the new skill {}; reconcile it with that skill (or improve that skill) first",
                conflicts.join("; ")
            ));
        }

        // Write a staging copy, validate it as a skill, then swap it in.
        std::fs::create_dir_all(&own_dir)?;
        let staging_root = own_dir.join(format!(".new-{}", ulid::Ulid::new()));
        let staging = staging_root.join(name);
        std::fs::create_dir_all(&staging)?;
        std::fs::write(staging.join(agenium_skills::SKILL_FILE), &text)?;
        if let Err(e) = agenium_skills::validate(&staging, Origin::Agent) {
            let _ = std::fs::remove_dir_all(&staging_root);
            return refuse(format!("the new skill is not valid: {e}"));
        }
        std::fs::rename(&staging, own_dir.join(name))?;
        let _ = std::fs::remove_dir_all(&staging_root);

        created.insert(
            name.to_owned(),
            CreatedSkill {
                lesson: u.lesson,
                lesson_text: lesson.text.clone(),
                reason: u.reason.trim().to_owned(),
                at: now_iso(),
                flagged: None,
            },
        );
        save(&mem, &created)?;
        self.store().commit(None, &format!("agent:{agent}"), |w| {
            w.emit(DomainEvent::AgentUpdated {
                agent,
                field: format!("skill_created:{name}"),
            })
            .map_err(OrchError::from)
        })?;
        Ok(serde_json::json!({
            "status": "created",
            "skill": name,
            "note": "your own new skill (other agents do not get it). It is in your skills index from your next run; the user can review or delete it, and it is flagged if its lesson stops holding",
        }))
    }

    /// Anti-drift (ADR-91): flags the created skills whose lesson no longer holds — deleted, retired or replaced, or
    /// its confidence fell below [`DOUBTFUL_CONFIDENCE`] after reopens — and asks the user once to review them.
    /// Returns the newly flagged ones.
    pub fn audit_created_skills(&self, agent: AgentId) -> Result<Vec<String>> {
        let mem = self.agent_memory_store(agent)?;
        let mut created = load(&mem)?;
        let mut newly = Vec::new();
        for (name, c) in created.iter_mut() {
            if c.flagged.is_some() {
                continue;
            }
            let why = match mem.get(c.lesson) {
                Err(_) => Some("its lesson was deleted"),
                Ok(f) if !f.is_live() => Some("its lesson was retired or replaced"),
                Ok(f) if f.confidence < DOUBTFUL_CONFIDENCE => {
                    Some("its lesson lost confidence after reopens")
                }
                _ => None,
            };
            if let Some(w) = why {
                c.flagged = Some(format!("{w} ({})", now_iso()));
                newly.push(format!("{name}: {w}"));
            }
        }
        if newly.is_empty() {
            return Ok(newly);
        }
        save(&mem, &created)?;
        let who = self.agent(agent)?.name;
        self.store().commit(None, "system", |w| {
            w.emit(DomainEvent::NeedsUser {
                ticket: None,
                reason: format!(
                    "{who} has skills it created whose lesson no longer holds — review or delete them: {}",
                    newly.join("; ")
                ),
            })
            .map_err(OrchError::from)
        })?;
        Ok(newly)
    }

    /// The user deletes a skill the agent created (an overlay of an existing skill is reverted instead).
    pub fn remove_created_skill(&self, agent: AgentId, name: &str) -> Result<()> {
        let mem = self.agent_memory_store(agent)?;
        let mut created = load(&mem)?;
        if created.remove(name).is_none() {
            return refuse(format!(
                "{name} is not a skill this agent created (an overlay is reverted instead)"
            ));
        }
        let dir = self
            .skill_registry()
            .dir_of(&Owner::Agent(agent.to_string()))
            .join(name);
        if dir.is_dir() {
            std::fs::remove_dir_all(&dir)?;
        }
        save(&mem, &created)?;
        self.store().commit(None, "user", |w| {
            w.emit(DomainEvent::AgentUpdated {
                agent,
                field: format!("skill_removed:{name}"),
            })
            .map_err(OrchError::from)
        })?;
        Ok(())
    }
}
