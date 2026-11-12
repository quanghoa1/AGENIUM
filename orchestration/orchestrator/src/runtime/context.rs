//! The inputs of a run's work order: the agent's soul, role, skills and memory, its tools, the loop settings, the
//! project's state and the user's decisions.

use super::*;

/// The work order's shell line. Windows PowerShell 5.1 puts a byte-order mark before UTF-8 text and writes UTF-16 by
/// default; the file then fails exact checks (a first line, a shebang, a parser) while looking right when read back
/// (Codex agents in the memory-reuse benchmark, docs/memory-eval.md).
pub(super) fn shell_text(shell: &agenium_sandbox::Shell) -> String {
    let mut s = shell.describe();
    if let agenium_sandbox::Shell::PowerShell(p) = shell {
        if p.file_name()
            .is_some_and(|n| n.eq_ignore_ascii_case("powershell.exe"))
        {
            s.push_str(
                "; it is Windows PowerShell 5.1: Set-Content/Out-File -Encoding utf8 start the file with a byte-order \
                 mark and `>` writes UTF-16, so write files with your file tool or \
                 [IO.File]::WriteAllText(path, text) (UTF-8 without a BOM)",
            );
        }
        // Inside an MXC container PowerShell's Remove-Item and cmd's del fail with "Access is denied" while the .NET
        // and Python calls delete (measured 2026-10-27): agents gave up on a deletion or asked for permission.
        if agenium_sandbox::detect().backend == Some(agenium_sandbox::Backend::WindowsMxc) {
            s.push_str(
                "; in this sandbox Remove-Item and del fail with \"Access is denied\": delete files with [IO.File]::Delete(path) and folders with [IO.Directory]::Delete(path, $true), or with your file tool (apply_patch \"*** Delete File\")",
            );
        }
    }
    // Seatbelt cannot be applied inside Seatbelt: SwiftPM sandboxes its manifest builds with sandbox-exec and stops
    // with "sandbox_apply: Operation not permitted" (macOS kit v9, 2026-11-12).
    if agenium_sandbox::detect().backend == Some(agenium_sandbox::Backend::Seatbelt) {
        s.push_str(
            "; commands already run inside macOS's sandbox, which cannot be nested: pass --disable-sandbox to swift build, swift test and swift run",
        );
    }
    // ADR-120: what an agent may install itself, and what goes to the user.
    s.push_str(
        "; install project dependencies inside the worktree (a virtual environment, node_modules, the build's own fetch). Installing software on the user's machine (winget, choco, apt, brew, npm -g, pip outside a virtual environment, cargo install, an installer script) waits for the user's approval: if a tool is missing, ask instead of working around it silently",
    );
    s
}

impl Runtime {
    /// The user's answers on the project's other tickets, newest first (ADR-120): what the user decided for one ticket
    /// (a tool not to install, a convention, a choice) holds for the next, so the agent does not ask again.
    pub(super) fn user_decisions(&self, project: ProjectId, own: &TicketKey) -> Result<String> {
        use agenium_core::comment::CommentBody;
        let answers = self
            .app
            .store()
            .read(|c| comments::user_answers(c, project, 8))?;
        let cut = |s: &str, n: usize| -> String {
            let s = s.trim();
            match s.char_indices().nth(n) {
                Some((i, _)) => format!("{}…", &s[..i]),
                None => s.to_owned(),
            }
        };
        let lines: Vec<String> = answers
            .iter()
            .filter(|(a, _)| &a.ticket_key != own)
            .take(5)
            .filter_map(|(a, q)| {
                let CommentBody::Answer(ans) = &a.body else {
                    return None;
                };
                let asked = match q.as_ref().map(|q| &q.body) {
                    Some(CommentBody::Question(q)) => {
                        format!("asked \"{}\" — ", cut(&q.question, 300))
                    }
                    _ => String::new(),
                };
                Some(format!(
                    "- On {}: {asked}the user answered: {}",
                    a.ticket_key,
                    cut(&ans.resolution, 400)
                ))
            })
            .collect();
        if lines.is_empty() {
            return Ok(String::new());
        }
        Ok(format!(
            "\n## The user's decisions on other tickets of this project\nThey hold for this ticket too: do not ask them \
             again.\n{}\n",
            lines.join("\n")
        ))
    }

    pub(super) fn agent_memory(&self, agent: AgentId) -> Result<Arc<agenium_store::AgentStore>> {
        let s = self
            .app
            .agent_store(agent, self.embedder.dim(), self.embedder.model_id())?;
        // Text is the source of truth: vectors from another embedder are rebuilt (model installed later, switch).
        match agenium_memory::refresh_vectors(&s, self.embedder.as_ref()) {
            Ok(0) => {}
            Ok(n) => tracing::info!(%agent, n, "re-embedded memory with the current embedder"),
            Err(e) => tracing::warn!(%agent, error = %e, "could not re-embed memory"),
        }
        Ok(s)
    }

    pub(super) fn soul_and_role(&self, agent: &agenium_store::core::agents::Agent) -> String {
        let dir = self.app.layout().agent_dir(&agent.id);
        let soul = std::fs::read_to_string(dir.join("SOUL.md")).unwrap_or_else(|_| format!("You are {}, a careful and honest {} agent. You prefer small, verified steps and say clearly what you did not do.", agent.name, agent.role));
        let role =
            std::fs::read_to_string(self.app.layout().roles().join(&agent.role).join("ROLE.md"))
                .unwrap_or_default();
        let mut s = format!("# Identity (SOUL.md)\n{}", soul.trim());
        if !role.trim().is_empty() {
            s.push_str(&format!("\n\n# Role (ROLE.md)\n{}", role.trim()));
        }
        s.push_str(&format!("\n\n# Skills\n{}", self.skills(agent).index()));
        s
    }

    /// Skills of an agent (role skills + its own; its binding filters them).
    pub fn skills(&self, agent: &agenium_store::core::agents::Agent) -> SkillSet {
        let set =
            self.app
                .skill_registry()
                .for_agent(&agent.id.to_string(), &agent.role, &agent.skills);
        for p in &set.problems {
            tracing::warn!(dir = %p.dir.display(), error = %p.error, "skipping invalid skill");
        }
        set
    }

    pub(super) fn manifest(
        &self,
        agent: &agenium_store::core::agents::Agent,
        solo: bool,
        ctx: &ToolCtx,
        tools: &[ToolSpec],
        team: Option<serde_json::Value>,
        allow: &[String],
    ) -> (String, String) {
        let sandbox = agenium_sandbox::detect();
        let approval = self.app.config().sandbox.approval;
        // What commands really get: on a Linux host without a private network they run without one (ADR-95).
        let asked = self.app.config().sandbox.network && sandbox.backend.is_some();
        let available = agenium_sandbox::network_available();
        let net_on = asked && available.is_ok();
        let net_note = match (asked, available) {
            (true, Ok(())) => "network on".to_owned(),
            (true, Err(why)) => format!("no network on this machine: {why}"),
            (false, _) => "no network".to_owned(),
        };
        let unsandboxed = self.app.config().sandbox.runs_unsandboxed();
        let sandbox_text = match (sandbox.backend, approval) {
            (None, agenium_config::ApprovalPolicy::Never | agenium_config::ApprovalPolicy::Sandboxed) if unsandboxed => format!("none: {}; the user chose to run without one, so shell commands run directly on the user's computer without asking (nothing protects it: stay inside your worktree)", sandbox.reason),
            (Some(b), agenium_config::ApprovalPolicy::Sandboxed) => format!("{b:?}, commands run without asking inside the sandbox (writes stay in your worktree, {net_note}); a command the sandbox blocks {}, and shell's outside_sandbox asks that up front for one that cannot work inside (credentials, docker, a server on localhost). Pushing is not yours to do: Agenium pushes after the review and the merge", if self.app.config().sandbox.auto_escalate { "runs again outside it by the user's standing approval" } else { "is offered to the user to run outside it" }),
            (Some(b), agenium_config::ApprovalPolicy::Never) => format!("{b:?}, commands run without asking inside the sandbox (writes stay in your worktree, {net_note}); nothing leaves it"),
            (None, agenium_config::ApprovalPolicy::Sandboxed) => format!("none: {}; every shell command that is not on the project allowlist is shown to the user for approval", sandbox.reason),
            (_, agenium_config::ApprovalPolicy::Never) => format!("none: {}; shell commands are refused", sandbox.reason),
            (_, agenium_config::ApprovalPolicy::OnRequest) => "the user reviews commands for this project: every shell command that is not on the project allowlist is shown to the user for approval".into(),
        };
        let m = Manifest {
            agent: agent.name.clone(),
            role: agent.role.clone(),
            solo,
            os: std::env::consts::OS.into(),
            shell: shell_text(ctx.executor.shell()),
            sandbox: sandbox_text,
            network: match (net_on, allow.is_empty()) {
                (true, true) => {
                    "commands may reach the network; fetch has no allowed domains".into()
                }
                (true, false) => format!(
                    "commands may reach the network; fetch may reach {}",
                    allow.join(", ")
                ),
                (false, true) => "off; fetch has no allowed domains".into(),
                (false, false) => format!("off for commands; fetch may reach {}", allow.join(", ")),
            },
            worktree: ctx.root.display().to_string(),
            allowed_paths: vec![],
            tools: tools.iter().map(|t| t.name.clone()).collect(),
            team,
            extra: ctx.capabilities.clone(),
        };
        (prompt::manifest_text(&m), prompt::run_context_text(&m))
    }

    pub(super) fn tool_ctx(
        &self,
        root: &Path,
        run: RunId,
        project: ProjectId,
        ticket: Option<TicketKey>,
        agent: &str,
        allow: Vec<String>,
    ) -> ToolCtx {
        let gate = Arc::new(RunGate {
            app: self.app.clone(),
            project,
            ticket,
            agent: agent.to_owned(),
            approver: self.approver.clone(),
        });
        let mut ctx = ToolCtx::new(root, &run.to_string(), gate);
        let rt = self.app.config().runtime;
        ctx.limits = agenium_tools::Limits {
            max_chars: rt.tool_output_max_chars as usize,
            max_lines: rt.tool_output_max_lines as usize,
            shell_timeout: std::time::Duration::from_secs(rt.shell_timeout_secs as u64),
            shell_timeout_max: std::time::Duration::from_secs(rt.shell_timeout_max_secs as u64),
        };
        ctx.fetch_allow = allow;
        ctx
    }

    pub(super) fn loop_cfg(
        &self,
        agent: &agenium_store::core::agents::Agent,
        cache_key: String,
    ) -> LoopConfig {
        let rt = self.app.config().runtime;
        LoopConfig {
            model: agent.model.model.clone(),
            effort: agent.model.effort.clone(),
            max_tokens: agent.model.max_tokens.unwrap_or(32_000),
            max_steps: rt.max_steps_per_run,
            same_call_limit: rt.loop_detect_same_call,
            error_limit: rt.tool_error_stop,
            nudge_limit: 2,
            retry: self.retry,
            cache_key: Some(cache_key),
            long_cache: true,
            context_window: super::models::DEFAULT_WINDOW,
            max_compactions: 2,
            vision: false,
        }
    }

    /// The protocol tools whose calls Claude's and OpenAI's APIs decode against the schema (ADR-144): the forms that
    /// move a ticket, a plan or a spec. Together they stay within Claude's limits for strict schemas in one request
    /// (23 of 24 optional parameters, 6 of 16 with alternatives, 7 of 20 tools), whatever the role.
    const STRICT: [ProtocolTool; 7] = [
        ProtocolTool::SubmitReport,
        ProtocolTool::ReviewTicket,
        ProtocolTool::CreateTicket,
        ProtocolTool::CreatePlan,
        ProtocolTool::ProposeSpec,
        ProtocolTool::AssignTicket,
        ProtocolTool::SubmitPlanDraft,
    ];

    pub(super) fn protocol_specs(
        role: RoleKind,
        solo: bool,
        exclude: &[ProtocolTool],
    ) -> (Vec<ToolSpec>, Vec<String>) {
        let tools: Vec<ProtocolTool> = ProtocolTool::for_role(role, solo)
            .into_iter()
            .filter(|t| !exclude.contains(t))
            .collect();
        let specs = tools
            .iter()
            .map(|t| ToolSpec {
                name: t.name().into(),
                description: t.description().into(),
                parameters: t.parameters(),
                strict: Self::STRICT.contains(t),
            })
            .collect();
        (specs, tools.iter().map(|t| t.name().to_owned()).collect())
    }

    /// Characters a checkpoint note needs to stand for the run's conversation.
    const MIN_NOTE: usize = 40;

    /// Previous conversation of the ticket for continuation: the last finished run's messages, unless it ended with a
    /// checkpoint (then a fresh start with the note), and when that run started.
    pub(super) fn previous(
        &self,
        agent: AgentId,
        ticket: TicketId,
        mem: &agenium_store::AgentStore,
    ) -> Result<(Vec<Message>, Option<String>, Option<String>)> {
        let rs = self.app.store().read(|c| runs::for_ticket(c, ticket))?;
        let Some(last) = rs
            .iter()
            .rev()
            .find(|r| r.agent == agent && r.ended_at.is_some())
        else {
            return Ok((vec![], None, None));
        };
        let outcome = last.outcome.clone().unwrap_or_default();
        if outcome.starts_with("Checkpoint") {
            let note = self
                .app
                .store()
                .read(|c| runs::log_entries(c, last.id))?
                .into_iter()
                .rev()
                .find(|e| e.kind == "checkpoint")
                .and_then(|e| e.data["summary"].as_str().map(str::to_owned));
            // A checkpoint with next to no note (a CLI cut off by its time limit says nothing) would leave the next
            // run with nothing of its work: its conversation is resumed instead, folded to fit if it must be
            // (ADR-155).
            if note
                .as_deref()
                .is_some_and(|n| n.trim().len() >= Self::MIN_NOTE)
            {
                return Ok((vec![], note, Some(last.started_at.clone())));
            }
        }
        let msgs: Vec<Message> = mem
            .load_messages(&last.id.to_string())?
            .into_iter()
            .filter_map(|v| serde_json::from_value(v).ok())
            .collect();
        Ok((msgs, None, Some(last.started_at.clone())))
    }

    /// Compact view of the plan and tickets for the Lead's chat context.
    pub(super) fn project_state_text(&self, project: ProjectId) -> Result<String> {
        use agenium_store::core::plans;
        let (spec, plan) = self.app.store().read(|c| {
            Ok((
                plans::latest_spec(c, project)?,
                plans::latest_plan(c, project)?,
            ))
        })?;
        let mut s = String::new();
        // The spec in full: the Lead writes tickets from it, and workers only see what the tickets carry.
        match spec {
            Some(sp) => {
                s.push_str(&format!(
                    "Spec v{} ({}):\nGoals: {}\n",
                    sp.version,
                    if sp.locked_at.is_some() {
                        "locked"
                    } else {
                        "waiting for the user"
                    },
                    sp.body.goals.join("; ")
                ));
                for (label, list) in [
                    ("Scope", &sp.body.scope),
                    ("Out of scope", &sp.body.out_of_scope),
                    ("Constraints", &sp.body.constraints),
                ] {
                    if !list.is_empty() {
                        s.push_str(&format!("{label}: {}\n", list.join("; ")));
                    }
                }
                for r in &sp.body.requirements {
                    s.push_str(&format!("- {}: {}\n", r.id, r.text));
                    for a in &r.acceptance {
                        s.push_str(&format!("  - {}: {}\n", a.id, a.text));
                    }
                }
            }
            None => s.push_str("No requirement spec yet.\n"),
        }
        match plan {
            Some(p) => {
                s.push_str(&format!(
                    "Plan v{} ({}):\n",
                    p.version,
                    if p.approved_by.is_some() {
                        "approved"
                    } else {
                        "waiting for the user"
                    }
                ));
                for it in &p.body.items {
                    s.push_str(&format!(
                        "- {} {} (traces {})\n",
                        it.id,
                        it.title,
                        it.trace_to.join(",")
                    ));
                }
            }
            None => s.push_str("No plan yet.\n"),
        }
        if let Some(p) = self.app.store().read(|c| plans::current_plan(c, project))? {
            if let Some(d) = self.app.plan_draft(project, p.body.spec_version)? {
                s.push_str(&format!(
                    "Planner's rationale for the initial plan: {}\n",
                    d.rationale
                ));
            }
        }
        for p in self.app.role_proposals(project)? {
            use agenium_store::core::roles::ProposalStatus as S;
            s.push_str(&format!(
                "Role proposal {} ({}): {}\n",
                p.id,
                p.body.role_id,
                match (p.status, p.agent) {
                    (S::Pending, _) => "waiting for the user".to_owned(),
                    (S::Approved, Some(a)) => format!(
                        "approved, {} joined the team",
                        self.app.agent(a).map(|x| x.name).unwrap_or_default()
                    ),
                    (S::Approved, None) => "approved".to_owned(),
                    (S::Declined, _) => format!(
                        "declined by the user{}",
                        p.note.map(|n| format!(": {n}")).unwrap_or_default()
                    ),
                }
            ));
        }
        let ts = self.app.tickets(project)?;
        if ts.is_empty() {
            s.push_str("No tickets yet.");
        }
        for t in ts {
            s.push_str(&format!(
                "- {} [{}] {} (plan item {}{})\n",
                t.key,
                t.status,
                t.title,
                t.plan_item,
                t.assignee
                    .map(|a| format!(", assignee {a}"))
                    .unwrap_or_default()
            ));
        }
        Ok(s)
    }
}

#[cfg(test)]
mod tests {
    use agenium_sandbox::Shell;

    /// ADR-144: every strict protocol tool has a strict form, and all of them fit Claude's limits in one request.
    #[test]
    fn the_strict_protocol_tools_fit_claude_s_limits_together() {
        let costs: Vec<_> = super::Runtime::STRICT
            .iter()
            .map(|t| agenium_llm::strict::fit(&t.parameters()).map(|(_, c)| c))
            .collect();
        assert!(costs.iter().all(Option::is_some), "{costs:?}");
        assert!(
            agenium_llm::strict::claude_admits(&costs)
                .iter()
                .all(|x| *x),
            "{costs:?}"
        );
    }

    #[test]
    fn windows_powershell_5_is_named_with_its_byte_order_mark() {
        // Joined, not a `C:\…` literal: on Linux and macOS a backslash is not a separator (VM run, 2026-10-26).
        let dir = std::path::Path::new("WindowsPowerShell").join("v1.0");
        let old = super::shell_text(&Shell::PowerShell(dir.join("powershell.exe")));
        assert!(
            old.contains("byte-order mark") && old.contains("WriteAllText"),
            "{old}"
        );
        let new = super::shell_text(&Shell::PowerShell(
            std::path::Path::new("PowerShell")
                .join("7")
                .join("pwsh.exe"),
        ));
        assert!(!new.contains("byte-order mark"), "{new}");
    }
}
