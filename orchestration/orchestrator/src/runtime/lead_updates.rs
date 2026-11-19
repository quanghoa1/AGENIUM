//! What the Lead did in a run of its own (an inbox run, a review), kept for the chat to show: its moves (tickets
//! created, assigned, reviewed, closed; permissions and questions answered) and what it wrote. Kept apart from the
//! conversation, which the model reads and which stays the user's and the Lead's. `at`: the chat's number of messages
//! then, so the chat shows it between the turns it came between (2026-11-19: the Lead's work showed nothing in the chat).

use std::str::FromStr;

use super::*;

/// The session that keeps them, in the Lead's own database.
pub fn session(project: ProjectId) -> String {
    format!("lead-updates:{project}")
}

/// A protocol move worth showing; the Lead's reads (`list_tickets`, `read_ticket`, `get_team`) and its file reads are
/// not. A CLI may name the tool with its MCP server (`mcp__agenium__create_ticket`, `agenium_create_ticket`).
pub(crate) fn shown(name: &str) -> bool {
    let bare = name
        .trim_start_matches("cli:")
        .rsplit("__")
        .next()
        .unwrap_or(name);
    let bare = bare.strip_prefix("agenium_").unwrap_or(bare);
    matches!(
        ProtocolTool::from_str(bare),
        Ok(t) if !matches!(t, ProtocolTool::GetTeam | ProtocolTool::ListTickets | ProtocolTool::ReadTicket)
    )
}

impl Runtime {
    /// What an inbox run or a review ends with: a few words for the user on what the Lead did, in the language the user
    /// writes to it in (the chat shows them as the Lead's update), while all a ticket or a worker reads stays English.
    pub(super) fn user_note_rule(&self, lead: AgentId, project: ProjectId) -> String {
        let last: String = self
            .agent_memory(lead)
            .ok()
            .and_then(|m| m.load_messages(&learn::chat_session(project)).ok())
            .unwrap_or_default()
            .into_iter()
            .rev()
            .filter_map(|v| serde_json::from_value::<Message>(v).ok())
            .find(|m| m.role == agenium_llm::Role::User)
            .map(|m| {
                learn::user_words(&m.text())
                    .trim()
                    .chars()
                    .take(300)
                    .collect()
            })
            .unwrap_or_default();
        let language = if last.is_empty() {
            "English".to_owned()
        } else {
            format!("the language of the user's latest message to you:\n<user_message>\n{last}\n</user_message>")
        };
        format!(
            "\n\nThe user reads in your chat what you write here. Your run may end right after your last call (closing \
             or reopening the ticket, or handling the last item), so write it in the same message as that call: one to \
             three sentences for the user on what you did and what comes next, in {language}\nTickets, comments, \
             questions, reviews and all a worker reads stay in English."
        )
    }

    /// Keeps the moves of run `run` (from its log, the same for every brain) and its last text. Nothing is kept for a
    /// run that made no move and wrote nothing; a failure is logged, never failing the run.
    pub(super) fn note_lead_update(
        &self,
        agent: AgentId,
        project: ProjectId,
        run: RunId,
        kind: &str,
        ticket: Option<&TicketKey>,
        text: &str,
    ) {
        let calls: Vec<serde_json::Value> = self
            .app
            .store()
            .read(|c| runs::log_entries(c, run))
            .unwrap_or_default()
            .into_iter()
            .filter(|e| e.kind == "tool")
            .filter(|e| e.data["tool"].as_str().is_some_and(shown))
            .map(|e| serde_json::json!({ "name": e.data["tool"], "args": e.data["args"], "ok": e.data["ok"] != false }))
            .collect();
        if calls.is_empty() && text.trim().is_empty() {
            return;
        }
        let Ok(mem) = self.agent_memory(agent) else {
            return;
        };
        let at = mem
            .load_messages(&learn::chat_session(project))
            .map(|m| m.len())
            .unwrap_or(0);
        let v = serde_json::json!({ "at": at, "kind": kind, "ticket": ticket, "calls": calls, "text": text.trim() });
        if let Err(e) = mem.append_message(&session(project), &v) {
            tracing::warn!(error = %e, %project, "the Lead's work was not kept for the chat");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::shown;

    #[test]
    fn moves_are_shown_reads_are_not() {
        for n in [
            "create_ticket",
            "mcp__agenium__assign_ticket",
            "agenium_close_ticket",
            "approve_permission",
        ] {
            assert!(shown(n), "{n}");
        }
        for n in [
            "list_tickets",
            "mcp__agenium__read_ticket",
            "get_team",
            "read_file",
            "cli:Bash",
        ] {
            assert!(!shown(n), "{n}");
        }
    }
}
