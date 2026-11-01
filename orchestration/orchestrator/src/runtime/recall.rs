//! What an agent is reminded of from its memory in every kind of run (ADR-155). A work run recalled from the
//! ticket's text since ADR-51; a chat turn, the Lead's inbox, a review and the planner's draft recalled nothing, so the
//! preferences and facts the Lead learned from the chat (ADR-98) were written and never read back. Each now recalls
//! by what it is about, logs what it showed on the run's page, and credits it as a work run does.

use agenium_backend::prompt;
use agenium_core::*;
use agenium_memory::MemoryRetriever;

use super::Runtime;
use crate::Result;

impl Runtime {
    /// The memories `agent` is shown for `query` in `run` of `project`, as the prompt's memory section ("" when none).
    pub(super) async fn recall_text(
        &self,
        agent: AgentId,
        project: ProjectId,
        run: RunId,
        ticket: Option<&str>,
        query: String,
    ) -> Result<String> {
        if query.trim().is_empty() {
            return Ok(String::new());
        }
        let mem = self.agent_memory(agent)?;
        let pool = if self.app.config().memory.share {
            self.app.pool_memory().ok()
        } else {
            None
        };
        let retriever = MemoryRetriever::new(mem, self.embedder.clone()).with_pool(pool);
        let at = agenium_memory::retrieve::RecallContext {
            project: Some(project.to_string()),
            scope: serde_json::json!({ "project": self.app.project(project)?.name }),
        };
        // Embedding the query takes the CPU for a while: off the async threads (audit L17).
        let (r, k) = (retriever.clone(), self.memory_k);
        let recalled = tokio::task::spawn_blocking(move || r.recall_in(&query, &at, k))
            .await
            .ok()
            .and_then(|r| r.ok())
            .unwrap_or_default();
        if recalled.is_empty() {
            return Ok(String::new());
        }
        retriever.record(&run.to_string(), ticket, &recalled).ok();
        let shown: Vec<serde_json::Value> = recalled
            .iter()
            .map(|r| serde_json::json!({ "id": r.id, "kind": r.kind, "text": r.text, "shared": r.shared, "exact": r.exact }))
            .collect();
        let _ = self
            .app
            .run_log(run, "memory", serde_json::json!({ "recalled": shown }));
        let facts: Vec<String> = recalled
            .iter()
            .map(agenium_memory::retrieve::label)
            .collect();
        Ok(prompt::memory_text(&facts))
    }

    /// The plan as a worker sees it beside its ticket (ADR-155): each item with the state of its tickets, the
    /// worker's own item marked, so it knows where its work sits and what others build. "" without a plan.
    pub(super) fn plan_outline(&self, project: ProjectId, own_item: &str) -> Result<String> {
        use agenium_store::core::plans;
        let Some(plan) = self.app.store().read(|c| plans::latest_plan(c, project))? else {
            return Ok(String::new());
        };
        let tickets = self.app.tickets(project)?;
        let mut s = format!(
            "## The plan (v{}; your ticket's item is marked)\n",
            plan.version
        );
        for it in &plan.body.items {
            let of: Vec<String> = tickets
                .iter()
                .filter(|t| t.plan_item == it.id)
                .map(|t| format!("{} {:?}", t.key, t.status))
                .collect();
            s.push_str(&format!(
                "- {}{} {} (role {}{}){}\n",
                it.id,
                if it.id == own_item { " [yours]" } else { "" },
                it.title,
                it.role,
                if it.depends_on.is_empty() {
                    String::new()
                } else {
                    format!(", after {}", it.depends_on.join(", "))
                },
                if of.is_empty() {
                    String::new()
                } else {
                    format!(": {}", of.join("; "))
                }
            ));
        }
        s.push('\n');
        Ok(s)
    }
}
