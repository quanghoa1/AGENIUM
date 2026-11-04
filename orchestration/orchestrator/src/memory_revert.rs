//! Undoing what one memory episode learned (a learning run of a ticket or of the chat): the facts it added are retired,
//! those it replaced or retired are live again. The memory had no way back before: a wrong extraction (a review
//! misread, an untrusted comment that got through) could only be deleted fact by fact. Facts are copy-on-write, so the
//! old text is still there; the run's log names the ids each change touched.

use agenium_core::RunId;
use agenium_store::core::runs;
use agenium_store::Undo;
use serde_json::{json, Value};

use crate::app::App;
use crate::{OrchError, Result};

/// The run log line that marks a learning run undone (a second undo does nothing).
pub const REVERTED: &str = "memory-revert";

impl App {
    /// Undoes the memory changes of the learning run `run` in its agent's memory. A fact the episode wrote that held
    /// in contexts since (a proven lesson) is named in `warnings` and nothing is changed unless `force`. Refused when
    /// a later episode changed one of its facts, or when the run was undone already.
    pub fn revert_memory_run(
        &self,
        run: RunId,
        dim: usize,
        model_id: &str,
        force: bool,
    ) -> Result<Value> {
        let (r, log) = self
            .store()
            .read(|c| Ok((runs::get(c, run)?, runs::log_entries(c, run)?)))?;
        if log.iter().any(|e| e.kind == REVERTED) {
            return Err(OrchError::Invalid(
                "this learning was undone already".into(),
            ));
        }
        let changes: Vec<&Value> = log
            .iter()
            .filter(|e| e.kind == "memory")
            .filter_map(|e| e.data["changes"].as_array())
            .flatten()
            .collect();
        if changes.is_empty() {
            return Err(OrchError::Invalid(
                "this run changed nothing in the memory".into(),
            ));
        }
        let id = |c: &Value, k: &str| c[k].as_i64();
        let undo: Vec<Undo> = changes
            .iter()
            .map(|c| Undo {
                id: id(c, "id"),
                old_id: id(c, "old_id"),
            })
            .collect();
        if undo.iter().any(|u| u.id.is_none() && u.old_id.is_none()) {
            return Err(OrchError::Invalid(
                "this run learned before undo was possible (its changes do not name their facts)"
                    .into(),
            ));
        }
        let mem = self.agent_store(r.agent, dim, model_id)?;
        let warnings: Vec<String> = undo
            .iter()
            .filter_map(|u| u.id)
            .filter_map(|i| mem.get(i).ok())
            .filter(|f| !f.validated.is_empty())
            .map(|f| {
                format!(
                    "{:?} held in {} context(s) since: undoing it loses that proof",
                    f.text,
                    f.validated.len()
                )
            })
            .collect();
        if !warnings.is_empty() && !force {
            return Ok(json!({ "done": false, "warnings": warnings }));
        }
        let n = mem.undo(&undo).map_err(OrchError::from)?;
        // A skill the agent built on a lesson that is gone now is flagged for review (ADR-91).
        let flagged = self.audit_created_skills(r.agent)?;
        self.run_log(
            run,
            REVERTED,
            json!({ "undone": n, "warnings": warnings, "skills_flagged": flagged }),
        )?;
        Ok(json!({ "done": true, "undone": n, "warnings": warnings, "skills_flagged": flagged }))
    }
}
