//! Reconciling the store with the repository at start.

use super::*;

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ReconcileReport {
    pub actions: Vec<String>,
}

impl App {
    /// Startup reconcile (M0-10): finishes or replays every journaled git operation, closes runs that died with
    /// the previous process, restores missing worktrees of active tickets and removes orphan worktrees.
    pub fn reconcile(&self) -> Result<ReconcileReport> {
        let mut report = ReconcileReport::default();
        for row in self.store.read(ops::pending)? {
            let r = self.run_op(row.seq);
            report.actions.push(match r {
                Ok(()) => format!("replayed {} #{} for ticket {}", row.op, row.seq, row.ticket),
                Err(e) => format!("could not replay {} #{}: {e}", row.op, row.seq),
            });
        }
        // Runs cannot survive a restart: close them; their tickets get a continuation run from the scheduler.
        let dead = self.store.read(runs::unfinished)?;
        if !dead.is_empty() {
            self.store.commit(None, "system", |w| -> Result<()> {
                for r in &dead {
                    runs::finish(w.tx, r.id, &now_iso(), "Interrupted")?;
                    w.emit(DomainEvent::RunEnded {
                        run: r.id,
                        agent: r.agent,
                        outcome: "Interrupted".into(),
                    })?;
                }
                for a in agents::agents(w.tx)? {
                    if a.status == AgentStatus::Working {
                        agents::set_agent_status(w.tx, a.id, AgentStatus::Idle)?;
                        w.emit(DomainEvent::AgentStatusChanged {
                            agent: a.id,
                            from: AgentStatus::Working,
                            to: AgentStatus::Idle,
                        })?;
                    }
                }
                Ok(())
            })?;
            // What an interrupted run left in its worktree is the agent's work (ADR-147): recorded as the run's state,
            // so the next run goes on from it instead of the guard undoing it as a change made outside Agenium.
            for r in &dead {
                if let Some(t) = r.ticket {
                    let project = self.store.read(|c| tickets::get(c, t))?.project;
                    if let Err(e) = self.record_worktree_state(project, r.id) {
                        tracing::warn!(run = %r.id, error = %e, "worktree state of an interrupted run not recorded");
                    }
                }
            }
            report
                .actions
                .push(format!("closed {} interrupted run(s)", dead.len()));
        }
        // Messages to a Lead that a chat turn took when the process died: back in the queue (audit M9).
        let back = self
            .store
            .commit(None, "system", |w| {
                agenium_store::core::ledger::return_taken(w.tx)
            })?
            .value;
        if back > 0 {
            report.actions.push(format!(
                "put {back} message(s) to a Lead back in the queue (its chat turn was interrupted)"
            ));
        }
        for p in self.projects()? {
            if !Path::new(&p.path).is_dir() {
                report.actions.push(format!(
                    "project {} is missing at {} (left untouched)",
                    p.name, p.path
                ));
                continue;
            }
            let repo = self.repo(&p);
            let live = self.store.read(|c| workspaces::live_for_project(c, p.id))?;
            for ws in &live {
                let t = self.store.read(|c| tickets::get(c, ws.ticket))?;
                let busy = self
                    .store
                    .read(|c| ops::for_ticket(c, t.id))?
                    .iter()
                    .any(|o| o.state == "pending");
                if busy {
                    continue;
                }
                if t.status.is_terminal() {
                    let seq = self
                        .store
                        .commit(None, "system", |w| {
                            journal(
                                w,
                                &t,
                                Op::RemoveWorktree {
                                    delete_branch: t.merged,
                                },
                            )
                        })?
                        .value;
                    // A failure (a file still held on Windows) is journaled and retried later, never a reason not to
                    // open (audit M10, 2026-10-31: one held file kept Agenium from starting).
                    report.actions.push(match self.run_op(seq) {
                        Ok(()) => format!("removed worktree of finished ticket {}", t.key),
                        Err(e) => format!(
                            "worktree of finished ticket {} not removed yet (retried later): {e}",
                            t.key
                        ),
                    });
                } else if !Path::new(&ws.worktree).is_dir() {
                    let seq = self
                        .store
                        .commit(None, "system", |w| journal(w, &t, Op::CreateWorktree))?
                        .value;
                    report.actions.push(match self.run_op(seq) {
                        Ok(()) => format!("restored missing worktree of {}", t.key),
                        Err(e) => format!(
                            "missing worktree of {} not restored yet (retried later): {e}",
                            t.key
                        ),
                    });
                }
            }
            // Worktree directories that no live workspace owns.
            if let Ok(rd) = std::fs::read_dir(repo.worktrees_dir()) {
                for e in rd.filter_map(|e| e.ok()) {
                    let name = e.file_name().to_string_lossy().into_owned();
                    let owned = live.iter().any(|w| {
                        Path::new(&w.worktree)
                            .file_name()
                            .is_some_and(|f| f.to_string_lossy() == name)
                    });
                    if !owned {
                        // Tried again at the next start when it fails (a file still held).
                        if let Err(e) = repo.remove_worktree(&name) {
                            report.actions.push(format!(
                                "orphan worktree {name} not removed yet (tried again at the next start): {e}"
                            ));
                            continue;
                        }
                        agenium_sandbox::remove_run_temp_dir(&repo.worktree_path(&name));
                        report
                            .actions
                            .push(format!("removed orphan worktree {name}"));
                    }
                }
            }
        }
        Ok(report)
    }
}
