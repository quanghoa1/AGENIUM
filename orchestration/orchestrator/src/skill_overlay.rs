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
pub(crate) const INJECTION: &[&str] = &[
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
    /// The base SKILL.md changed since this overlay was built, so the diff mixes base changes with the agent's;
    /// `rebase_skill_overlays` can 3-way merge it onto the new base (ADR-86).
    pub stale: bool,
}

/// The result of rebasing one overlay onto its updated base (ADR-86).
#[derive(Debug, Clone, Serialize)]
pub struct RebaseOutcome {
    pub skill: String,
    pub from_version: String,
    pub to_version: String,
    /// `up-to-date`, `rebased`, `conflict`, or `no-base`.
    pub status: String,
    /// On a conflict: the merged text with `<<<<<<<` markers, for the user to resolve (the overlay is left unchanged).
    pub conflict_preview: Option<String>,
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

/// Reverse-applies `patch` (a base -> overlay diff) to `overlay`, reconstructing the base it was built on.
fn unapply_patch(overlay: &str, patch: &str) -> Result<String> {
    let dir = tempfile::tempdir().map_err(io)?;
    std::fs::write(dir.path().join("SKILL.md"), overlay).map_err(io)?;
    let patch_file = dir.path().join("change.patch");
    let mut p = patch.replace("\r\n", "\n");
    if !p.ends_with('\n') {
        p.push('\n');
    }
    std::fs::write(&patch_file, p).map_err(io)?;
    let mut last = String::new();
    // `diff_texts` writes under a/ and b/ subdirs, so git's own a/ b/ prefix makes the paths a/a/SKILL.md, b/b/SKILL.md
    // (strip 2); -p1/-p0 cover a patch stored in another shape.
    for strip in ["-p2", "-p1", "-p0"] {
        let out = std::process::Command::new("git")
            .args(["-c", "core.autocrlf=false", "-c", "core.eol=lf"])
            .args([
                "apply",
                "--reverse",
                "--recount",
                "--whitespace=nowarn",
                strip,
            ])
            .arg(&patch_file)
            .current_dir(dir.path())
            .output()
            .map_err(io)?;
        if out.status.success() {
            let t = std::fs::read_to_string(dir.path().join("SKILL.md")).map_err(io)?;
            return Ok(t.replace("\r\n", "\n"));
        }
        last = String::from_utf8_lossy(&out.stderr).trim().to_owned();
    }
    Err(invalid(format!(
        "cannot reconstruct the base the overlay was built on: {last}"
    )))
}

/// 3-way merge (ADR-86): start from `new_base` and fold in the agent's `old_base` -> `overlay` changes. Returns the
/// merged text and whether it carries conflict markers. `git merge-file` needs no repository.
fn merge_three(new_base: &str, old_base: &str, overlay: &str) -> Result<(String, bool)> {
    let dir = tempfile::tempdir().map_err(io)?;
    let cur = dir.path().join("current");
    let anc = dir.path().join("ancestor");
    let oth = dir.path().join("other");
    std::fs::write(&cur, new_base.replace("\r\n", "\n")).map_err(io)?;
    std::fs::write(&anc, old_base.replace("\r\n", "\n")).map_err(io)?;
    std::fs::write(&oth, overlay.replace("\r\n", "\n")).map_err(io)?;
    let out = std::process::Command::new("git")
        .args([
            "-c",
            "core.autocrlf=false",
            "-c",
            "core.eol=lf",
            "merge-file",
            "-p",
            "--diff3",
            "-L",
            "new base",
            "-L",
            "previous base",
            "-L",
            "your overlay",
        ])
        .arg(&cur)
        .arg(&anc)
        .arg(&oth)
        .output()
        .map_err(io)?;
    // Exit status: 0 = clean, > 0 = number of conflicts, < 0 = error. Treat a non-zero code as a conflict; a real git
    // error leaves the merged text with markers too, which the caller surfaces for the user.
    let merged = String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n");
    let conflict = out.status.code().unwrap_or(1) != 0;
    Ok((merged, conflict))
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

    pub(crate) fn agent_memory_store(
        &self,
        agent: AgentId,
    ) -> Result<std::sync::Arc<agenium_store::AgentStore>> {
        let e = self.embedder();
        self.agent_store(agent, e.dim(), e.model_id())
    }

    /// The SKILL.md an update would produce, without writing anything: (current, updated). Lets the judge read a change
    /// for contradictions before it is applied (ADR-92).
    pub fn preview_skill_update(
        &self,
        agent: AgentId,
        u: &ProposeSkillUpdate,
    ) -> Result<(String, String)> {
        let a = self.agent(agent)?;
        let name = u.skill.trim();
        let base = self
            .base_skill(&a, name)
            .ok_or_else(|| invalid(format!("{name:?} is not a role or library skill of yours")))?;
        let dir = self.overlay_dir(agent, name);
        let path = if dir.join("SKILL.md").is_file() {
            dir.join("SKILL.md")
        } else {
            base.meta.dir.join("SKILL.md")
        };
        let current = std::fs::read_to_string(path).map_err(io)?;
        let updated = apply_patch(&current, &u.patch)?;
        Ok((current, updated))
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
        // ADR-92: like a new skill, a lesson that came from external content never changes a skill.
        if lesson.untrusted {
            return Err(invalid(format!(
                "lesson #{} came from external content (a page, a file, a tool output): it cannot change a skill",
                u.lesson
            )));
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
        // ADR-92: the change must not introduce an explicit contradiction with another skill the agent uses (an
        // inconsistency that was already there before the change does not block it).
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
            return Err(invalid(format!(
                "the change {}; reconcile the two skills first (or improve the other one)",
                introduced.join("; ")
            )));
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
        // ADR-88: the same skill-shape checks the user sees when authoring also run on an agent's self-upgrade, as
        // advice recorded with the change (never blocking).
        let notes = self.lint_skill(name, &crate::skill_shape::strip_frontmatter(&updated));
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
                "shape_notes": notes,
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
            // Stale when the base the overlay was built on (reconstructed by reverse-applying the stored patch) no
            // longer matches the current base — i.e. the base SKILL.md changed under the overlay.
            let stale = base.is_some()
                && unapply_patch(&overlay_text, &o.patch)
                    .map(|old| old != base_text)
                    .unwrap_or(false);
            out.push(OverlayView {
                diff: diff_texts(&base_text, &overlay_text)?,
                base: match base.map(|b| b.meta.origin) {
                    Some(Origin::Library) => "library".into(),
                    _ => "role".into(),
                },
                skill: o.skill,
                base_version: o.base_version,
                changelog: o.changelog,
                stale,
            });
        }
        Ok(out)
    }

    /// Rebases every stale overlay of `agent` onto its updated base (ADR-86): reconstructs the base each overlay was
    /// built on, 3-way merges the agent's change onto the current base, and — when the merge is clean — swaps the
    /// overlay in and records a `rebased_onto` changelog entry. A conflicting overlay is left untouched and reported
    /// for the user to resolve (revert or re-propose), with one Needs-you.
    pub fn rebase_skill_overlays(&self, agent: AgentId) -> Result<Vec<RebaseOutcome>> {
        let a = self.agent(agent)?;
        let mem = self.agent_memory_store(agent)?;
        let mut out = Vec::new();
        let mut conflicts = Vec::new();
        for o in mem.overlays()? {
            let outcome = self.rebase_one(agent, &a, mem.as_ref(), &o)?;
            if outcome.status == "conflict" {
                conflicts.push(outcome.skill.clone());
            }
            out.push(outcome);
        }
        if !conflicts.is_empty() {
            self.store.commit(None, &format!("agent:{agent}"), |w| {
                w.emit(DomainEvent::NeedsUser {
                    ticket: None,
                    reason: format!(
                        "skill overlay(s) {} could not be rebased onto the updated base automatically; review and revert or re-propose them",
                        conflicts.join(", ")
                    ),
                })
                .map_err(OrchError::from)
            })?;
        }
        Ok(out)
    }

    fn rebase_one(
        &self,
        agent: AgentId,
        a: &agenium_store::core::agents::Agent,
        mem: &agenium_store::AgentStore,
        o: &agenium_store::agent::OverlayRow,
    ) -> Result<RebaseOutcome> {
        let mut outcome = RebaseOutcome {
            skill: o.skill.clone(),
            from_version: o.base_version.clone(),
            to_version: o.base_version.clone(),
            status: "up-to-date".into(),
            conflict_preview: None,
        };
        let Some(base) = self.base_skill(a, &o.skill) else {
            outcome.status = "no-base".into();
            return Ok(outcome);
        };
        let new_version = base
            .meta
            .version
            .clone()
            .unwrap_or_else(|| "unversioned".into());
        outcome.to_version = new_version.clone();
        let new_base = std::fs::read_to_string(base.meta.dir.join("SKILL.md")).map_err(io)?;
        let dir = self.overlay_dir(agent, &o.skill);
        let overlay = std::fs::read_to_string(dir.join("SKILL.md")).map_err(io)?;
        let old_base = unapply_patch(&overlay, &o.patch)?;
        if old_base == new_base {
            // The base is unchanged; only refresh the recorded version if its label drifted.
            if o.base_version != new_version {
                mem.put_overlay(&o.skill, &new_version, &o.patch, serde_json::json!({}))?;
            }
            return Ok(outcome);
        }
        let (merged, conflict) = merge_three(&new_base, &old_base, &overlay)?;
        if conflict {
            outcome.status = "conflict".into();
            outcome.conflict_preview = Some(merged);
            return Ok(outcome);
        }
        // Clean merge: validate in a staging copy, then swap it in (as apply_skill_update does).
        let staging_root = dir.with_file_name(format!(".rebase-{}", ulid::Ulid::new()));
        let staging = staging_root.join(&o.skill);
        copy_dir(&dir, &staging).map_err(io)?;
        std::fs::write(staging.join("SKILL.md"), &merged).map_err(io)?;
        if let Err(e) = agenium_skills::validate(&staging, Origin::Overlay) {
            let _ = std::fs::remove_dir_all(&staging_root);
            outcome.status = "conflict".into();
            outcome.conflict_preview =
                Some(format!("the rebased skill is not valid: {e}\n\n{merged}"));
            return Ok(outcome);
        }
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::rename(&staging, &dir).map_err(io)?;
        let _ = std::fs::remove_dir_all(&staging_root);
        let cumulative = diff_texts(&new_base, &merged)?;
        mem.put_overlay(
            &o.skill,
            &new_version,
            &cumulative,
            serde_json::json!({
                "at": now_iso(),
                "rebased_onto": new_version,
                "from_version": o.base_version,
            }),
        )?;
        self.store.commit(None, &format!("agent:{agent}"), |w| {
            w.emit(DomainEvent::AgentUpdated {
                agent,
                field: format!("skill_overlay_rebased:{}", o.skill),
            })
            .map_err(OrchError::from)
        })?;
        outcome.status = "rebased".into();
        Ok(outcome)
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

#[cfg(test)]
mod tests {
    use super::{diff_texts, merge_three, unapply_patch};

    const BASE: &str = "---\nname: demo\ndescription: a demo skill\n---\n\n# Demo\n\nStep 1: read the file.\nStep 2: write the file.\n";

    #[test]
    fn unapply_reconstructs_the_base_the_overlay_was_built_on() {
        let overlay = BASE.replace(
            "Step 2: write the file.",
            "Step 2: write the file.\nStep 3: run the tests.",
        );
        let patch = diff_texts(BASE, &overlay).unwrap();
        assert_eq!(unapply_patch(&overlay, &patch).unwrap(), BASE);
    }

    #[test]
    fn a_non_conflicting_base_change_rebases_cleanly() {
        // Agent added Step 3 at the end; the base later added a note at the top. Disjoint regions -> clean merge.
        let overlay = BASE.replace(
            "Step 2: write the file.",
            "Step 2: write the file.\nStep 3: run the tests.",
        );
        let new_base = BASE.replace(
            "# Demo\n",
            "# Demo\n\nNote: always format before committing.\n",
        );
        let (merged, conflict) = merge_three(&new_base, BASE, &overlay).unwrap();
        assert!(!conflict, "disjoint edits should not conflict:\n{merged}");
        assert!(
            merged.contains("Step 3: run the tests."),
            "keeps the overlay's change"
        );
        assert!(
            merged.contains("always format before committing"),
            "keeps the base's change"
        );
    }

    #[test]
    fn an_overlapping_base_change_reports_a_conflict() {
        // Both the base and the overlay rewrite the same line.
        let overlay = BASE.replace(
            "Step 2: write the file.",
            "Step 2: write the file carefully.",
        );
        let new_base = BASE.replace(
            "Step 2: write the file.",
            "Step 2: write the file atomically.",
        );
        let (merged, conflict) = merge_three(&new_base, BASE, &overlay).unwrap();
        assert!(conflict, "overlapping edits should conflict");
        assert!(
            merged.contains("<<<<<<<"),
            "carries conflict markers:\n{merged}"
        );
    }
}
