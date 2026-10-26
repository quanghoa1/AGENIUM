//! Git side effects, journal-first (decision 22, ADR-44): the deciding transaction inserts a `vcs_op` row and a
//! `VcsOp` event; the git work runs after commit; completion writes `VcsOpDone` and updates state. Every operation
//! is idempotent, so [`App::reconcile`] simply re-runs pending rows after a crash.

use std::path::Path;

use agenium_core::*;
use agenium_store::core::{agents, ops, projects, runs, tickets, workspaces, WriteCtx};
use agenium_vcs::{MergeOutcome, Repo};
use serde::{Deserialize, Serialize};

use crate::app::App;
use crate::engine::transition;
use crate::{OrchError, Result};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    CreateWorktree,
    /// Auto-commit at submit time; `intent` is the commit trailer that identifies it in history.
    Commit {
        intent: String,
    },
    Merge,
    /// Best-effort push of `branch` to `origin` after a ticket closes (only when the project has a remote).
    Push {
        branch: String,
    },
    RemoveWorktree {
        delete_branch: bool,
    },
}

impl Op {
    pub fn name(&self) -> &'static str {
        match self {
            Op::CreateWorktree => "create_worktree",
            Op::Commit { .. } => "commit",
            Op::Merge => "merge",
            Op::Push { .. } => "push",
            Op::RemoveWorktree { .. } => "remove_worktree",
        }
    }
}

/// Journals `op` for ticket `t` inside the current transaction; returns the op id (event seq).
pub(crate) fn journal(w: &mut WriteCtx<'_, '_>, t: &Ticket, op: Op) -> Result<i64> {
    let detail = serde_json::to_value(&op).map_err(|e| OrchError::Invalid(e.to_string()))?;
    let rec = w.emit_as(
        "system",
        DomainEvent::VcsOp {
            op: op.name().into(),
            ticket: t.id,
            key: t.key.clone(),
            detail: detail.clone(),
        },
    )?;
    ops::insert(w.tx, rec.seq as i64, t.id, op.name(), &detail)?;
    Ok(rec.seq as i64)
}

fn complete(
    w: &mut WriteCtx<'_, '_>,
    row: &ops::VcsOpRow,
    t: &Ticket,
    state: &str,
    result: serde_json::Value,
) -> Result<bool> {
    if !ops::complete(w.tx, row.seq, state, &result)? {
        return Ok(false);
    }
    w.emit_as(
        "system",
        DomainEvent::VcsOpDone {
            op: row.op.clone(),
            ticket: t.id,
            key: t.key.clone(),
            detail: serde_json::json!({ "op_seq": row.seq, "state": state, "result": result }),
        },
    )?;
    Ok(true)
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CommitInfo {
    pub sha: Option<String>,
    pub head: Option<String>,
    pub has_code: bool,
    /// Where the ticket's work starts now ([`work_base`]).
    #[serde(default)]
    pub base: Option<String>,
}

/// Where the ticket's own work starts: the merge-base of its worktree's HEAD with main. The base recorded when the
/// worktree was made goes stale once the branch is rebased (by the merge queue before a failed check, or by the
/// worker after a conflict); a diff from it would count the files other tickets merged meanwhile as this ticket's.
pub(crate) fn work_base(repo: &Repo, wt: &Path, main: &str, recorded: &str) -> String {
    repo.git()
        .check(wt, ["merge-base", "HEAD", main])
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| recorded.to_owned())
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ReconcileReport {
    pub actions: Vec<String>,
}

const LARGE_CONFLICT_FILES: usize = 5;
const MAX_LOG_CHARS: usize = 4000;

fn tail(s: &str, max: usize) -> String {
    let n = s.chars().count();
    if n <= max {
        s.to_owned()
    } else {
        format!(
            "[... {} characters cut ...]\n{}",
            n - max,
            s.chars().skip(n - max).collect::<String>()
        )
    }
}

/// Copies `include_files` (untracked, e.g. `.env.example`) into a new worktree (decision: `[worktree]` section).
fn prepare_worktree(app: &App, p: &projects::Project, wt: &Path) -> Result<()> {
    let m = app.manifest(p)?;
    for f in &m.worktree.include_files {
        let src = Path::new(&p.path).join(f);
        let dst = wt.join(f);
        if src.is_file() && !dst.exists() {
            if let Some(d) = dst.parent() {
                std::fs::create_dir_all(d)
                    .map_err(|e| OrchError::Invalid(format!("{}: {e}", d.display())))?;
            }
            std::fs::copy(&src, &dst)
                .map_err(|e| OrchError::Invalid(format!("{}: {e}", dst.display())))?;
        }
    }
    Ok(())
}

impl App {
    /// Re-runs the pending journaled operations of `project` (a merge or worktree removal that failed for a
    /// transient reason, e.g. a Windows file lock held by a CLI process that has exited since). Returns how many
    /// completed.
    pub fn replay_pending_ops(&self, project: ProjectId) -> Result<usize> {
        let mut done = 0;
        for row in self.store.read(ops::pending)? {
            if self.store.read(|c| tickets::get(c, row.ticket))?.project != project {
                continue;
            }
            match self.run_op(row.seq) {
                Ok(()) => done += 1,
                Err(e) => {
                    tracing::warn!(op = row.seq, error = %e, "journaled git operation still failing")
                }
            }
        }
        Ok(done)
    }

    /// Fast-forwards a ticket worktree without work of its own to the latest main and records the new base (the
    /// review diff and the scope check compare against it).
    pub fn catch_up_worktree(&self, project: ProjectId, key: &TicketKey) -> Result<bool> {
        let p = self.project(project)?;
        let Some(base) = self.repo(&p).catch_up(key.as_str())? else {
            return Ok(false);
        };
        let t = self.ticket(project, key)?;
        self.store.commit(None, "system", |w| -> Result<()> {
            if let Some(ws) = workspaces::get(w.tx, t.id)? {
                workspaces::upsert(
                    w.tx,
                    &workspaces::TicketWorkspace {
                        base_sha: base.clone(),
                        ..ws
                    },
                )?;
            }
            Ok(())
        })?;
        Ok(true)
    }

    /// Executes one journaled operation if it is still pending. Idempotent.
    pub fn run_op(&self, seq: i64) -> Result<()> {
        let Some(row) = self.store.read(|c| ops::get(c, seq))? else {
            return Ok(());
        };
        if row.state != "pending" {
            return Ok(());
        }
        let op: Op = serde_json::from_value(row.detail.clone())
            .map_err(|e| OrchError::Invalid(e.to_string()))?;
        let t = self.store.read(|c| tickets::get(c, row.ticket))?;
        let p = self.project(t.project)?;
        let repo = self.repo(&p);
        self.inject(&format!("before:{}", op.name()))?;
        match op {
            Op::CreateWorktree => {
                let info = repo.add_worktree(t.key.as_str(), &p.main_branch)?;
                prepare_worktree(self, &p, &info.path)?;
                self.inject("after:create_worktree")?;
                self.store.commit(None, "system", |w| -> Result<()> {
                    let prev = workspaces::get(w.tx, t.id)?;
                    workspaces::upsert(
                        w.tx,
                        &workspaces::TicketWorkspace {
                            ticket: t.id,
                            worktree: info.path.to_string_lossy().into_owned(),
                            branch: info.branch.clone(),
                            base_sha: prev
                                .as_ref()
                                .map(|w| w.base_sha.clone())
                                .unwrap_or(info.base_sha.clone()),
                            head_sha: prev.as_ref().and_then(|w| w.head_sha.clone()),
                            merged_sha: None,
                            removed: false,
                        },
                    )?;
                    complete(
                        w,
                        &row,
                        &t,
                        "done",
                        serde_json::json!({ "path": info.path, "branch": info.branch }),
                    )?;
                    Ok(())
                })?;
            }
            Op::Commit { intent } => {
                // Only reached from reconcile: decide from git history whether the commit happened.
                let sha =
                    repo.commit_with_intent(&agenium_vcs::ticket_branch(t.key.as_str()), &intent)?;
                self.store.commit(None, "system", |w| -> Result<()> {
                    complete(
                        w,
                        &row,
                        &t,
                        if sha.is_some() { "done" } else { "failed" },
                        serde_json::json!({ "sha": sha, "recovered": true }),
                    )?;
                    Ok(())
                })?;
            }
            Op::Merge => self.merge_now(&row, &t, &p, &repo)?,
            Op::Push { branch } => {
                // Outward-facing and best-effort: a failure is recorded, not retried, and never blocks the close.
                let result = repo.push("origin", &format!("{branch}:{branch}"), &[]);
                self.store.commit(None, "system", |w| -> Result<()> {
                    match &result {
                        Ok(()) => {
                            complete(w, &row, &t, "done", serde_json::json!({ "pushed": branch }))?;
                        }
                        Err(e) => {
                            let msg = e.to_string();
                            crate::engine::system_comment(
                                w,
                                &t,
                                format!("Could not push {branch} to the remote (the work is merged and saved locally): {msg}"),
                            )?;
                            complete(w, &row, &t, "done", serde_json::json!({ "push_failed": msg }))?;
                        }
                    }
                    Ok(())
                })?;
            }
            Op::RemoveWorktree { delete_branch } => {
                repo.remove_worktree(t.key.as_str())?;
                agenium_sandbox::remove_run_temp_dir(&repo.worktree_path(t.key.as_str()));
                if delete_branch {
                    repo.delete_branch(&agenium_vcs::ticket_branch(t.key.as_str()))?;
                }
                self.inject("after:remove_worktree")?;
                self.store.commit(None, "system", |w| -> Result<()> {
                    if let Some(ws) = workspaces::get(w.tx, t.id)? {
                        workspaces::upsert(
                            w.tx,
                            &workspaces::TicketWorkspace {
                                removed: true,
                                ..ws
                            },
                        )?;
                    }
                    complete(
                        w,
                        &row,
                        &t,
                        "done",
                        serde_json::json!({ "branch_deleted": delete_branch }),
                    )?;
                    Ok(())
                })?;
            }
        }
        Ok(())
    }

    /// Merge queue step (serialised per project): rebase on the latest main, pre-merge check, fast-forward.
    /// Merge never closes the ticket (decision 36); failures reopen it to the same worker with the reason.
    fn merge_now(
        &self,
        row: &ops::VcsOpRow,
        t: &Ticket,
        p: &projects::Project,
        repo: &Repo,
    ) -> Result<()> {
        let lock = self.merge_lock(p.id);
        let _g = lock.lock().unwrap_or_else(|e| e.into_inner());
        let manifest = self.manifest(p)?;
        let check = self.checks.for_project(p, &manifest);
        let outcome = repo.merge_ticket(t.key.as_str(), check.as_ref())?;
        self.inject("after:merge")?;
        self.store.commit(None, "system", |w| -> Result<()> {
            let mut t = tickets::get(w.tx, t.id)?;
            match &outcome {
                MergeOutcome::Merged { sha, .. } | MergeOutcome::AlreadyMerged { sha } => {
                    if !t.merged {
                        t.merged = true;
                        tickets::update(w.tx, &t)?;
                        if let Some(ws) = workspaces::get(w.tx, t.id)? {
                            workspaces::upsert(w.tx, &workspaces::TicketWorkspace { merged_sha: Some(sha.clone()), ..ws })?;
                        }
                        w.emit_as("system", DomainEvent::TicketMerged { ticket: t.id, key: t.key.clone(), sha: sha.clone() })?;
                        let changed = match &outcome {
                            MergeOutcome::Merged { changed_files, .. } => format!(" ({} files)", changed_files.len()),
                            _ => String::new(),
                        };
                        crate::engine::system_comment(w, &t, format!("Merged into {} at {}{changed}. The ticket stays Resolved until the Lead closes it.", p.main_branch, &sha[..sha.len().min(12)]))?;
                    }
                    complete(w, row, &t, "done", serde_json::to_value(&outcome).unwrap_or_default())?;
                }
                MergeOutcome::Conflict { files } => {
                    let large = files.len() > LARGE_CONFLICT_FILES;
                    let reason = format!("conflict with {} in {}", p.main_branch, files.join(", "));
                    if t.status == TicketStatus::Resolved {
                        transition(w, &mut t, &Actor::System, TicketStatus::Reopened, &Guards { merge_failed: true, ..Default::default() })?;
                        let mut text = format!(
                            "Merge failed: rebase on the latest {} conflicts in: {}.\nRebase your branch on {} in your worktree, resolve the conflicts, then submit_report again.",
                            p.main_branch,
                            files.join(", "),
                            p.main_branch
                        );
                        if large {
                            text.push_str("\nThis is a large conflict: the Lead decides how to proceed (decision 18).");
                            w.emit_as("system", DomainEvent::NeedsUser { ticket: Some(t.id), reason: format!("large merge conflict on {}", t.key) })?;
                        }
                        crate::engine::system_comment(w, &t, text)?;
                    }
                    w.emit_as("system", DomainEvent::MergeFailed { ticket: t.id, key: t.key.clone(), reason, files: files.clone() })?;
                    complete(w, row, &t, "failed", serde_json::to_value(&outcome).unwrap_or_default())?;
                }
                MergeOutcome::CheckFailed { log } => {
                    if t.status == TicketStatus::Resolved {
                        transition(w, &mut t, &Actor::System, TicketStatus::Reopened, &Guards { merge_failed: true, ..Default::default() })?;
                        crate::engine::system_comment(w, &t, format!("Merge failed: the pre-merge check failed.\n```\n{}\n```", tail(log, MAX_LOG_CHARS)))?;
                    }
                    w.emit_as("system", DomainEvent::MergeFailed { ticket: t.id, key: t.key.clone(), reason: "pre-merge check failed".into(), files: vec![] })?;
                    complete(w, row, &t, "failed", serde_json::json!({ "check_failed": true, "log": tail(log, MAX_LOG_CHARS) }))?;
                }
            }
            Ok(())
        })?;
        Ok(())
    }

    /// Auto-commit of the ticket worktree at submit time (journal-first). Secrets block the commit and the error
    /// goes back to the agent.
    pub(crate) fn auto_commit(
        &self,
        p: &projects::Project,
        t: &Ticket,
        agent: AgentId,
        intent: Option<&str>,
    ) -> Result<CommitInfo> {
        let ws = self
            .store
            .read(|c| workspaces::get(c, t.id))?
            .filter(|w| !w.removed)
            .ok_or_else(|| OrchError::Invalid(format!("{} has no worktree", t.key)))?;
        let intent = intent
            .map(str::to_owned)
            .unwrap_or_else(|| ulid::Ulid::new().to_string());
        let seq = self
            .store
            .commit(None, "system", |w| {
                journal(
                    w,
                    t,
                    Op::Commit {
                        intent: intent.clone(),
                    },
                )
            })?
            .value;
        let row = self
            .store
            .read(|c| ops::get(c, seq))?
            .ok_or_else(|| OrchError::Invalid("journal row vanished".into()))?;
        self.inject("before:commit")?;
        let repo = self.repo(p);
        let a = self.agent(agent)?;
        let wt = Path::new(&ws.worktree);
        let message = format!("{}: {}", t.key, t.title);
        let email = format!("{}@agents.agenium.local", a.id);
        let committed = repo.commit_all(
            wt,
            &message,
            (&a.name, &email),
            &[("Ticket", t.key.as_str()), ("Agenium-Intent", &intent)],
        );
        let sha = match committed {
            Ok(sha) => sha,
            Err(e) => {
                let msg = e.to_string();
                self.store.commit(None, "system", |w| {
                    complete(w, &row, t, "failed", serde_json::json!({ "error": msg }))
                })?;
                return Err(e.into());
            }
        };
        self.inject("after:commit")?;
        let head = repo
            .git()
            .check(wt, ["rev-parse", "HEAD"])?
            .trim()
            .to_owned();
        let base = work_base(&repo, wt, &p.main_branch, &ws.base_sha);
        let has_code = !repo.changed_files(wt, &base, &head)?.is_empty();
        self.store.commit(None, "system", |w| {
            complete(w, &row, t, "done", serde_json::json!({ "sha": sha }))
        })?;
        Ok(CommitInfo {
            sha,
            head: Some(head),
            has_code,
            base: Some(base),
        })
    }

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
            report
                .actions
                .push(format!("closed {} interrupted run(s)", dead.len()));
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
                    self.run_op(seq)?;
                    report
                        .actions
                        .push(format!("removed worktree of finished ticket {}", t.key));
                } else if !Path::new(&ws.worktree).is_dir() {
                    let seq = self
                        .store
                        .commit(None, "system", |w| journal(w, &t, Op::CreateWorktree))?
                        .value;
                    self.run_op(seq)?;
                    report
                        .actions
                        .push(format!("restored missing worktree of {}", t.key));
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
                        repo.remove_worktree(&name)?;
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
