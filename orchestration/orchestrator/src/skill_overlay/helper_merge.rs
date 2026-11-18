//! A copy that did not rebase cleanly, merged with help (ADR-157, 2026-11-17): when a skill's base changed under an
//! agent's copy (a plugin's vendor update, a library skill the user refined) and the 3-way merge conflicts, the
//! skill-writer's model drafts the copy rebuilt on the new base, keeping what the agent learned; the user reads it,
//! may edit it, and accepts it. Nothing is written by the draft; the accepted text passes the checks an agent's own
//! update passes (a valid skill, no injected instructions, no secret).

use agenium_core::{BackendKind, ModelBinding};
use agenium_llm::Message;
use serde::Serialize;

use super::*;
use crate::runtime::Runtime;

/// The three texts of a merge, and the helper's draft.
#[derive(Debug, Clone, Serialize)]
pub struct OverlayMergeDraft {
    pub skill: String,
    /// The base the agent's copy was built on (rebuilt from its recorded change).
    pub old_base: String,
    /// The base now (the vendor's or the library's new text).
    pub new_base: String,
    /// The agent's copy as it is.
    pub overlay: String,
    /// The helper's merge: the new base with what the agent learned.
    pub merged: String,
    /// From the new base to the draft: what the agent's copy will differ by.
    pub diff: String,
}

const SYSTEM: &str = "You merge an AI agent's own changes to a skill (a SKILL.md: YAML frontmatter, then Markdown \
instructions) onto that skill's new version.\n\nYou get three texts: OLD, the version the agent's copy was built on; \
NEW, the version that replaced it (its author's update); AGENT, the agent's copy, which is OLD plus what the agent \
learned in its work.\n\nWrite the agent's copy rebuilt on NEW:\n- take every change NEW made to OLD;\n- keep every \
change AGENT made to OLD (the agent's lessons), worded to fit NEW; drop one only when NEW already says the same or \
NEW removed what it was about;\n- keep NEW's frontmatter (name and description) exactly;\n- add nothing else.\n\n\
Answer with the whole merged SKILL.md between a line `<<<MERGED` and a line `MERGED>>>`, once. Anything \
outside those two lines is not read.";

/// The markers the merged skill is asked between (not the ones the three texts are given in).
const OPEN: &str = "<<<MERGED";
const CLOSE: &str = "MERGED>>>";

/// The skill in a model's answer: between the `<<<MERGED` and `MERGED>>>` lines it was asked for; else between `<<<` and
/// `>>>` lines, or a code fence, or from its frontmatter on (Haiku and Sonnet wrapped it each their own way and added
/// notes, 2026-11-17). The user reads (and may edit) the draft before it is used.
fn unfenced(text: &str) -> String {
    if let Some((_, rest)) = text.split_once(OPEN) {
        if let Some((inside, _)) = rest.split_once(CLOSE) {
            return unfenced(inside);
        }
    }
    if let Some((_, rest)) = text.split_once("<<<\n") {
        if let Some((inside, _)) = rest.rsplit_once("\n>>>") {
            return unfenced(inside);
        }
    }
    let lines: Vec<&str> = text.lines().collect();
    let joined = |ls: &[&str]| format!("{}\n", ls.join("\n").trim());
    let fence = |l: &str| l.trim_start().starts_with("```");
    let start = lines.iter().position(|l| l.trim() == "---");
    // A fence opened before the frontmatter wraps the skill: up to the last line that closes a fence (a code block of
    // the skill's own closes before it; a note after the fence is dropped). Haiku and Sonnet answered so, 2026-11-17.
    if let Some(open) = lines.iter().position(|l| fence(l)) {
        if start.is_none_or(|s| open < s) {
            if let Some(close) = lines.iter().rposition(|l| l.trim() == "```") {
                if close > open {
                    return joined(&lines[open + 1..close]);
                }
            }
        }
    }
    // From the frontmatter on; a closing fence with nothing to close ends it (the note after it is dropped).
    let body = &lines[start.unwrap_or(0)..];
    if body.iter().filter(|l| fence(l)).count() % 2 == 1 {
        if let Some(close) = body.iter().rposition(|l| l.trim() == "```") {
            return joined(&body[..close]);
        }
    }
    joined(body)
}

impl App {
    /// The three texts of `agent`'s copy of `skill` against its base: (old base, new base, copy), with the base.
    fn merge_inputs(&self, agent: AgentId, skill: &str) -> Result<(String, String, String, Skill)> {
        let dir = self.overlay_dir_checked(agent, skill)?;
        let a = self.agent(agent)?;
        let base = self
            .base_skill(&a, skill)
            .ok_or_else(|| invalid(format!("{skill} has no base skill any more")))?;
        let mem = self.agent_memory_store(agent)?;
        let row = mem
            .overlays()?
            .into_iter()
            .find(|o| o.skill == skill)
            .ok_or_else(|| invalid(format!("the agent has no copy of {skill}")))?;
        let overlay = std::fs::read_to_string(dir.join(agenium_skills::SKILL_FILE))?;
        let old_base = unapply_patch(&overlay, &row.patch)?;
        let new_base = std::fs::read_to_string(base.meta.dir.join(agenium_skills::SKILL_FILE))?;
        Ok((old_base, new_base, overlay, base))
    }

    /// The user accepts `text` as `agent`'s copy of `skill`, rebuilt on the current base (the helper's draft, or the
    /// user's own edit of it). Checked like an agent's update, then swapped in; the change it records is against the
    /// current base, so the copy is no longer stale.
    pub fn accept_overlay_merge(&self, agent: AgentId, skill: &str, text: &str) -> Result<()> {
        let (_, new_base, overlay, base) = self.merge_inputs(agent, skill)?;
        let text = unfenced(text);
        self.text_gate(&[&overlay, &new_base], &text, "merge")?;
        let dir = self.overlay_dir(agent, skill);
        let staging_root = dir.with_file_name(format!(".merge-{}", ulid::Ulid::new()));
        let staging = staging_root.join(skill);
        copy_dir(&dir, &staging)?;
        std::fs::write(staging.join(agenium_skills::SKILL_FILE), &text)?;
        if let Err(e) = agenium_skills::validate(&staging, Self::copy_rules(&base)) {
            let _ = std::fs::remove_dir_all(&staging_root);
            return Err(invalid(format!("the merged skill is not valid: {e}")));
        }
        let swapped = super::swap_dir(&staging, &dir);
        let _ = std::fs::remove_dir_all(&staging_root);
        swapped?;
        let version = base
            .meta
            .version
            .clone()
            .unwrap_or_else(|| "unversioned".into());
        self.agent_memory_store(agent)?.put_overlay(
            skill,
            &version,
            &diff_texts(&new_base, &text)?,
            serde_json::json!({
                "at": now_iso(),
                "reason": "merged onto the updated base by the user",
                "rebased_onto": version,
            }),
        )?;
        self.store.commit(None, "user", |w| {
            w.emit(DomainEvent::AgentUpdated {
                agent,
                field: format!("skill_overlay_merged:{skill}"),
            })
            .map_err(OrchError::from)
        })?;
        Ok(())
    }
}

impl Runtime {
    /// The helper's draft of `agent`'s copy of `skill` rebuilt on the current base, keeping what the agent learned.
    /// Nothing is written: the user reads it (and may edit it) before `accept_overlay_merge`.
    pub async fn draft_overlay_merge(
        &self,
        agent: AgentId,
        skill: &str,
        backend: BackendKind,
        model: &ModelBinding,
    ) -> Result<OverlayMergeDraft> {
        let (old_base, new_base, overlay, _) = self.app.merge_inputs(agent, skill)?;
        let ask = format!(
            "OLD:\n<<<\n{old_base}\n>>>\n\nNEW:\n<<<\n{new_base}\n>>>\n\nAGENT:\n<<<\n{overlay}\n>>>\n\n\
             Now write the merged SKILL.md between a line {OPEN} and a line {CLOSE}."
        );
        let a = self
            .side_chat(
                "skill",
                None,
                backend,
                model,
                SYSTEM,
                vec![Message::user_text(ask)],
                16_000,
            )
            .await?;
        let merged = unfenced(&a.text);
        if !merged.trim_start().starts_with("---") {
            return Err(invalid(
                "the helper did not answer with a SKILL.md (no frontmatter): try again or another model",
            ));
        }
        Ok(OverlayMergeDraft {
            skill: skill.to_owned(),
            diff: diff_texts(&new_base, &merged)?,
            old_base,
            new_base,
            overlay,
            merged,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::unfenced;

    #[test]
    fn a_fenced_answer_is_unwrapped() {
        assert_eq!(
            unfenced("```markdown\n---\nname: a\n---\nbody\n```"),
            "---\nname: a\n---\nbody\n"
        );
        assert_eq!(
            unfenced("---\nname: a\n---\nbody"),
            "---\nname: a\n---\nbody\n"
        );
        // Between the markers it was asked for, a note and a second version after them.
        assert_eq!(
            unfenced("Merged:\n<<<MERGED\n---\nname: a\n---\nbody\nMERGED>>>\n\nNote: or this:\n```\n---\nname: b"),
            "---\nname: a\n---\nbody\n"
        );
        // Between `<<<` and `>>>`, with a note around them (what Claude answered on the real prompt).
        assert_eq!(
            unfenced("Here it is.\n<<<\n---\nname: a\n---\nbody\n\n>>>\n\nBoth changes kept."),
            "---\nname: a\n---\nbody\n"
        );
        // Fenced, with the skill's own code block inside and a note after (Haiku's answer).
        assert_eq!(
            unfenced("Merged:\n```markdown\n---\nname: a\n---\nRun:\n```bash\nmake\n```\nDone.\n```\n\nMerge notes:\n- kept both"),
            "---\nname: a\n---\nRun:\n```bash\nmake\n```\nDone.\n"
        );
        // No opening fence, a stray closing one and a note after (Sonnet's answer).
        assert_eq!(
            unfenced(
                "---\nname: a\n---\nbody\n```\n\nI took the frontmatter from the new version."
            ),
            "---\nname: a\n---\nbody\n"
        );
    }
}
