//! What a run reports as it goes: its events to the live view, its messages to its memory, its tool calls and
//! steps to the run log, its cost.

use super::*;

/// Persists what happens in a run. Conversation messages go to the agent's private database (session = run id)
/// so a continuation run resumes from them; tool calls and errors go to the structured run log (redacted).
pub struct RunObserver {
    pub app: Arc<App>,
    pub run: RunId,
    pub memory: Option<Arc<AgentStore>>,
    pub live: Option<LiveFn>,
    pub steps: Mutex<u32>,
    /// When each pending tool call (by name) was emitted, for its duration in the run log.
    /// When each tool call started, by tool name in call order (the observer is not told the call id).
    pub tool_started:
        Mutex<std::collections::HashMap<String, std::collections::VecDeque<std::time::Instant>>>,
    /// The model's price (an API provider), and the run's cost so far, priced request by request.
    pub price: Option<agenium_llm::pricing::Price>,
    pub cost: Mutex<f64>,
    /// The run's ticket: each check it runs is logged with the tree its worktree held then (ADR-148).
    pub ticket: Option<TicketId>,
    /// The context at the last request and the model's window (ADR-155); the window known here stands in for a
    /// backend that does not know it (a CLI).
    pub context: Mutex<(u64, u64)>,
    pub window: u64,
    /// A conversation folded to fit replaces the stored one (not a chat turn: the chat keeps its own history).
    pub keep_folded: bool,
}

impl RunObserver {
    fn with_context(&self, mut s: UsageSummary) -> UsageSummary {
        let (used, window) = *self.context.lock().unwrap_or_else(|p| p.into_inner());
        if used > 0 {
            s.context_used = used;
            s.context_window = if window > 0 { window } else { self.window };
        }
        s
    }
}

impl Observer for RunObserver {
    fn event(&self, e: &ChatEvent) {
        if let ChatEvent::ToolCall(c) = e {
            self.tool_started
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .entry(c.name.clone())
                .or_default()
                .push_back(std::time::Instant::now());
        }
        if let Some(l) = &self.live {
            l(e);
        }
    }

    fn message(&self, m: &Message) {
        if let (Some(s), Ok(v)) = (&self.memory, serde_json::to_value(m)) {
            if let Err(e) = s.append_message(&self.run.to_string(), &v) {
                tracing::warn!(error = %e, "could not persist conversation message");
            }
        }
    }

    fn tool(&self, name: &str, args: &serde_json::Value, result: &Result<String, ToolError>) {
        let (ok, preview) = match result {
            Ok(t) => (true, t.chars().take(2000).collect::<String>()),
            Err(e) => (false, e.to_string()),
        };
        let ms = self
            .tool_started
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get_mut(name)
            .and_then(|q| q.pop_front())
            .map(|t| t.elapsed().as_millis() as u64);
        let mut entry = serde_json::json!({ "tool": name, "args": args, "ok": ok, "result": preview, "ms": ms });
        let check = name == "shell"
            && args["command"]
                .as_str()
                .is_some_and(crate::judge::done_check::is_check);
        if let (true, Some(t)) = (check, self.ticket) {
            if let Some(tree) = self.app.worktree_tree(t) {
                entry["tree"] = tree.into();
            }
        }
        // The done-check and the verification read this log: a lost line is said.
        if let Err(e) = self.app.run_log(self.run, "tool", entry) {
            tracing::warn!(error = %e, "a tool call was not logged");
        }
    }

    fn response(&self, usage: &Usage) {
        if let Some(p) = &self.price {
            *self.cost.lock().unwrap_or_else(|e| e.into_inner()) += p.cost(usage);
        }
    }

    fn step(&self, step: u32, usage: &Usage) {
        *self.steps.lock().unwrap_or_else(|p| p.into_inner()) = step;
        let _ = self
            .app
            .run_progress(self.run, step, &self.with_context(summary(usage, 0.0, 0)));
    }

    fn context(&self, used: u64, window: u64) {
        *self.context.lock().unwrap_or_else(|p| p.into_inner()) = (used, window);
        let steps = *self.steps.lock().unwrap_or_else(|p| p.into_inner());
        // Before the first request too, so the gauge shows what the run starts with.
        if steps == 0 {
            let _ = self
                .app
                .run_progress(self.run, 0, &self.with_context(UsageSummary::default()));
        }
    }

    fn replaced(&self, messages: &[Message]) {
        if !self.keep_folded {
            return;
        }
        if let Some(s) = &self.memory {
            let values: Vec<serde_json::Value> = messages
                .iter()
                .filter_map(|m| serde_json::to_value(m).ok())
                .collect();
            if let Err(e) = s.replace_messages(&self.run.to_string(), &values) {
                tracing::warn!(error = %e, "could not keep the folded conversation");
            }
        }
    }

    fn retry(&self, attempt: u32, delay_ms: u64, error: &str) {
        let _ = self.app.run_log(
            self.run,
            "retry",
            serde_json::json!({ "attempt": attempt, "delay_ms": delay_ms, "error": error }),
        );
    }

    fn control(&self, kind: &str, text: &str) {
        let _ = self.app.run_log(
            self.run,
            "control",
            serde_json::json!({ "what": kind, "text": text.chars().take(500).collect::<String>() }),
        );
    }
}
