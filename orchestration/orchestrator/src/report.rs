//! The project's handover report (M5, ADR-147): what was agreed and what was delivered, in Markdown, built from the
//! stored state — no model call. Per requirement, how the plan covers it; per ticket, who worked on it (from the runs,
//! not the team as it is now), the worker's result per criterion with its evidence and what the runs' logs show
//! checked it, the review and who did it (an agent reviewing its own work in Solo is said so: not an independent
//! review), the commit, the checks not run, risks, follow-ups, and why a ticket was cancelled or waived; then usage.
//! A copy is kept each time the project completes (`<root>/reports/<project>/`).

use std::collections::BTreeSet;
use std::fmt::Write;

use agenium_core::forms::{ReviewVerdict, WorkReport};
use agenium_core::*;
use agenium_store::core::{comments, ledger, runs, workspaces};

use crate::app::App;
use crate::Result;

/// One line of Markdown table text: no pipes or line breaks.
fn cell(s: &str) -> String {
    s.replace('|', "\\|").replace(['\n', '\r'], " ")
}

impl App {
    /// The handover report of `project` (see the module docs).
    pub fn project_report_md(&self, project: ProjectId) -> Result<String> {
        let p = self.project(project)?;
        let v = self.plan_view(project)?;
        let state = self.run_state(project)?;
        let tickets = self.tickets(project)?;
        let solo = self.team_of_project(project)?.is_some_and(|(t, _)| t.solo);
        let name = |a: AgentId| {
            self.agent(a)
                .map(|a| a.name)
                .unwrap_or_else(|_| a.to_string())
        };
        let mut s = String::new();
        let _ = writeln!(s, "# {} ({}) — handover report\n", p.name, p.key_prefix);
        let _ = writeln!(s, "- Status: {state:?}");
        let _ = writeln!(s, "- Written: {}", now_iso());
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
            let _ = writeln!(
                s,
                "- Plan: v{} ({approved}, for spec v{})",
                plan.version, plan.spec_version
            );
        }
        if solo {
            s.push_str("- Team: Solo — the agent reviewed its own work; no independent review unless the user reviewed.\n");
        }

        if let Some(spec) = &v.spec {
            s.push_str("\n## Goals\n");
            for g in &spec.body.goals {
                let _ = writeln!(s, "- {g}");
            }
            // Coverage: each requirement, the items that trace to it and where they stand.
            s.push_str("\n## Requirements and coverage\n");
            let items = v
                .plan
                .as_ref()
                .map(|p| p.body.items.as_slice())
                .unwrap_or_default();
            for r in &spec.body.requirements {
                let _ = writeln!(s, "- **{}**: {}", r.id, r.text);
                for a in &r.acceptance {
                    let _ = writeln!(s, "  - {}: {}", a.id, a.text);
                }
                let tracing: Vec<String> = items
                    .iter()
                    .filter(|i| i.trace_to.contains(&r.id))
                    .map(|i| {
                        let st = v.items.iter().find(|x| x.id == i.id).map_or("open", |x| {
                            if x.done {
                                "done"
                            } else if x.waived {
                                "waived by the user"
                            } else {
                                "open"
                            }
                        });
                        format!("{} ({st})", i.id)
                    })
                    .collect();
                if tracing.is_empty() {
                    s.push_str("  - Covered by: **no plan item**\n");
                } else {
                    let _ = writeln!(s, "  - Covered by: {}", tracing.join(", "));
                }
            }
        }

        if !v.items.is_empty() {
            s.push_str("\n## Plan progress\n");
            for it in &v.items {
                let done = if it.done || it.waived { "x" } else { " " };
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
                let waived = if it.waived { " (waived)" } else { "" };
                let _ = writeln!(s, "- [{done}] {} {}{waived}{trailer}", it.id, it.title);
                if !it.unaccounted.is_empty() {
                    let ks: Vec<String> = it.unaccounted.iter().map(|k| k.to_string()).collect();
                    let _ = writeln!(
                        s,
                        "  - waits: {} cancelled, not carried on or waived",
                        ks.join(", ")
                    );
                }
            }
        }

        if !tickets.is_empty() {
            s.push_str("\n## Tickets\n\n| Key | Title | Status | Worked by | Reopens |\n|---|---|---|---|---|\n");
            for t in &tickets {
                let _ = writeln!(
                    s,
                    "| {} | {} | {} | {} | {} |",
                    t.key,
                    cell(&t.title),
                    t.status,
                    cell(&self.worked_by(t, &name)?),
                    t.reopen_count
                );
            }
            let closed = tickets
                .iter()
                .filter(|t| t.status == TicketStatus::Closed)
                .count();
            let _ = writeln!(s, "\n{closed}/{} ticket(s) closed.", tickets.len());
            s.push_str("\n## Ticket details\n");
            let mut risks = Vec::new();
            let mut follow_ups = Vec::new();
            for t in &tickets {
                self.ticket_section(&mut s, t, solo, &name, &mut risks, &mut follow_ups)?;
            }
            if !risks.is_empty() || !follow_ups.is_empty() {
                s.push_str("\n## Open risks and follow-ups\n");
                for r in &risks {
                    let _ = writeln!(s, "- Risk: {r}");
                }
                for f in &follow_ups {
                    let _ = writeln!(s, "- Follow-up: {f}");
                }
            }
            let mut usage = UsageSummary::default();
            for t in &tickets {
                usage.add(&t.usage);
            }
            s.push_str("\n## Usage\n");
            let _ = writeln!(
                s,
                "- Tickets' runs: {} tokens in ({} from cache), {} out, about ${:.2}",
                usage.tokens_in, usage.cache_read, usage.tokens_out, usage.estimated_cost_usd
            );
        }
        Ok(s)
    }

    /// Who worked on `t`, from its runs (a reassigned ticket names everyone); the assignee if it has no run yet.
    fn worked_by(&self, t: &Ticket, name: &dyn Fn(AgentId) -> String) -> Result<String> {
        let mut who: Vec<String> = Vec::new();
        for r in self.store.read(|c| runs::for_ticket(c, t.id))? {
            let n = name(r.agent);
            if !who.contains(&n) {
                who.push(n);
            }
        }
        if who.is_empty() {
            who.extend(t.assignee.map(name));
        }
        Ok(if who.is_empty() {
            "-".into()
        } else {
            who.join(", ")
        })
    }

    fn ticket_section(
        &self,
        s: &mut String,
        t: &Ticket,
        solo: bool,
        name: &dyn Fn(AgentId) -> String,
        risks: &mut Vec<String>,
        follow_ups: &mut Vec<String>,
    ) -> Result<()> {
        let cs = self.store.read(|c| comments::for_ticket(c, t.id))?;
        let _ = writeln!(s, "\n### {} {} ({})", t.key, t.title, t.status);
        let _ = writeln!(
            s,
            "- Plan item: {}; worked by {}",
            t.plan_item,
            self.worked_by(t, name)?
        );
        if t.status == TicketStatus::Cancelled {
            let x = self.store.read(|c| ledger::cancellation(c, t.id))?;
            let how = match &x {
                Some(x) if x.waived_by.is_some() => "waived: the work is not needed".to_owned(),
                Some(x) => match x.replaced_by {
                    Some(r) => format!(
                        "its work is carried on by {}",
                        self.store
                            .read(|c| agenium_store::core::tickets::get(c, r))?
                            .key
                    ),
                    None => "**not carried on or waived yet**".to_owned(),
                },
                None => "**not carried on or waived yet**".to_owned(),
            };
            let _ = writeln!(
                s,
                "- Cancelled: {} — {how}",
                t.close_reason.as_deref().unwrap_or("no reason given")
            );
        }
        let report: Option<&WorkReport> = cs.iter().rev().find_map(|c| match &c.body {
            CommentBody::Report(r) => Some(r),
            _ => None,
        });
        if let Some(r) = report {
            let logs = self
                .store
                .read(|c| crate::verification::ticket_logs(c, t.id))?;
            let record = crate::verification::record(r, &logs);
            let _ = writeln!(s, "- Result: {}", r.summary.trim());
            for (a, c) in r.acceptance.iter().zip(&record.criteria) {
                let _ = writeln!(
                    s,
                    "  - {} {:?}: {} — check: {}",
                    a.ac_id,
                    a.status,
                    a.evidence.trim(),
                    c.backing.text()
                );
            }
            if record.checks_after.is_empty() {
                s.push_str("- Checks after the last change: none on record\n");
            } else {
                let ran: Vec<String> = record
                    .checks_after
                    .iter()
                    .map(|(cmd, code)| {
                        format!(
                            "`{cmd}` -> {}",
                            code.map_or("no exit code".into(), |c| c.to_string())
                        )
                    })
                    .collect();
                let _ = writeln!(s, "- Checks after the last change: {}", ran.join("; "));
            }
            if !r.checks_not_run.is_empty() {
                let _ = writeln!(s, "- Checks not run: {}", r.checks_not_run.join("; "));
            }
            if !r.beyond_ticket.is_empty() {
                let _ = writeln!(s, "- Beyond the ticket: {}", r.beyond_ticket.join("; "));
            }
            if !r.tests_changed.is_empty() {
                let _ = writeln!(s, "- Tests changed: {}", r.tests_changed.join("; "));
            }
            if !r.artifacts.is_empty() {
                let a: Vec<String> = r
                    .artifacts
                    .iter()
                    .map(|a| format!("{} {}", a.kind, a.reference))
                    .collect();
                let _ = writeln!(s, "- Artifacts: {}", a.join(", "));
            }
            let open = !matches!(t.status, TicketStatus::Cancelled);
            if open {
                risks.extend(r.risks.iter().map(|x| format!("{} — {x}", t.key)));
                follow_ups.extend(r.follow_ups.iter().map(|x| format!("{} — {x}", t.key)));
            }
        }
        // The last review, and who made it.
        if let Some((rev, author)) = cs.iter().rev().find_map(|c| match &c.body {
            CommentBody::Review(r) => Some((r, &c.author)),
            _ => None,
        }) {
            let by = match author {
                Author::Agent(a) if solo && Some(*a) == t.assignee => {
                    format!("{} (self-review, Solo)", name(*a))
                }
                Author::Agent(a) => name(*a),
                other => other.to_string(),
            };
            let verdict = match rev.decision {
                ReviewVerdict::Approve => "approved",
                ReviewVerdict::Reopen => "reopened",
            };
            let _ = writeln!(s, "- Review: {verdict} by {by}");
            for f in &rev.ac_feedback {
                let _ = writeln!(s, "  - {}: {}", f.ac_id, f.feedback.trim());
            }
        }
        if let Some(ws) = self.store.read(|c| workspaces::get(c, t.id))? {
            match (&ws.merged_sha, &ws.head_sha) {
                (Some(m), _) => {
                    let main = self.project(t.project)?.main_branch;
                    let _ = writeln!(s, "- Merged into {main} at {}", &m[..m.len().min(12)]);
                }
                (None, Some(h)) => {
                    let _ = writeln!(
                        s,
                        "- Branch agenium/{} at {} (not merged)",
                        t.key,
                        &h[..h.len().min(12)]
                    );
                }
                _ => {}
            }
        }
        if let Some(r) = t
            .close_reason
            .as_deref()
            .filter(|_| t.status == TicketStatus::Closed)
        {
            let _ = writeln!(s, "- Closed: {r}");
        }
        Ok(())
    }

    /// Writes the report as a new version under `<root>/reports/<project>/` and returns its path (ADR-147: a project
    /// is marked Completed only once its report is written).
    pub fn save_handover(&self, project: ProjectId) -> Result<std::path::PathBuf> {
        let p = self.project(project)?;
        let md = self.project_report_md(project)?;
        let dir = self.layout().root.join("reports").join(&p.name);
        std::fs::create_dir_all(&dir)?;
        let taken: BTreeSet<String> = std::fs::read_dir(&dir)?
            .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().into_owned()))
            .collect();
        let n = (1..)
            .find(|n| !taken.contains(&format!("handover-v{n}.md")))
            .unwrap_or(1);
        let file = dir.join(format!("handover-v{n}.md"));
        std::fs::write(&file, md)?;
        Ok(file)
    }
}
