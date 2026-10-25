//! A Markdown report of a project (M5): the agreed requirements, the plan and its progress, and the tickets with
//! their outcomes — a readable summary to share or keep. Built from the stored state; no model call.

use std::fmt::Write;

use agenium_core::{ProjectId, TicketStatus};

use crate::app::App;
use crate::Result;

impl App {
    /// A Markdown summary of `project`: spec goals and requirements, plan items with progress, and a ticket table.
    pub fn project_report_md(&self, project: ProjectId) -> Result<String> {
        let p = self.project(project)?;
        let v = self.plan_view(project)?;
        let state = self.run_state(project)?;
        let mut s = String::new();
        let _ = writeln!(s, "# {} ({})\n", p.name, p.key_prefix);
        let _ = writeln!(s, "- Status: {state:?}");
        if let Some(spec) = &v.spec {
            let agreed = if spec.locked_at.is_some() {
                "agreed"
            } else {
                "waiting for the user"
            };
            let _ = writeln!(s, "- Requirements: spec v{} ({agreed})", spec.version);
        }
        if let Some(plan) = &v.plan {
            let approved = if plan.approved_by.is_some() {
                "approved"
            } else {
                "waiting for the user"
            };
            let _ = writeln!(s, "- Plan: v{} ({approved})", plan.version);
        }

        if let Some(spec) = &v.spec {
            s.push_str("\n## Goals\n");
            for g in &spec.body.goals {
                let _ = writeln!(s, "- {g}");
            }
            if !spec.body.requirements.is_empty() {
                s.push_str("\n## Requirements\n");
                for r in &spec.body.requirements {
                    let _ = writeln!(s, "- **{}**: {}", r.id, r.text);
                    for a in &r.acceptance {
                        let _ = writeln!(s, "  - {}: {}", a.id, a.text);
                    }
                }
            }
        }

        if !v.items.is_empty() {
            s.push_str("\n## Plan progress\n");
            for it in &v.items {
                let done = if it.done { "x" } else { " " };
                let tickets = it
                    .tickets
                    .iter()
                    .map(|(k, st)| format!("{k} {st}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                let trailer = if tickets.is_empty() {
                    String::new()
                } else {
                    format!(" — {tickets}")
                };
                let _ = writeln!(s, "- [{done}] {} {}{trailer}", it.id, it.title);
            }
        }

        let tickets = self.tickets(project)?;
        if !tickets.is_empty() {
            let members = self
                .team_of_project(project)?
                .map(|(_, m)| m)
                .unwrap_or_default();
            let who = |a: Option<agenium_core::AgentId>| {
                a.and_then(|id| members.iter().find(|m| m.id == id))
                    .map(|m| m.name.clone())
                    .unwrap_or_else(|| "-".into())
            };
            s.push_str("\n## Tickets\n\n| Key | Title | Status | Assignee | Reopens |\n|---|---|---|---|---|\n");
            for t in &tickets {
                let title = t.title.replace('|', "\\|");
                let _ = writeln!(
                    s,
                    "| {} | {title} | {} | {} | {} |",
                    t.key,
                    t.status,
                    who(t.assignee),
                    t.reopen_count
                );
            }
            let closed = tickets
                .iter()
                .filter(|t| t.status == TicketStatus::Closed)
                .count();
            let _ = writeln!(s, "\n{closed}/{} ticket(s) closed.", tickets.len());
        }
        Ok(s)
    }
}
