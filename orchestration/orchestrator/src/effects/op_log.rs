//! The journal of git operations: each written before it runs and completed after, retried with backoff.

use super::*;

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

/// Ends the pending merges of `t` without running them (it was reopened or cancelled: its branch is not the approved
/// work any more). Recorded as failed with the reason.
pub(crate) fn drop_pending_merges(w: &mut WriteCtx<'_, '_>, t: &Ticket, why: &str) -> Result<()> {
    for row in ops::for_ticket(w.tx, t.id)?
        .into_iter()
        .filter(|o| o.state == "pending" && o.op == Op::Merge.name())
    {
        complete(w, &row, t, "failed", serde_json::json!({ "obsolete": why }))?;
    }
    Ok(())
}

pub(super) fn complete(
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

/// Failed replays of a journaled operation in a row before the user is told (once) and the replays back off.
pub const OP_FAILURES_BEFORE_USER: u32 = 5;

/// Replays skipped between two tries of an operation that keeps failing, at most (10 minutes at a 3-second tick).
pub(super) const OP_BACKOFF_MAX_SKIPS: u32 = 200;

/// Retry state of a pending operation that failed (in memory: a restart retries every pending operation once).
#[derive(Debug, Default)]
pub(crate) struct OpRetry {
    pub(super) failures: u32,
    /// Replays to skip before the next try.
    pub(super) skip: u32,
}

impl App {
    /// Re-runs the pending journaled operations of `project` (a merge or worktree removal that failed for a
    /// transient reason, e.g. a Windows file lock held by a CLI process that has exited since). Returns how many
    /// completed.
    pub fn replay_pending_ops(&self, project: ProjectId) -> Result<usize> {
        let mut done = 0;
        let pending = self.store.read(ops::pending)?;
        // Operations ended elsewhere (a merge dropped by a reopen) leave the retry state.
        self.op_retries
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .retain(|seq, _| pending.iter().any(|r| r.seq == *seq));
        for row in pending {
            let t = self.store.read(|c| tickets::get(c, row.ticket))?;
            if t.project != project {
                continue;
            }
            // An operation that keeps failing is retried less and less often (a merge re-runs the pre-merge check).
            let waiting = {
                let mut m = self.op_retries.lock().unwrap_or_else(|p| p.into_inner());
                m.get_mut(&row.seq).is_some_and(|r| {
                    let wait = r.skip > 0;
                    r.skip = r.skip.saturating_sub(1);
                    wait
                })
            };
            if waiting {
                continue;
            }
            match self.run_op(row.seq) {
                Ok(()) => {
                    self.op_retries
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .remove(&row.seq);
                    done += 1;
                }
                Err(e) => {
                    tracing::warn!(op = row.seq, error = %e, "journaled git operation still failing");
                    self.op_failed(&row, &t, &e)?;
                }
            }
        }
        Ok(done)
    }

    /// Counts a failed replay; at [`OP_FAILURES_BEFORE_USER`] in a row the ticket gets a System comment and the user
    /// is asked to look (once), and the replays back off.
    pub(super) fn op_failed(&self, row: &ops::VcsOpRow, t: &Ticket, e: &OrchError) -> Result<()> {
        let failures = {
            let mut m = self.op_retries.lock().unwrap_or_else(|p| p.into_inner());
            let r = m.entry(row.seq).or_default();
            r.failures += 1;
            if r.failures >= OP_FAILURES_BEFORE_USER {
                r.skip = 1u32
                    .checked_shl(r.failures - OP_FAILURES_BEFORE_USER + 1)
                    .unwrap_or(u32::MAX)
                    .min(OP_BACKOFF_MAX_SKIPS);
            }
            r.failures
        };
        if failures != OP_FAILURES_BEFORE_USER {
            return Ok(());
        }
        let what = if row.op == Op::Merge.name() {
            format!(
                "The merge of {} keeps failing ({failures} tries): {e}. The ticket stays Resolved and approved and its dependents wait; Agenium retries less often now. Look at the project folder (main changed by hand, a locked file).",
                t.key
            )
        } else {
            format!(
                "The git operation {} of {} keeps failing ({failures} tries): {e}. Agenium retries less often now.",
                row.op, t.key
            )
        };
        self.store.commit(None, "system", |w| -> Result<()> {
            let t = tickets::get(w.tx, t.id)?;
            crate::engine::system_comment(w, &t, &what)?;
            w.emit_as(
                "system",
                DomainEvent::NeedsUser {
                    ticket: Some(t.id),
                    reason: format!("{}: {} keeps failing", t.key, row.op),
                },
            )?;
            Ok(())
        })?;
        Ok(())
    }
}
