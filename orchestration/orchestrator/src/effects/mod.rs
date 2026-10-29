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

mod merge;
mod op_log;
mod reconcile;

pub use op_log::*;
pub use reconcile::*;

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
    /// The worktree of a ticket that ended (Cancelled) while a run still worked in it: removed once no run is live on
    /// the ticket and no removal is journaled yet (`finish_run`).
    pub(crate) fn remove_ended_worktree(&self, ticket: TicketId) -> Result<()> {
        let seq = self
            .store
            .commit(None, "system", |w| -> Result<Option<i64>> {
                let t = tickets::get(w.tx, ticket)?;
                let live_ws = workspaces::get(w.tx, t.id)?.is_some_and(|ws| !ws.removed);
                if !t.status.is_terminal() || !live_ws {
                    return Ok(None);
                }
                let live_run = runs::unfinished(w.tx)?
                    .iter()
                    .any(|r| r.ticket == Some(t.id));
                let journaled = ops::for_ticket(w.tx, t.id)?
                    .iter()
                    .any(|o| o.state == "pending" && o.op == "remove_worktree");
                if live_run || journaled {
                    return Ok(None);
                }
                journal(
                    w,
                    &t,
                    Op::RemoveWorktree {
                        delete_branch: t.merged,
                    },
                )
                .map(Some)
            })?
            .value;
        if let Some(seq) = seq {
            if let Err(e) = self.run_op(seq) {
                tracing::warn!(op = seq, error = %e, "worktree removal failed; it is retried later");
            }
        }
        Ok(())
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
                self.forget_cli_sessions(t.key.as_str());
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
}
