//! The team's planner in a round (ADR-59): a plan drafted for the latest approved spec.

use super::*;

impl Runtime {
    /// Runs the team's planner when the latest spec is approved, there is no plan and no draft for that spec yet
    /// (ADR-59). Two failed runs for the same spec stop it and go to the user.
    pub(super) async fn planner_round(&self, project: ProjectId) -> Result<Option<RunSummary>> {
        let Some(planner) = self.app.planner(project)? else {
            return Ok(None);
        };
        use agenium_store::core::plans;
        let (spec, any_plan) = self.app.store().read(|c| {
            Ok((
                plans::latest_spec(c, project)?,
                plans::latest_plan(c, project)?.is_some(),
            ))
        })?;
        let Some(spec) = spec.filter(|s| s.locked_at.is_some()) else {
            return Ok(None);
        };
        if any_plan
            || planner.status != AgentStatus::Idle
            || self.app.plan_draft(project, spec.version)?.is_some()
        {
            return Ok(None);
        }
        let key = (project, spec.version);
        let failures = *self
            .planner_failures
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&key)
            .unwrap_or(&0);
        if failures >= MAX_FAILED_RUNS {
            return Ok(None);
        }
        // A run that fails before its loop counts like one without a draft (else it is retried every round); one the
        // user stopped does not count.
        let (s, outcome) = match self.plan_draft_run(project, &planner, &spec).await {
            Ok(s) => {
                let o = s.outcome.clone();
                (Some(s), o)
            }
            Err(e) => {
                tracing::warn!(error = %e, "planner run failed");
                (None, format!("Error:{e}"))
            }
        };
        if outcome != "Cancelled" && self.app.plan_draft(project, spec.version)?.is_none() {
            let n = failures + 1;
            self.planner_failures
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .insert(key, n);
            if n >= MAX_FAILED_RUNS {
                self.app.store().commit(None, "system", |w| {
                    w.emit(DomainEvent::NeedsUser {
                        ticket: None,
                        reason: format!(
                            "the planner {} could not draft plan v1 for spec v{} ({}); the Lead can create the plan in chat",
                            planner.name, spec.version, outcome
                        ),
                    })
                    .map_err(OrchError::from)
                })?;
            }
        }
        Ok(s)
    }

    /// The planner's run: read the locked spec and the repository, submit a plan draft with its rationale.
    async fn plan_draft_run(
        &self,
        project: ProjectId,
        planner: &agenium_store::core::agents::Agent,
        spec: &agenium_store::core::plans::SpecRow,
    ) -> Result<RunSummary> {
        let mut opened = OpenRun::new(&self.app);
        let r = self
            .plan_draft(project, planner, spec, &mut opened.run)
            .await;
        self.end_on_error(opened.take(), r)
    }

    async fn plan_draft(
        &self,
        project: ProjectId,
        planner: &agenium_store::core::agents::Agent,
        spec: &agenium_store::core::plans::SpecRow,
        opened: &mut Option<RunId>,
    ) -> Result<RunSummary> {
        let p = self.app.project(project)?;
        let root = crate::project_create::checkout_dir(std::path::Path::new(&p.path));
        let run = self.open_run(planner, None, RunId::new())?;
        *opened = Some(run);
        // The roles the plan can use: the team's workers except the planner.
        let roles: Vec<String> = self
            .app
            .team_of_project(project)?
            .map(|(_, m)| m)
            .unwrap_or_default()
            .into_iter()
            .filter(|m| m.role_kind == RoleKind::Worker && m.id != planner.id)
            .map(|m| {
                let paths = self.app.role_allowed_paths(&m.role);
                format!(
                    "- {} (role {}, tags {}, writes {})",
                    m.name,
                    m.role,
                    m.tags.join(", "),
                    if paths.is_empty() {
                        "anywhere".to_owned()
                    } else {
                        paths.join(", ")
                    }
                )
            })
            .collect();
        let task = format!(
            "Draft plan v1 for the approved requirement spec v{v}.\n\n## Spec v{v}\n{spec}\n\n## Team members who will do the work\n{roles}\n\n\
             Read the repository with your read-only tools first. Then call submit_plan_draft once: milestones; items with \
             acceptance criteria (ids AC-n; at least one per item states what a user or caller observes on the real path, \
             'running X prints Y', never just 'tests pass'), trace_to (requirement ids of this spec), role (one of the roles above), \
             depends_on (item ids), estimate and scope_paths (inside the paths its role writes; items touching the same \
             paths must depend on each other); and the rationale of the split. Split for parallel work: each worker takes one ticket at a time, so give every worker an item it can start early and let items depend on each other only where one needs the other's result or they share paths; a chain through one worker leaves the others waiting. You do not create \
             tickets; the Lead reviews your draft.",
            v = spec.version,
            spec = serde_json::to_string_pretty(&spec.body).unwrap_or_default(),
            roles = roles.join("\n"),
        );
        // What the planner remembers about plans like this (ADR-155).
        let memory = self
            .recall_text(
                planner.id,
                project,
                run,
                None,
                serde_json::to_string(&spec.body).unwrap_or_default(),
            )
            .await?;
        let task = format!("{memory}{task}");
        let soul = self.soul_and_role(planner);
        let started = Instant::now();
        let builder = |ctx: &ToolCtx, specs: &[ToolSpec]| -> Result<WorkOrder> {
            let (manifest, run_context) =
                self.manifest(planner, false, ctx, specs, None, &ctx.fetch_allow);
            Ok(WorkOrder {
                system: vec![
                    SystemBlock {
                        text: prompt::system_rules(false),
                        cache: false,
                    },
                    SystemBlock {
                        text: soul.clone(),
                        cache: true,
                    },
                    SystemBlock {
                        text: manifest,
                        cache: false,
                    },
                ],
                messages: vec![Message::user_text(format!("{run_context}\n\n{task}"))],
            })
        };
        let read_only: Vec<Arc<dyn Tool>> = super::read_only_tools();
        let others: Vec<ProtocolTool> = ProtocolTool::ALL
            .iter()
            .copied()
            .filter(|t| !matches!(t, ProtocolTool::SubmitPlanDraft | ProtocolTool::LoadSkill))
            .collect();
        let res = self
            .execute(
                RunKind::Plan,
                project,
                planner,
                false,
                run,
                None,
                &root,
                builder,
                read_only,
                &others,
            )
            .await;
        let (report, cost) = res?;
        let usage = bridge::summary(&report.usage, cost, started.elapsed().as_millis() as u64);
        let label = report.outcome.label();
        self.app.finish_run(run, &label, report.steps, &usage)?;
        self.agent_state_after(planner, None, &report.outcome)?;
        self.app
            .run_log(run, "run", serde_json::json!({ "kind": "plan" }))?;
        Ok(RunSummary {
            run,
            kind: "plan".into(),
            ticket: None,
            agent: planner.name.clone(),
            outcome: label,
            steps: report.steps,
            usage,
            text: String::new(),
        })
    }
}
