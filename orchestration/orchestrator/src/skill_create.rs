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
//! - **accountable, anti-drift**: every creation is audited and listed for the user, who can delete it; when a
//!   lesson it stands on (the one it was made from, or a revision's) stops holding — deleted, retired, or its
//!   confidence falls after reopens; a refinement of it carries on — the skill is flagged and the user is asked to
//!   review it. One lesson, one skill holds along the lesson's line of refinements.

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

/// Skill proposals of one agent that may wait for the user at once (ADR-152).
pub const MAX_PENDING_PROPOSALS: usize = 5;

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
    /// Later lessons written into it (ADR-146).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub revisions: Vec<Revision>,
}

/// A lesson written into a skill the agent created, after it was made (ADR-146).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Revision {
    pub lesson: i64,
    pub lesson_text: String,
    pub reason: String,
    pub at: String,
}

fn refuse<T>(m: impl Into<String>) -> Result<T> {
    Err(OrchError::Invalid(m.into()))
}

/// The live end of lesson `id`'s line: a refinement (UPDATE) supersedes a lesson with a new one, which carries its
/// validated contexts; a retirement supersedes it with itself. `None` when the line ends retired or deleted.
pub(crate) fn live_end(mem: &AgentStore, id: i64) -> Option<agenium_store::Fact> {
    let mut at = id;
    for _ in 0..64 {
        let f = mem.get(at).ok()?;
        match f.superseded_by {
            None => return Some(f),
            Some(next) if next == at => return None,
            Some(next) => at = next,
        }
    }
    None
}

impl CreatedSkill {
    /// The lessons the skill stands on: the one it was made from and each revision's.
    pub(crate) fn lessons(&self) -> impl Iterator<Item = i64> + '_ {
        std::iter::once(self.lesson).chain(self.revisions.iter().map(|r| r.lesson))
    }
}

fn load(mem: &AgentStore) -> Result<BTreeMap<String, CreatedSkill>> {
    Ok(mem
        .meta(META_KEY)?
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default())
}

pub(crate) fn save_created(mem: &AgentStore, map: &BTreeMap<String, CreatedSkill>) -> Result<()> {
    save(mem, map)
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
        let created = load(&mem)?;
        // One lesson, one skill, along the lesson's line: a refinement gets a new id and keeps the contexts it held
        // in, and made a second skill of the same lesson before (audit M16, 2026-10-31).
        if let Some((n, _)) = created.iter().find(|(_, c)| {
            c.lessons()
                .any(|l| l == u.lesson || live_end(&mem, l).is_some_and(|f| f.id == u.lesson))
        }) {
            return refuse(format!(
                "lesson #{} already became skill {n} (or a lesson it was refined from did): improve that one with \
                 propose_skill_update",
                u.lesson
            ));
        }

        // The same along proposals (ADR-152): a lesson proposed once, pending or saved, is not proposed again.
        let proposed = self
            .store()
            .read(|c| agenium_store::core::skill_proposals::of_agent(c, agent))?;
        if let Some(p) = proposed.iter().find(|p| {
            p.lesson == u.lesson || live_end(&mem, p.lesson).is_some_and(|f| f.id == u.lesson)
        }) {
            return refuse(format!(
                "lesson #{} is already in your skill proposal {} (waiting for the user, or saved into the library): improve that skill with propose_skill_update once it is yours",
                u.lesson, p.name
            ));
        }
        if proposed.iter().any(|p| p.name == name) {
            return refuse(format!("you already proposed a skill named {name}"));
        }
        let pending = proposed
            .iter()
            .filter(|p| p.status == agenium_store::core::roles::ProposalStatus::Pending)
            .count();
        if pending >= MAX_PENDING_PROPOSALS {
            return refuse(format!(
                "{pending} of your skill proposals already wait for the user: wait for their decision"
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
        if created.len() + proposed.len() >= MAX_CREATED_SKILLS {
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

        // Well-formed and not a copy of a skill the agent has: the full shape check must come back clean. A library skill
        // it does not have is not its to improve: closeness to one is shown to the user with the proposal (ADR-152).
        let others: Vec<(String, String)> = mine
            .skills
            .iter()
            .map(|s| (s.meta.name.clone(), s.body.clone()))
            .collect();
        let notes = crate::skill_shape::lint_skill(name, body, &others);
        if !notes.is_empty() {
            return refuse(format!("the new skill is not ready: {}", notes.join("; ")));
        }

        // No contradiction (ADR-92). One subject, one skill: a subject a skill the agent has already covers must be
        // improved in place (its own copy), so two skills about the same thing never drift apart; an explicit opposite
        // directive against such a skill is refused. Against a library skill the agent does not have, neither is the
        // agent's to settle: the proposal carries it for the user (or the skill writer) to merge or reconcile.
        let has: std::collections::HashSet<String> =
            mine.skills.iter().map(|s| s.meta.name.clone()).collect();
        let (known, elsewhere): (Vec<_>, Vec<_>) = self
            .known_skills(Some(&a), name)
            .into_iter()
            .partition(|k| has.contains(&k.name));
        let subjects: Vec<(String, String)> = known
            .iter()
            .map(|k| (k.name.clone(), k.subject.clone()))
            .collect();
        let me = crate::skill_conflict::subject(description, body);
        if let Some((other, score, how)) =
            crate::skill_conflict::same_subject(&me, &subjects, Some(self.embedder().as_ref()))
        {
            return refuse(format!(
                "skill {other} already covers this subject (same {how}, {:.0}%): improve it with propose_skill_update instead of adding a second skill about the same thing",
                score * 100.0
            ));
        }
        let guidance = |ks: &[crate::skill_conflict::Known]| -> Vec<(String, String)> {
            ks.iter()
                .map(|k| (k.name.clone(), k.guidance.clone()))
                .collect()
        };
        let conflicts = crate::skill_conflict::directive_conflicts(name, body, &guidance(&known));
        if !conflicts.is_empty() {
            return refuse(format!(
                "the new skill {}; reconcile it with that skill (or improve that skill) first",
                conflicts.join("; ")
            ));
        }
        let similar: Vec<crate::skill_similar::SimilarSkill> = self
            .similar_skills(description, body, Some(name), 3)
            .into_iter()
            .filter(|x| !has.contains(&x.name))
            .collect();
        let proposal_notes =
            crate::skill_conflict::directive_conflicts(name, body, &guidance(&elsewhere));

        // Valid as a skill (checked in a staging folder that is removed), then proposed to the user (ADR-152): the
        // library is the one place skills live; the agent never adds one on its own.
        let staging_root =
            std::env::temp_dir().join(format!("agenium-skill-proposal-{}", ulid::Ulid::new()));
        let staging = staging_root.join(name);
        std::fs::create_dir_all(&staging)?;
        std::fs::write(staging.join(agenium_skills::SKILL_FILE), &text)?;
        let valid = agenium_skills::validate(&staging, Origin::Agent);
        let _ = std::fs::remove_dir_all(&staging_root);
        if let Err(e) = valid {
            return refuse(format!("the new skill is not valid: {e}"));
        }
        let project = match a.team {
            Some(t) => self.team(t)?.0.project,
            None => None,
        };
        let proposal = agenium_store::core::skill_proposals::SkillProposal {
            id: ulid::Ulid::new().to_string(),
            project,
            agent,
            name: name.to_owned(),
            text,
            lesson: u.lesson,
            lesson_text: lesson.text.clone(),
            contexts: self.evidenced_contexts(&lesson),
            similar: serde_json::to_value(&similar).unwrap_or_default(),
            notes: proposal_notes,
            reason: u.reason.trim().to_owned(),
            status: agenium_store::core::roles::ProposalStatus::Pending,
            skill: None,
            note: None,
            created_at: now_iso(),
            decided_at: None,
        };
        self.store().commit(None, &format!("agent:{agent}"), |w| {
            agenium_store::core::skill_proposals::insert(w.tx, &proposal)?;
            w.emit(DomainEvent::NeedsUser {
                ticket: None,
                reason: format!("{} proposes a new skill {name}: {description}", a.name),
            })
            .map_err(OrchError::from)
        })?;
        Ok(serde_json::json!({
            "status": "proposed",
            "skill": name,
            "note": "proposed to the user: if they save it, it goes into the skill library (every agent may enable it) and is switched on for you; until then you do not have it",
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
            // Every lesson the skill stands on, each where its line is now: a refined lesson still holds; one retired,
            // deleted or doubtful does not. A revision's lesson counted for nothing before (audit M16, 2026-10-31).
            let why = c.lessons().find_map(|l| match live_end(&mem, l) {
                None => Some("a lesson it stands on was retired or deleted"),
                Some(f) if f.confidence < DOUBTFUL_CONFIDENCE => {
                    Some("a lesson it stands on lost confidence after reopens")
                }
                Some(_) => None,
            });
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
