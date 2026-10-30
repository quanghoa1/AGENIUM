//! From report to close: the worker's report, the Lead's review, the user's merge approval, closing and cancelling.
//! Split out of `engine.rs`.

use super::*;

impl App {
    pub(super) fn submit_report(
        &self,
        ctx: &CallCtx,
        project: &projects::Project,
        r: WorkReport,
    ) -> Result<Outcome> {
        let pid = ctx.project;
        // Pre-checks outside the transaction so the auto-commit only happens for a report that can be accepted.
        let t0 = self
            .store
            .read(|c| tickets::get_by_key(c, pid, &r.ticket_id))?;
        let agent = require_assignee(&t0, &ctx.actor)?;
        if t0.status != TicketStatus::InProgress {
            return Err(invalid(format!(
                "{} is {}; submit_report is only valid while InProgress",
                t0.key, t0.status
            )));
        }
        check_report_covers_acceptance(&t0, &r)?;
        let role = self.agent(agent)?.role;
        let outside = self.out_of_scope_changes(project, &t0, &role)?;
        if !outside.is_empty() {
            return Err(invalid(format!(
                "{} changes files outside the write scope of the ticket and your role: {}. Restore them, or request_permission (kind=path) for them, then submit again",
                t0.key,
                outside.join(", ")
            )));
        }
        crate::templates::validate_extension(&t0.report_template, &r.extension)?;
        // Auto-commit the worktree (journal-first) and learn whether the ticket carries code.
        let commit = self.auto_commit(project, &t0, agent, ctx.intent_id.as_deref())?;
        self.commit(ctx, |w| {
            let mut t = tickets::get(w.tx, t0.id)?;
            if t.status != TicketStatus::InProgress {
                return Err(invalid(format!("{} changed to {} meanwhile", t.key, t.status)));
            }
            t.has_code = commit.has_code;
            let c = add_comment(w, &t, Author::Agent(agent), None, CommentBody::Report(r))?;
            transition(w, &mut t, &ctx.actor, TicketStatus::Resolved, &Guards { valid_report: true, ..Default::default() })?;
            set_agent_status(w, agent, AgentStatus::Idle)?;
            if let Some(ws) = workspaces::get(w.tx, t.id)? {
                // The base follows the branch's rebases, so the review diff shows this ticket's work only.
                let base_sha = commit.base.clone().unwrap_or_else(|| ws.base_sha.clone());
                workspaces::upsert(w.tx, &workspaces::TicketWorkspace { head_sha: commit.head.clone(), base_sha, ..ws })?;
            }
            Ok(Outcome {
                result: serde_json::json!({ "ticket_id": t.key, "status": t.status, "report_comment": c.id, "has_code": t.has_code, "commit": commit.sha }),
                run: RunEffect::End,
                ops: vec![],
            })
        })
    }

    pub(super) fn review_ticket(
        &self,
        ctx: &CallCtx,
        project: &projects::Project,
        r: ReviewDecision,
    ) -> Result<Outcome> {
        let pid = ctx.project;
        let review_gate = project.review_before_merge();
        self.commit(ctx, |w| {
            let team = require_lead(w, pid, &ctx.actor)?;
            let mut t = ticket_by_key(w, pid, &r.ticket_id)?;
            if t.status != TicketStatus::Resolved {
                return Err(invalid(format!("{} is {}; only Resolved tickets are reviewed", t.key, t.status)));
            }
            let ac_ids: HashSet<&str> = t.acceptance.iter().map(|a| a.id.as_str()).collect();
            for f in &r.ac_feedback {
                if !ac_ids.contains(f.ac_id.as_str()) {
                    return Err(invalid(format!("{} has no acceptance criterion {}", t.key, f.ac_id)));
                }
            }
            if r.decision == ReviewVerdict::Approve {
                // An approval answers the worker's report: a criterion it left Partial or NotMet is approved only on
                // the reviewer's own word (what they checked), otherwise it is a reopen (found on Claude Haiku: a
                // Partial criterion was approved without a word on it).
                let cs = comments::for_ticket(w.tx, t.id)?;
                let report = cs.iter().rev().find_map(|c| match (&c.body, &c.author) {
                    (CommentBody::Report(rep), Author::Agent(a)) if Some(*a) == t.assignee => Some(rep),
                    _ => None,
                });
                if let Some(rep) = report {
                    // ADR-147: a Met criterion whose evidence cites a check the runs' logs do not back (it failed, ran
                    // only before the last change, or never ran) is approved only on the reviewer's word too.
                    let record = crate::verification::record(rep, &crate::verification::ticket_logs(w.tx, t.id)?);
                    let said = |id: &str| r.ac_feedback.iter().any(|f| f.ac_id == id && f.feedback.trim().len() >= 10);
                    let unverified: Vec<&str> = t
                        .acceptance
                        .iter()
                        .filter(|ac| {
                            rep.acceptance.iter().find(|x| x.ac_id == ac.id).is_none_or(|x| x.status != AcStatus::Met)
                                && !said(&ac.id)
                        })
                        .map(|ac| ac.id.as_str())
                        .collect();
                    if !unverified.is_empty() {
                        return Err(invalid(format!(
                            "the report of {} leaves {} not Met: approve with ac_feedback saying how you verified each, or reopen",
                            t.key,
                            unverified.join(", ")
                        )));
                    }
                    let unbacked: Vec<String> = record
                        .criteria
                        .iter()
                        .filter(|c| c.backing.unverified() && !said(&c.ac_id))
                        .map(|c| format!("{}: {}", c.ac_id, c.backing.text()))
                        .collect();
                    if !unbacked.is_empty() {
                        return Err(invalid(format!(
                            "the evidence of {} cites checks the runs' logs do not back ({}): run the check yourself and \
                             approve with ac_feedback saying what you verified, or reopen",
                            t.key,
                            unbacked.join("; ")
                        )));
                    }
                }
            }
            let decision = r.decision;
            add_comment(w, &t, author_of(&ctx.actor), None, CommentBody::Review(r))?;
            let mut ops = vec![];
            match decision {
                ReviewVerdict::Approve => {
                    if t.approved {
                        return Err(invalid(format!("{} is already approved", t.key)));
                    }
                    t.approved = true;
                    tickets::update(w.tx, &t)?;
                    w.emit(DomainEvent::TicketApproved { ticket: t.id, key: t.key.clone() })?;
                    let solo_gate = review_gate && team.as_ref().is_some_and(|t| t.solo) && ctx.actor != Actor::User;
                    if t.has_code && !t.merged {
                        if solo_gate {
                            // Project option "review before merge" (decision 28): the user approves the merge.
                            w.emit_as("system", DomainEvent::NeedsUser { ticket: Some(t.id), reason: format!("{} is approved by the agent and waits for your review before merge", t.key) })?;
                        } else {
                            ops.push(crate::effects::journal(w, &t, Op::Merge)?);
                        }
                    }
                    // Dependents may be unblocked now (approved, and merged when it has code).
                }
                ReviewVerdict::Reopen => {
                    transition(w, &mut t, &ctx.actor, TicketStatus::Reopened, &Guards { per_ac_feedback: true, ..Default::default() })?;
                    // A merge still pending from an earlier approval would land the work the reviewer sent back.
                    crate::effects::drop_pending_merges(w, &t, "the ticket was reopened")?;
                }
            }
            let next = if t.status == TicketStatus::Resolved && t.approved && t.has_code && !t.merged && ops.is_empty() {
                "the merge waits for the user's review (project option review before merge); stop here".to_owned()
            } else {
                review_next(&t)
            };
            Ok(Outcome { result: serde_json::json!({ "ticket_id": t.key, "status": t.status, "approved": t.approved, "merged": t.merged, "next": next }), run: RunEffect::Continue, ops })
        })
    }

    /// The user releases a merge held by "review before merge".
    pub fn user_approve_merge(&self, project: ProjectId, key: &TicketKey) -> Result<Outcome> {
        let out = self.store.commit(None, "user", |w| -> Result<Outcome> {
            let t = ticket_by_key(w, project, key)?;
            if !(t.status == TicketStatus::Resolved && t.approved && t.has_code && !t.merged) {
                return Err(invalid(format!("{} is not waiting for a merge", t.key)));
            }
            let op = crate::effects::journal(w, &t, Op::Merge)?;
            Ok(Outcome {
                result: serde_json::json!({ "ticket_id": t.key, "merge_queued": true }),
                run: RunEffect::Continue,
                ops: vec![op],
            })
        })?;
        for op in &out.value.ops {
            if let Err(e) = self.run_op(*op) {
                tracing::warn!(op, error = %e, "merge failed; reconcile will retry");
            }
        }
        Ok(out.value)
    }

    pub(super) fn close_ticket(&self, ctx: &CallCtx, c: CloseTicket) -> Result<Outcome> {
        let pid = ctx.project;
        self.commit(ctx, |w| {
            require_lead(w, pid, &ctx.actor)?;
            let mut t = ticket_by_key(w, pid, &c.ticket_id)?;
            if t.status == TicketStatus::Resolved && !t.approved {
                return Err(invalid(format!(
                    "{} is not approved yet: review_ticket first",
                    t.key
                )));
            }
            t.close_reason = Some(c.reason.trim().to_owned());
            transition(
                w,
                &mut t,
                &ctx.actor,
                TicketStatus::Closed,
                &Guards {
                    close_reason: true,
                    ..Default::default()
                },
            )?;
            let mut ops = vec![];
            // When the project has a remote (the user added a repo), push the merged work; otherwise it stays local.
            if t.merged {
                let p = projects::get(w.tx, pid)?;
                let repo = self.repo(&p);
                if repo.remote_url("origin").is_some() {
                    ops.push(crate::effects::journal(
                        w,
                        &t,
                        Op::Push {
                            branch: p.main_branch.clone(),
                        },
                    )?);
                }
            }
            if workspaces::get(w.tx, t.id)?.is_some_and(|ws| !ws.removed) {
                ops.push(crate::effects::journal(
                    w,
                    &t,
                    Op::RemoveWorktree {
                        delete_branch: t.merged,
                    },
                )?);
            }
            plan_item_done(w, &t)?;
            Ok(Outcome {
                result: serde_json::json!({ "ticket_id": t.key, "status": t.status }),
                run: RunEffect::Continue,
                ops,
            })
        })
    }

    pub(super) fn cancel_ticket(&self, ctx: &CallCtx, c: CancelTicket) -> Result<Outcome> {
        let pid = ctx.project;
        self.commit(ctx, |w| {
            require_lead(w, pid, &ctx.actor)?;
            let mut t = ticket_by_key(w, pid, &c.ticket_id)?;
            let replacement = match &c.replaced_by {
                Some(k) => {
                    let r = ticket_by_key(w, pid, k)?;
                    if r.id == t.id || r.status == TicketStatus::Cancelled {
                        return Err(invalid(format!(
                            "{k} cannot carry {}'s work on: name an open or closed ticket other than it",
                            t.key
                        )));
                    }
                    Some(r)
                }
                None => None,
            };
            let was = t.status;
            t.close_reason = Some(c.reason.trim().to_owned());
            transition(
                w,
                &mut t,
                &ctx.actor,
                TicketStatus::Cancelled,
                &Guards::default(),
            )?;
            system_comment(w, &t, format!("Cancelled: {}", c.reason.trim()))?;
            // ADR-147: how the cancelled work is accounted for in its plan item.
            crate::coverage::record_cancel(w, &t, &ctx.actor, replacement.as_ref())?;
            // A run still working on the ticket stops (the runtime watches its ticket) and keeps the worktree until it
            // ends; its agent stays busy until then (`finish_run` sets it Idle), or the scheduler could start a
            // second run of the same agent beside it.
            let live_run = runs::unfinished(w.tx)?
                .iter()
                .any(|r| r.ticket == Some(t.id));
            if let Some(a) = t.assignee {
                if !live_run && matches!(was, TicketStatus::InProgress | TicketStatus::Pending) {
                    set_agent_status(w, a, AgentStatus::Idle)?;
                }
            }
            // A merge decided before the cancel must not land the cancelled work in main.
            crate::effects::drop_pending_merges(w, &t, "the ticket was cancelled")?;
            let mut ops = vec![];
            if !live_run && workspaces::get(w.tx, t.id)?.is_some_and(|ws| !ws.removed) {
                // Keep the unmerged branch (decision 11: delete the worktree, keep branches that were not merged).
                ops.push(crate::effects::journal(
                    w,
                    &t,
                    Op::RemoveWorktree {
                        delete_branch: false,
                    },
                )?);
            }
            // Open tickets that depend on this one can never start now (the Lead's inbox keeps listing them).
            let mut blocked = Vec::new();
            for id in tickets::dependents(w.tx, t.id)? {
                let d = tickets::get(w.tx, id)?;
                if !d.status.is_terminal() {
                    blocked.push(d.key);
                }
            }
            Ok(Outcome {
                result: serde_json::json!({ "ticket_id": t.key, "status": t.status, "blocked_dependents": blocked }),
                run: RunEffect::Continue,
                ops,
            })
        })
    }
}
