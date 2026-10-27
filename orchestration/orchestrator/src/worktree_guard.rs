//! Decision 19: while a project runs, its ticket worktrees and `agenium/*` branches belong to the agents. Agenium
//! records what each run left in the worktree (`worktree_state` in the run log); before the next run on the ticket a
//! change made outside Agenium (another editor, a commit by hand) is undone, saved as a patch under
//! `<root>/recovered/<project>/`, and reported to the user, who changes the work through the ticket or the Lead.

use std::path::Path;

use agenium_core::*;
use agenium_store::core::{runs, workspaces};
use agenium_vcs::WorktreeState;

use crate::app::App;
use crate::Result;

const STATE_LOG: &str = "worktree_state";

impl App {
    /// After a run on a ticket: what it left in the worktree (nothing when the worktree is gone).
    pub fn record_worktree_state(&self, project: ProjectId, run: RunId) -> Result<()> {
        let Some(ticket) = self.store.read(|c| runs::get(c, run))?.ticket else {
            return Ok(());
        };
        let Some(ws) = self
            .store
            .read(|c| workspaces::get(c, ticket))?
            .filter(|w| !w.removed)
        else {
            return Ok(());
        };
        // A worktree being removed (its ticket merged; the folder may still be there for a moment, or until a
        // journaled removal retries) has nothing of the run's to keep.
        let ended = self
            .store
            .read(|c| agenium_store::core::tickets::get(c, ticket))?
            .status
            .is_terminal();
        if ended || !Path::new(&ws.worktree).join(".git").exists() {
            return Ok(());
        }
        let repo = self.repo(&self.project(project)?);
        match repo.worktree_state(Path::new(&ws.worktree)) {
            Ok(s) => self.run_log(run, STATE_LOG, serde_json::to_value(s).unwrap_or_default()),
            Err(e) => {
                tracing::warn!(%run, error = %e, "worktree state not recorded");
                Ok(())
            }
        }
    }

    /// The state the last run of `ticket` left, if one was recorded.
    fn left_by_last_run(&self, ticket: TicketId) -> Result<Option<WorktreeState>> {
        let rs = self.store.read(|c| runs::for_ticket(c, ticket))?;
        for r in rs.iter().rev().filter(|r| r.ended_at.is_some()) {
            let entries = self.store.read(|c| runs::log_entries(c, r.id))?;
            if let Some(e) = entries.iter().rev().find(|e| e.kind == STATE_LOG) {
                return Ok(serde_json::from_value(e.data.clone()).ok());
            }
        }
        Ok(None)
    }

    /// Before a run on `key`: undoes changes made to its worktree outside Agenium since the last run, keeps them as
    /// a patch and tells the user. Returns the patch file when something was undone.
    pub fn guard_worktree(
        &self,
        project: ProjectId,
        key: &TicketKey,
    ) -> Result<Option<std::path::PathBuf>> {
        let t = self.ticket(project, key)?;
        let Some(ws) = self
            .store
            .read(|c| workspaces::get(c, t.id))?
            .filter(|w| !w.removed)
        else {
            return Ok(None);
        };
        let Some(left) = self.left_by_last_run(t.id)? else {
            return Ok(None);
        };
        let p = self.project(project)?;
        let repo = self.repo(&p);
        let wt = Path::new(&ws.worktree);
        let now = repo.worktree_state(wt)?;
        if now == left {
            return Ok(None);
        }
        let head_tree = |h: &str| -> Result<String> {
            Ok(repo
                .git()
                .check(wt, ["rev-parse", &format!("{h}^{{tree}}")])?
                .trim()
                .to_owned())
        };
        // Where to go back to: what the last run left, or, when Agenium itself moved the branch since (catch-up,
        // rebase), the clean new HEAD.
        let target = if now.head == left.head || !repo.foreign_commits(wt, &left.head)?.is_empty() {
            left
        } else if left.tree == head_tree(&left.head)? && now.tree != head_tree(&now.head)? {
            WorktreeState {
                tree: head_tree(&now.head)?,
                head: now.head.clone(),
            }
        } else {
            return Ok(None);
        };
        let patch = repo.restore_worktree(wt, &target)?;
        let dir = self.layout().root.join("recovered").join(&p.name);
        std::fs::create_dir_all(&dir).map_err(|e| crate::OrchError::Invalid(e.to_string()))?;
        let file = dir.join(format!(
            "{}-{}.patch",
            t.key,
            chrono::Utc::now().format("%Y%m%dT%H%M%SZ")
        ));
        std::fs::write(&file, patch).map_err(|e| crate::OrchError::Invalid(e.to_string()))?;
        let shown = file.display().to_string();
        self.store.commit(None, "system", |w| -> Result<()> {
            let tk = agenium_store::core::tickets::get(w.tx, t.id)?;
            crate::engine::system_comment(
                w,
                &tk,
                format!(
                    "Changes made to this ticket's worktree outside Agenium were undone (the agents own it while the project runs); they are saved in {shown}. To change the work, comment on the ticket or tell the Lead; edit main directly only while the project is stopped."
                ),
            )?;
            w.emit(DomainEvent::NeedsUser {
                ticket: Some(t.id),
                reason: format!("{}: outside changes to its worktree were undone and saved in {shown}", t.key),
            })?;
            Ok(())
        })?;
        Ok(Some(file))
    }
}
