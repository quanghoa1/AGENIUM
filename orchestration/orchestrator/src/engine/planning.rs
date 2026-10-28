//! Spec and plan intents (decision 3h): the Lead proposes the spec, the user approves it; the plan (or the planner's
//! draft) is created, revised and approved. Split out of `engine.rs`.

use super::*;

impl App {
    pub(super) fn propose_spec(
        &self,
        ctx: &CallCtx,
        spec: agenium_core::plan::RequirementSpec,
    ) -> Result<Outcome> {
        let project = ctx.project;
        self.commit(ctx, |w| {
            let team = require_lead(w, project, &ctx.actor)?;
            let version = plans::latest_spec(w.tx, project)?.map_or(1, |s| s.version + 1);
            // Solo self-approves (decision 6); a team's spec waits for the user.
            let solo = team.as_ref().is_some_and(|t| t.solo) || ctx.actor == Actor::User;
            let row = SpecRow {
                id: ulid::Ulid::new().to_string(),
                project,
                version,
                body: spec,
                proposed_by: ctx.actor.label(),
                approved_by: solo.then(|| ctx.actor.label()),
                locked_at: solo.then(now_iso),
            };
            plans::insert_spec(w.tx, &row)?;
            w.emit(DomainEvent::SpecProposed { project, version })?;
            if solo {
                w.emit(DomainEvent::SpecApproved {
                    project,
                    version,
                    by: ctx.actor.label(),
                })?;
            } else {
                w.emit(DomainEvent::NeedsUser {
                    ticket: None,
                    reason: format!("requirement spec v{version} waits for your approval"),
                })?;
            }
            Ok(Outcome::ok(
                serde_json::json!({ "spec_version": version, "locked": solo }),
            ))
        })
    }

    /// The user approves (locks) a spec version.
    pub fn approve_spec(&self, project: ProjectId, version: u32) -> Result<()> {
        self.store.commit(None, "user", |w| -> Result<()> {
            plans::approve_spec(w.tx, project, version, "user", &now_iso())?;
            w.emit(DomainEvent::SpecApproved {
                project,
                version,
                by: "user".into(),
            })?;
            Ok(())
        })?;
        Ok(())
    }

    pub(super) fn create_plan(
        &self,
        ctx: &CallCtx,
        plan: agenium_core::plan::TicketPlan,
    ) -> Result<Outcome> {
        let project = ctx.project;
        // ADR-59: a team with a planner builds plan v1 from its draft. A Lead asked in chat used to plan alone and
        // the planner (often the stronger model, chosen for this) never ran; it waits for the draft unless the
        // planner gave up.
        if let Some(planner) = self.planner(project)? {
            if self.plan_draft(project, plan.spec_version)?.is_none()
                && !self.planner_gave_up(&planner, project, plan.spec_version)?
            {
                return Err(invalid(format!(
                    "{} drafts plan v1 for spec v{} first: its draft comes to your inbox as a Planning item, then you \
create the plan from it. Tell the user the plan is being drafted",
                    planner.name, plan.spec_version
                )));
            }
        }
        self.commit(ctx, |w| {
            let team = require_lead(w, project, &ctx.actor)?;
            let spec = plans::spec(w.tx, project, plan.spec_version)?.ok_or_else(|| {
                invalid(format!(
                    "spec v{} does not exist; propose_spec first",
                    plan.spec_version
                ))
            })?;
            if spec.locked_at.is_none() {
                return Err(invalid(format!(
                    "spec v{} is not approved yet",
                    spec.version
                )));
            }
            check_trace(&spec, &plan.items)?;
            check_roles(w, team.as_ref(), &plan.items)?;
            if plans::latest_plan(w.tx, project)?.is_some() {
                return Err(invalid("a plan already exists; use revise_plan"));
            }
            // The user agreed on the spec: the plan built from it is the team's work, approved by the Lead (ADR-63).
            let row = PlanRow {
                id: ulid::Ulid::new().to_string(),
                project,
                version: 1,
                spec_version: plan.spec_version,
                reason: None,
                diff: None,
                body: plan,
                proposed_by: ctx.actor.label(),
                approved_by: Some(ctx.actor.label()),
            };
            plans::insert_plan(w.tx, &row)?;
            w.emit(DomainEvent::PlanProposed {
                project,
                version: 1,
                needs_user: false,
            })?;
            w.emit(DomainEvent::PlanApproved {
                project,
                version: 1,
                by: ctx.actor.label(),
            })?;
            Ok(Outcome::ok(
                serde_json::json!({ "plan_version": 1, "approved": true }),
            ))
        })
    }

    /// The planner's draft of plan v1 (ADR-59): only the team's planner, only for the latest locked spec and only
    /// while there is no plan. Stored as a `PlanDrafted` event; the Lead's inbox shows it.
    pub(super) fn submit_plan_draft(
        &self,
        ctx: &CallCtx,
        d: agenium_core::plan::PlanDraft,
    ) -> Result<Outcome> {
        let project = ctx.project;
        let Actor::Worker(agent) = ctx.actor else {
            return Err(denied("only the team's planner submits a plan draft"));
        };
        if !self.is_planner_role(&self.agent(agent)?.role) {
            return Err(denied("only the team's planner submits a plan draft"));
        }
        self.commit(ctx, |w| {
            let spec = plans::latest_spec(w.tx, project)?.ok_or_else(|| invalid("there is no spec yet"))?;
            if spec.locked_at.is_none() || spec.version != d.spec_version {
                return Err(invalid(format!("draft for spec v{} only (the latest approved spec)", spec.version)));
            }
            if plans::latest_plan(w.tx, project)?.is_some() {
                return Err(invalid("a plan already exists; the Lead owns it now"));
            }
            check_trace(&spec, &d.items)?;
            check_roles(w, agents::teams_of_project(w.tx, project)?.first(), &d.items)?;
            w.emit(DomainEvent::PlanDrafted { project, spec_version: d.spec_version, by: agent, draft: d })?;
            Ok(Outcome {
                result: serde_json::json!({ "draft_for_spec": spec.version, "next": "the Lead reviews your draft and creates the plan; your work is done" }),
                run: RunEffect::End,
                ops: vec![],
            })
        })
    }

    /// Whether the planner ended `MAX_FAILED_RUNS` runs since `spec_version` was approved without a draft (the user
    /// was told the Lead can plan instead). A run the user stopped does not count.
    pub(super) fn planner_gave_up(
        &self,
        planner: &agents::Agent,
        project: ProjectId,
        spec_version: u32,
    ) -> Result<bool> {
        let Some(locked) = self
            .store
            .read(|c| plans::spec(c, project, spec_version))?
            .and_then(|s| s.locked_at)
        else {
            return Ok(false);
        };
        let failed = self
            .store
            .read(|c| runs::for_agent(c, planner.id, 20))?
            .into_iter()
            .filter(|r| r.started_at >= locked)
            .filter(|r| r.ended_at.is_some() && r.outcome.as_deref() != Some("Cancelled"))
            .count();
        Ok(failed >= crate::runtime::MAX_FAILED_RUNS)
    }

    /// The planner's latest draft for `spec_version`.
    pub fn plan_draft(
        &self,
        project: ProjectId,
        spec_version: u32,
    ) -> Result<Option<agenium_core::plan::PlanDraft>> {
        let pid = project.to_string();
        Ok(self
            .store
            .read(|c| agenium_store::core::events::recent_of_kind(c, "PlanDrafted", 50))?
            .into_iter()
            .filter(|e| {
                e.payload["project"] == pid.as_str() && e.payload["spec_version"] == spec_version
            })
            .find_map(|e| serde_json::from_value(e.payload["draft"].clone()).ok()))
    }

    pub(super) fn revise_plan(&self, ctx: &CallCtx, rev: PlanRevision) -> Result<Outcome> {
        let project = ctx.project;
        self.commit(ctx, |w| {
            let team = require_lead(w, project, &ctx.actor)?;
            let base = plans::latest_plan(w.tx, project)?.ok_or_else(|| invalid("no plan to revise; use create_plan"))?;
            if base.version != rev.base_version {
                return Err(OrchError::Conflict(format!("base_version {} is stale; the latest plan is v{}", rev.base_version, base.version)));
            }
            // A revision follows the newest approved spec (the spec may have grown since the base plan).
            let spec = match plans::latest_spec(w.tx, project)? {
                Some(s) if s.locked_at.is_some() && s.version >= base.body.spec_version => s,
                _ => plans::spec(w.tx, project, base.body.spec_version)?.ok_or_else(|| invalid("plan spec missing"))?,
            };
            check_trace(&spec, &rev.items)?;
            check_roles(w, team.as_ref(), &rev.items)?;
            // Items that have tickets must keep existing (shrink scope, never drop work silently).
            let all = tickets::list(w.tx, project, &Default::default())?;
            for t in &all {
                if !t.status.is_terminal() && !rev.items.iter().any(|i| i.id == t.plan_item) {
                    return Err(invalid(format!("plan item {} still has open ticket {}", t.plan_item, t.key)));
                }
            }
            // A done item (its tickets ended, one closed) keeps its work: new work under its id would count as done
            // by the old tickets, and nobody would do it. One the revision only retyped (case, spacing, punctuation)
            // gets its text back silently; one with other words is refused with the text to keep, so the Lead can
            // copy it instead of retyping a long plan until its run gives up (release check 2026-10-27).
            let mut items = rev.items.clone();
            let mut versions: Vec<PlanRow> = Vec::new();
            for new in items.iter_mut() {
                let of: Vec<_> = all.iter().filter(|t| t.plan_item == new.id && t.status != TicketStatus::Cancelled).collect();
                let done = of.iter().all(|t| t.status.is_terminal()) && of.iter().any(|t| t.status == TicketStatus::Closed);
                let same = |i: &agenium_core::plan::PlanItem| i.id == new.id && i.title == new.title && i.acceptance == new.acceptance;
                if !done || base.body.items.iter().any(same) {
                    continue;
                }
                if versions.is_empty() {
                    for v in 1..base.version {
                        versions.extend(plans::plan(w.tx, project, v)?);
                    }
                }
                if versions.iter().any(|p| p.body.items.iter().any(same)) {
                    continue;
                }
                let was = base
                    .body
                    .items
                    .iter()
                    .chain(versions.iter().rev().flat_map(|p| p.body.items.iter()))
                    .find(|i| i.id == new.id)
                    .cloned();
                let keys: Vec<String> = of.iter().map(|t| t.key.to_string()).collect();
                match was {
                    Some(was) if retyped(&was, new) => {
                        new.title = was.title;
                        new.acceptance = was.acceptance;
                    }
                    was => {
                        let keep = was.map(|w| {
                            let acs: Vec<String> = w.acceptance.iter().map(|a| format!("{} {:?}", a.id, a.text)).collect();
                            format!(": title {:?}, acceptance {}", w.title, acs.join("; "))
                        });
                        return Err(invalid(format!(
                            "plan item {} is done ({} closed): keep its title and acceptance as they are{} — and put the new work in a new item",
                            new.id,
                            keys.join(", "),
                            keep.unwrap_or_default()
                        )));
                    }
                }
            }
            let (diff, scope_change) = plan_diff(&base.body.items, &items);
            // Every item traces to the agreed spec, so a revision stays inside what the user agreed (ADR-63): the
            // Lead approves it and the user is informed. New requirements need a new spec, which the user approves.
            let self_approve = true;
            let version = base.version + 1;
            let row = PlanRow {
                id: ulid::Ulid::new().to_string(),
                project,
                version,
                spec_version: spec.version,
                reason: Some(rev.reason.clone()),
                diff: Some(diff.clone()),
                body: agenium_core::plan::TicketPlan { spec_version: spec.version, milestones: rev.milestones, items },
                proposed_by: ctx.actor.label(),
                approved_by: self_approve.then(|| ctx.actor.label()),
            };
            plans::insert_plan(w.tx, &row)?;
            w.emit(DomainEvent::PlanProposed { project, version, needs_user: !self_approve })?;
            if self_approve {
                w.emit(DomainEvent::PlanApproved { project, version, by: ctx.actor.label() })?;
            } else {
                w.emit(DomainEvent::NeedsUser { ticket: None, reason: format!("plan v{version} changes scope and waits for your approval: {}", rev.reason) })?;
            }
            Ok(Outcome::ok(serde_json::json!({ "plan_version": version, "approved": self_approve, "scope_change": scope_change, "diff": diff })))
        })
    }

    /// The user approves a pending plan version.
    pub fn approve_plan(&self, project: ProjectId, version: u32) -> Result<()> {
        self.store.commit(None, "user", |w| -> Result<()> {
            plans::approve_plan(w.tx, project, version, "user")?;
            w.emit(DomainEvent::PlanApproved {
                project,
                version,
                by: "user".into(),
            })?;
            Ok(())
        })?;
        Ok(())
    }
}
