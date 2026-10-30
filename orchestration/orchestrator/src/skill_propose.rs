//! An agent's proposal to write down a validated lesson as a skill, through every gate (decision 35, ADR-86, ADR-90..92):
//! the judge's contradiction read when it is on, then the orchestrator's own checks (the lesson held in two contexts,
//! the shape, one subject, no copy of another skill), then the quality read added to the answer. The same path for a
//! proposal the agent makes during a run and for the reflection on a newly validated lesson (ADR-146).

use std::sync::Arc;

use agenium_core::forms::{ProposeNewSkill, ProposeSkillUpdate};
use agenium_core::{AgentId, RunId};
use agenium_tools::ToolError;

use crate::app::App;
use crate::OrchError;

/// An orchestrator error as the agent reads it.
pub(crate) fn tool_error(e: &OrchError) -> ToolError {
    let te = e.to_tool_error();
    let prefix = format!("{}: ", te.code);
    let mut message = te
        .message
        .strip_prefix(&prefix)
        .map(str::to_owned)
        .unwrap_or(te.message);
    if let Some(d) = te.detail {
        message.push_str(&format!(" | detail: {d}"));
    }
    ToolError::new(&te.code, message)
}

impl App {
    /// Updates `agent`'s own copy of a skill from a validated lesson (decision 35), the judge reading it for
    /// contradictions first when `skill_quality` is on (ADR-92); `run` logs the quality read.
    pub async fn propose_skill_update(
        self: &Arc<Self>,
        agent: AgentId,
        u: ProposeSkillUpdate,
        run: Option<RunId>,
    ) -> Result<serde_json::Value, ToolError> {
        let skill = u.skill.trim().to_owned();
        if self.config().judge.skill_quality {
            let preview = tokio::task::spawn_blocking({
                let (app, u) = (self.clone(), u.clone());
                move || app.preview_skill_update(agent, &u)
            })
            .await
            .map_err(|e| ToolError::new("internal", e.to_string()))?;
            // ADR-147: whether the judge read the change is logged — read, not read (unavailable, nothing related) or
            // skipped (no preview: the update itself is refused below) — so a fail-open pass is never silent.
            let read = match &preview {
                Ok((_, updated)) => {
                    let body = crate::skill_shape::strip_frontmatter(updated);
                    let desc = updated
                        .lines()
                        .find_map(|l| l.strip_prefix("description: "))
                        .unwrap_or("");
                    let subject = crate::skill_conflict::subject(desc, &body);
                    self.judged_contradictions_read(agent, &skill, &subject, &body)
                        .await
                }
                Err(_) => None,
            };
            if let Some(run) = run {
                let status = match (&preview, &read) {
                    (Err(e), _) => format!("skipped: {e}"),
                    (Ok(_), None) => "not read: judge unavailable or nothing related".to_owned(),
                    (Ok(_), Some(h)) if h.is_empty() => "read: no contradiction".to_owned(),
                    (Ok(_), Some(h)) => format!("read: contradicts {}", h.join(", ")),
                };
                let _ = self.run_log(
                    run,
                    "judge",
                    serde_json::json!({ "use": "skill_contradiction", "skill": skill, "text": status }),
                );
            }
            if let Some(hits) = read {
                if !hits.is_empty() {
                    return Err(ToolError::new(
                        "contradiction",
                        format!(
                            "the judge reads this change as contradicting skill {}: reconcile the skills (or improve \
                             that one) first",
                            hits.join(", ")
                        ),
                    ));
                }
            }
        }
        let mut v = tokio::task::spawn_blocking({
            let app = self.clone();
            move || app.apply_skill_update(agent, &u)
        })
        .await
        .map_err(|e| ToolError::new("internal", e.to_string()))?
        .map_err(|e| tool_error(&e))?;
        // ADR-90: with the judge on, a calibrated quality read of the upgraded skill is added to the answer. Fail-open.
        if let Some(body) = self.agent_overlay_body(agent, &skill) {
            if let Some(quality) = self.skill_quality(&body).await {
                if let Some(run) = run {
                    let _ = self.run_log(
                        run,
                        "judge",
                        serde_json::json!({ "use": "skill_quality", "skill": skill, "text": quality.join("; ") }),
                    );
                }
                if let Some(obj) = v.as_object_mut() {
                    obj.insert("quality".into(), serde_json::json!(quality));
                }
            }
        }
        Ok(v)
    }

    /// Creates a new skill in `agent`'s own folder from a validated lesson, only when every gate passes (ADR-91); the
    /// judge reads it against the closest skills first when it is on (ADR-92).
    pub async fn propose_new_skill(
        self: &Arc<Self>,
        agent: AgentId,
        u: ProposeNewSkill,
    ) -> Result<serde_json::Value, ToolError> {
        let skill = u.name.trim().to_owned();
        let subject = crate::skill_conflict::subject(&u.description, &u.body);
        let hits = self
            .judged_contradictions(agent, &skill, &subject, &u.body)
            .await;
        if !hits.is_empty() {
            return Err(ToolError::new(
                "contradiction",
                format!(
                    "the judge reads the new skill as contradicting skill {}: improve that skill instead, or \
                     reconcile them first",
                    hits.join(", ")
                ),
            ));
        }
        let mut v = tokio::task::spawn_blocking({
            let app = self.clone();
            move || app.create_agent_skill(agent, &u)
        })
        .await
        .map_err(|e| ToolError::new("internal", e.to_string()))?
        .map_err(|e| tool_error(&e))?;
        // Same fail-open quality read as an update: only with the judge on.
        if let Some(body) = self.agent_overlay_body(agent, &skill) {
            if let Some(quality) = self.skill_quality(&body).await {
                if let Some(obj) = v.as_object_mut() {
                    obj.insert("quality".into(), serde_json::json!(quality));
                }
            }
        }
        Ok(v)
    }
}
