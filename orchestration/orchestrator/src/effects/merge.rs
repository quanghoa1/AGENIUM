//! Merging a ticket: the merge itself, and the rebase handed back to the agent on a conflict.

use super::*;

pub(super) const LARGE_CONFLICT_FILES: usize = 5;

pub(super) const MAX_LOG_CHARS: usize = 4000;

pub(super) fn tail(s: &str, max: usize) -> String {
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

impl App {
    /// Merge queue step (serialised per project): rebase on the latest main, pre-merge check, fast-forward.
    /// Merge never closes the ticket (decision 36); failures reopen it to the same worker with the reason.
    pub(super) fn merge_now(
        &self,
        row: &ops::VcsOpRow,
        t: &Ticket,
        p: &projects::Project,
        repo: &Repo,
    ) -> Result<()> {
        let lock = self.merge_lock(p.id);
        let _g = lock.lock().unwrap_or_else(|e| e.into_inner());
        // The ticket may have changed since the merge was decided (a retry runs later): merge only approved work that
        // is not merged yet, never a reopened or cancelled ticket's branch.
        let now = self.store.read(|c| tickets::get(c, t.id))?;
        if !(now.status == TicketStatus::Resolved && now.approved && !now.merged) {
            let (state, result) = if now.merged {
                ("done", serde_json::json!({ "already_merged": true }))
            } else {
                let why = format!("{} is {} and not approved for a merge", now.key, now.status);
                ("failed", serde_json::json!({ "obsolete": why }))
            };
            self.store.commit(None, "system", |w| {
                complete(w, row, &now, state, result.clone())
            })?;
            return Ok(());
        }
        let manifest = self.manifest(p)?;
        let check = self
            .checks
            .for_project(p, &manifest, self.config().sandbox.network);
        let prepared = repo.prepare_merge(t.key.as_str(), check.as_ref())?;
        let done = self.store.commit(None, "system", |w| -> Result<Option<MergeOutcome>> {
            let mut t = tickets::get(w.tx, t.id)?;
            let outcome = match prepared {
                // The rebase and the check took a while (a test suite): the user may have reopened or cancelled the
                // ticket meanwhile, and its work must not reach main then (audit H1: it did, and a rework approved
                // later was never merged and its branch deleted). Read again and fast-forward inside this
                // transaction, which a reopen or a cancel cannot interleave with.
                Prepared::Ready { head, main_before } => {
                    if !(t.status == TicketStatus::Resolved && t.approved && !t.merged) {
                        let why = format!("{} became {} during the merge's check", t.key, t.status);
                        complete(w, row, &t, "failed", serde_json::json!({ "obsolete": why }))?;
                        return Ok(None);
                    }
                    let o = repo.fast_forward(&head, &main_before)?;
                    self.inject("after:merge")?;
                    o
                }
                Prepared::Done(o) => o,
            };
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
                            "Merge failed: rebase on the latest {} conflicts in: {}.\nCall update_from_main to bring {} into your worktree (Agenium does the git work), resolve what it reports, then submit_report again.",
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
            Ok(Some(outcome))
        })?;
        if let Some(MergeOutcome::Merged { changed_files, .. }) = &done.value {
            if let Err(e) = self.nudge_rebase(p, repo, t, changed_files) {
                tracing::warn!(error = %e, "early rebase nudge failed");
            }
        }
        Ok(())
    }

    /// Decision 18: after a merge, a ticket in progress whose branch changes one of the merged files is told at once
    /// to rebase, instead of meeting the conflict at its own merge after all its work.
    pub(super) fn nudge_rebase(
        &self,
        p: &projects::Project,
        repo: &Repo,
        merged: &Ticket,
        changed: &[String],
    ) -> Result<()> {
        if changed.is_empty() {
            return Ok(());
        }
        let open = self
            .store
            .read(|c| tickets::list(c, p.id, &Default::default()))?;
        for other in open.iter().filter(|o| {
            o.id != merged.id
                && matches!(
                    o.status,
                    TicketStatus::InProgress | TicketStatus::Pending | TicketStatus::Reopened
                )
        }) {
            let Some(ws) = self
                .store
                .read(|c| workspaces::get(c, other.id))?
                .filter(|w| !w.removed)
            else {
                continue;
            };
            let wt = std::path::Path::new(&ws.worktree);
            if !wt.join(".git").exists() {
                continue;
            }
            let base = work_base(repo, wt, &p.main_branch, &ws.base_sha);
            let mut mine = repo.changed_files(wt, &base, "HEAD").unwrap_or_default();
            mine.extend(
                repo.status(wt)
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|l| l.get(3..))
                    .map(|s| s.trim().to_owned()),
            );
            let both: Vec<&String> = changed
                .iter()
                .filter(|f| mine.iter().any(|m| m == *f))
                .collect();
            if both.is_empty() {
                continue;
            }
            let text = format!(
                "{} was merged into {} and changed files you are changing too: {}. Call update_from_main now to bring {} into your worktree (Agenium does the git work) and resolve any conflict while the change is small.",
                merged.key,
                p.main_branch,
                both.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", "),
                p.main_branch
            );
            self.store.commit(None, "system", |w| -> Result<()> {
                let o = tickets::get(w.tx, other.id)?;
                crate::engine::system_comment(w, &o, text.clone())?;
                Ok(())
            })?;
        }
        Ok(())
    }
}
