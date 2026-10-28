//! `find_code` (ADR-128): a work tool that finds code by what it does. A coding agent asks a behavioural question
//! ("where is the write scope checked before a merge?") and gets the relevant files, most relevant first, and the
//! units of source that implement, control or test the behaviour, verbatim with line numbers — a place to start
//! instead of a round of grep, list_dir and read_file. The judge (Jev) rates folders, then files, then units; grep
//! stays the tool for an exact name.
//!
//! Learned from jevgrep (`docs/reference/JEV-OPPORTUNITIES.md` §5), written here: one file per step — [`files`] (what
//! may be read: nothing ignored, built, binary or holding a secret), [`outline`] (units of source), [`rate`] (one
//! criterion over many items), [`search`] (the steps), [`render`] (what the agent reads). Offered when the judge is on
//! and `find_code` is enabled; read-only; fail-open (no judge: the tool says so and the agent uses grep).

mod files;
mod outline;
mod rate;
mod render;
mod search;

use std::sync::Arc;

use agenium_tools::{Tool, ToolCtx, ToolError, ToolOutput};
use async_trait::async_trait;
use serde::Deserialize;

use crate::app::App;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Args {
    question: String,
    #[serde(default)]
    path: Option<String>,
}

/// The tool for a run on `ticket` (its key, for the judge's activity).
pub fn tool(app: Arc<App>, ticket: Option<String>) -> Arc<dyn Tool> {
    Arc::new(FindCode { app, ticket })
}

struct FindCode {
    app: Arc<App>,
    ticket: Option<String>,
}

#[async_trait]
impl Tool for FindCode {
    fn name(&self) -> &str {
        "find_code"
    }

    fn description(&self) -> &str {
        "Find code by what it does: ask a question about behaviour (\"where is a request authenticated before it \
         reaches a handler?\", \"how are database connections pooled and closed?\", \"which tests cover retries on \
         timeout?\"). Returns the relevant files, most relevant first, and the functions/types that implement or test \
         it, verbatim with line numbers. Use it first when you do not know where something lives; use grep for an exact \
         name or string, read_file to read further."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "question": {"type": "string", "description": "A question about behaviour, in plain words."},
                "path": {"type": "string", "description": "Folder to search (default: the whole worktree)."}
            },
            "required": ["question"],
            "additionalProperties": false
        })
    }

    fn read_only(&self) -> bool {
        true
    }

    async fn call(&self, ctx: &ToolCtx, args: serde_json::Value) -> Result<ToolOutput, ToolError> {
        let a: Args = serde_json::from_value(args)
            .map_err(|e| ToolError::new("invalid_arguments", e.to_string()))?;
        if a.question.trim().len() < 8 {
            return Err(ToolError::new(
                "invalid_arguments",
                "ask a question about behaviour in a few words; for an exact name use grep",
            ));
        }
        let root = match &a.path {
            Some(p) if !p.trim().is_empty() && p.trim() != "." => ctx.resolve(p, false)?,
            _ => ctx.root.clone(),
        };
        if !root.is_dir() {
            return Err(ToolError::new(
                "not_a_directory",
                format!("{} is not a folder", root.display()),
            ));
        }
        let shown = root
            .strip_prefix(&ctx.root)
            .map(|r| r.to_string_lossy().replace('\\', "/"))
            .ok()
            .filter(|r| !r.is_empty())
            .unwrap_or_else(|| ".".into());
        let text = self
            .app
            .find_code(&root, &shown, &a.question, self.ticket.as_deref())
            .await?;
        Ok(ToolOutput {
            text,
            truncated: false,
            full: None,
            untrusted: false,
            images: Vec::new(),
        })
    }
}

impl App {
    /// Searches `root` (shown to the agent as `shown`) for code that answers `question`. `Err` when the judge is off.
    pub async fn find_code(
        &self,
        root: &std::path::Path,
        shown: &str,
        question: &str,
        ticket: Option<&str>,
    ) -> Result<String, ToolError> {
        self.judge().ok().flatten().ok_or_else(|| {
            ToolError::new(
                "unavailable",
                "find_code needs the judge, which is off now: use grep and read_file",
            )
        })?;
        Ok(search::search(self, root, shown, question, ticket).await)
    }
}
