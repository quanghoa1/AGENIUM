//! Skill overlays (decision 35): an agent writes a lesson that held in two contexts into its own copy of a skill
//! (`propose_skill_update`), copy-on-write in `<root>/agents/<id>/skills/<name>/`. The base stays unchanged and other
//! agents keep using it. Each update is checked (the lesson, the patch, the skill's frontmatter, secrets, injected
//! instructions), recorded with a changelog in the agent's memory database and in the audit trail, and applied
//! without asking the user; the user sees the diff and can revert the overlay or promote it to the base.

use std::path::{Path, PathBuf};

use agenium_core::forms::ProposeSkillUpdate;
use agenium_core::*;
use agenium_skills::{Origin, Owner, Skill};
use agenium_store::FactKind;
use serde::Serialize;

use crate::app::App;
use crate::{OrchError, Result};

/// Validated contexts a lesson needs before it may change a skill (decision 35).
pub const MIN_CONTEXTS: usize = 2;

/// Phrases that turn a skill into someone else's instructions (prompt injection); an update carrying one is refused.
const INJECTION: &[&str] = &[
    "ignore previous instructions",
    "ignore all previous",
    "disregard the above",
    "disregard your instructions",
    "you are now",
    "system prompt",
    "| sh",
    "| bash",
    "iex(",
    "invoke-expression",
];

/// What the user sees of an agent's overlay.
#[derive(Debug, Clone, Serialize)]
pub struct OverlayView {
    pub skill: String,
    pub base_version: String,
    /// Diff from the base SKILL.md to the overlay's.
    pub diff: String,
    pub changelog: Vec<serde_json::Value>,
    /// Where the base lives: `role` or `library`.
    pub base: String,
}

fn invalid(m: impl Into<String>) -> OrchError {
    OrchError::Invalid(m.into())
}

fn io(e: std::io::Error) -> OrchError {
    OrchError::Invalid(e.to_string())
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        let target = to.join(e.file_name());
        if e.file_type()?.is_dir() {
            copy_dir(&e.path(), &target)?;
        } else {
            std::fs::copy(e.path(), target)?;
        }
    }
    Ok(())
}

/// Applies a unified diff to `text` with `git apply` in a scratch folder (`a/`/`b/` prefixes or none).
fn apply_patch(text: &str, patch: &str) -> Result<String> {
    let dir = tempfile::tempdir().map_err(io)?;
    std::fs::write(dir.path().join("SKILL.md"), text).map_err(io)?;
    let patch_file = dir.path().join("change.patch");
    let mut p = patch.replace("\r\n", "\n");
    if !p.ends_with('\n') {
        p.push('\n');
    }
    std::fs::write(&patch_file, p).map_err(io)?;
    let mut last = String::new();
    for strip in ["-p1", "-p0"] {
        let out = std::process::Command::new("git")
            .args(["apply", "--recount", "--whitespace=nowarn", strip])
            .arg(&patch_file)
            .current_dir(dir.path())
            .output()
            .map_err(io)?;
        if out.status.success() {
            return std::fs::read_to_string(dir.path().join("SKILL.md")).map_err(io);
        }
        last = String::from_utf8_lossy(&out.stderr).trim().to_owned();
    }
    Err(invalid(format!(
        "the patch does not apply to the current SKILL.md: {last}"
    )))
}

/// Unified diff `a/SKILL.md` -> `b/SKILL.md` (git, no repository needed).
fn diff_texts(base: &str, overlay: &str) -> Result<String> {
    let dir = tempfile::tempdir().map_err(io)?;
    let (a, b) = (dir.path().join("a"), dir.path().join("b"));
    for (d, t) in [(&a, base), (&b, overlay)] {
        std::fs::create_dir_all(d).map_err(io)?;
        std::fs::write(d.join("SKILL.md"), t).map_err(io)?;
    }
    let out = std::process::Command::new("git")
        .args([
            "diff",
            "--no-index",
            "--no-color",
            "a/SKILL.md",
            "b/SKILL.md",
        ])
        .current_dir(dir.path())
        .output()
        .map_err(io)?;
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

impl App {
    /// The base version of an agent's skill `name`: its role's skill, else the library's.
    fn base_skill(&self, agent: &agenium_store::core::agents::Agent, name: &str) -> Option<Skill> {
        let reg = self.skill_registry();
        let base = reg.for_agent(".base", &agent.role, &[name.to_owned()]);
        base.get(name).cloned()
    }

    fn overlay_dir(&self, agent: AgentId, name: &str) -> PathBuf {
        self.skill_registry()
            .dir_of(&Owner::Agent(agent.to_string()))
            .join(name)
    }

    fn agent_memory_store(
        &self,
        agent: AgentId,
    ) -> Result<std::sync::Arc<agenium_store::AgentStore>> {
        let e = self.embedder();
        self.agent_store(agent, e.dim(), e.model_id())
    }

    /// `propose_skill_update` of `agent`: checks and applies the change to its own overlay (decision 35).
    pub fn apply_skill_update(
        &self,
        agent: AgentId,
        u: &ProposeSkillUpdate,
    ) -> Result<serde_json::Value> {
        let a = self.agent(agent)?;
        let name = u.skill.trim();
        let base = self.base_skill(&a, name).ok_or_else(|| {
            invalid(format!(
                "{name:?} is not a role or library skill of yours: only those have overlays (write your own skills in your skills folder)"
            ))
        })?;
        let mem = self.agent_memory_store(agent)?;
        let lesson = mem
            .get(u.lesson)
            .map_err(|_| invalid(format!("lesson #{} is not in your memory", u.lesson)))?;
        if lesson.kind != FactKind::Lesson || !lesson.is_live() {
            return Err(invalid(format!("#{} is not a live lesson", u.lesson)));
        }
        if lesson.validated.len() < MIN_CONTEXTS {
            return Err(invalid(format!(
                "lesson #{} held in {} context(s); a skill changes only after a lesson held in {MIN_CONTEXTS}",
                u.lesson,
                lesson.validated.len()
            )));
        }
        if u.reason.trim().is_empty() {
            return Err(invalid("give the reason for the change"));
        }
        let dir = self.overlay_dir(agent, name);
        let fresh = !dir.join("SKILL.md").is_file();
        let current = if fresh {
            base.meta.dir.join("SKILL.md")
        } else {
            dir.join("SKILL.md")
        };
        let current = std::fs::read_to_string(current).map_err(io)?;
        let updated = apply_patch(&current, &u.patch)?;
        let lower = updated.to_lowercase();
        if let Some(p) = INJECTION
            .iter()
            .find(|p| lower.contains(*p) && !current.to_lowercase().contains(*p))
        {
            return Err(invalid(format!(
                "the change adds {p:?}, which reads as injected instructions: refused"
            )));
        }
        if !agenium_vcs::scan_diff(&u.patch).is_empty() || self.redactor.redact(&updated) != updated
        {
            return Err(invalid(
                "the change carries something that looks like a secret: refused",
            ));
        }
        // Write into a staging copy, validate the skill, then swap it in.
        // (The staging folder carries the skill's name: a skill's name must equal its folder's.)
        let staging_root = dir.with_file_name(format!(".update-{}", ulid::Ulid::new()));
        let staging = staging_root.join(name);
        copy_dir(if fresh { &base.meta.dir } else { &dir }, &staging).map_err(io)?;
        std::fs::write(staging.join("SKILL.md"), &updated).map_err(io)?;
        if let Err(e) = agenium_skills::validate(&staging, Origin::Overlay) {
            let _ = std::fs::remove_dir_all(&staging_root);
            return Err(invalid(format!("the updated skill is not valid: {e}")));
        }
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::rename(&staging, &dir).map_err(io)?;
        let _ = std::fs::remove_dir_all(&staging_root);
        let base_text = std::fs::read_to_string(base.meta.dir.join("SKILL.md")).map_err(io)?;
        let cumulative = diff_texts(&base_text, &updated)?;
        mem.put_overlay(
            name,
            base.meta.version.as_deref().unwrap_or("unversioned"),
            &cumulative,
            serde_json::json!({
                "at": now_iso(),
                "reason": u.reason.trim(),
                "lesson": u.lesson,
                "lesson_text": lesson.text,
                "validated": lesson.validated,
                "patch": u.patch,
            }),
        )?;
        self.store.commit(None, &format!("agent:{agent}"), |w| {
            w.emit(DomainEvent::AgentUpdated {
                agent,
                field: format!("skill_overlay:{name}"),
            })
            .map_err(OrchError::from)
        })?;
        Ok(serde_json::json!({
            "status": "applied",
            "skill": name,
            "note": "your own copy of the skill is updated; other agents keep the base, and the user can review or revert it",
        }))
    }

    /// The agent's overlays with their diff against the base.
    pub fn skill_overlays(&self, agent: AgentId) -> Result<Vec<OverlayView>> {
        let a = self.agent(agent)?;
        let mem = self.agent_memory_store(agent)?;
        let mut out = Vec::new();
        for o in mem.overlays()? {
            let base = self.base_skill(&a, &o.skill);
            let base_text = base
                .as_ref()
                .and_then(|b| std::fs::read_to_string(b.meta.dir.join("SKILL.md")).ok())
                .unwrap_or_default();
            let overlay_text =
                std::fs::read_to_string(self.overlay_dir(agent, &o.skill).join("SKILL.md"))
                    .unwrap_or_default();
            out.push(OverlayView {
                diff: diff_texts(&base_text, &overlay_text)?,
                base: match base.map(|b| b.meta.origin) {
                    Some(Origin::Library) => "library".into(),
                    _ => "role".into(),
                },
                skill: o.skill,
                base_version: o.base_version,
                changelog: o.changelog,
            });
        }
        Ok(out)
    }

    /// The user reverts an overlay: the agent uses the base again (the changelog goes with it).
    pub fn revert_skill_overlay(&self, agent: AgentId, skill: &str) -> Result<()> {
        let dir = self.overlay_dir(agent, skill);
        if dir.is_dir() {
            std::fs::remove_dir_all(&dir).map_err(io)?;
        }
        self.agent_memory_store(agent)?.remove_overlay(skill)?;
        self.store.commit(None, "user", |w| {
            w.emit(DomainEvent::AgentUpdated {
                agent,
                field: format!("skill_overlay_reverted:{skill}"),
            })
            .map_err(OrchError::from)
        })?;
        Ok(())
    }

    /// The user promotes an overlay to the base (the role's or the library's skill), which every agent then uses;
    /// the overlay is dropped.
    pub fn promote_skill_overlay(&self, agent: AgentId, skill: &str) -> Result<()> {
        let a = self.agent(agent)?;
        let base = self
            .base_skill(&a, skill)
            .ok_or_else(|| OrchError::NotFound(format!("base skill {skill}")))?;
        let dir = self.overlay_dir(agent, skill);
        let text = std::fs::read_to_string(dir.join("SKILL.md"))
            .map_err(|_| OrchError::NotFound(format!("overlay {skill}")))?;
        std::fs::write(base.meta.dir.join("SKILL.md"), text).map_err(io)?;
        std::fs::remove_dir_all(&dir).map_err(io)?;
        self.agent_memory_store(agent)?.remove_overlay(skill)?;
        self.store.commit(None, "user", |w| {
            w.emit(DomainEvent::AgentUpdated {
                agent,
                field: format!("skill_promoted:{skill}"),
            })
            .map_err(OrchError::from)
        })?;
        Ok(())
    }
}
