//! update_from_main: a worker's worktree brought up to the latest main by Agenium, outside the sandbox (git writes are
//! blocked inside it, ADR-53), when a ticket it depends on merged or it was told to rebase (ADR-158).

use super::*;

impl App {
    pub(super) fn update_from_main(
        &self,
        ctx: &CallCtx,
        project: &projects::Project,
        u: agenium_core::protocol::UpdateFromMain,
    ) -> Result<Outcome> {
        let t = self
            .store
            .read(|c| tickets::get_by_key(c, ctx.project, &u.ticket_id))?;
        let agent = require_assignee(&t, &ctx.actor)?;
        if t.status != TicketStatus::InProgress {
            return Err(invalid(format!(
                "{} is {}; update_from_main is only valid while InProgress",
                t.key, t.status
            )));
        }
        // The worker's changes become a commit of its branch first (journal-first, secrets refused), so the rebase
        // carries them and nothing is lost on a conflict.
        let commit = self.auto_commit(project, &t, agent, None)?;
        let ws = self
            .store
            .read(|c| workspaces::get(c, t.id))?
            .filter(|w| !w.removed)
            .ok_or_else(|| invalid(format!("{} has no worktree", t.key)))?;
        let repo = self.repo(project);
        let wt = std::path::Path::new(&ws.worktree);
        let main = project.main_branch.clone();
        match repo.rebase_with(wt, &main, u.keep_mine)? {
            agenium_vcs::RebaseOutcome::Rebased { head } => {
                let base = crate::effects::work_base(&repo, wt, &main, &ws.base_sha);
                self.store.commit(None, "system", |w| -> Result<()> {
                    if let Some(ws) = workspaces::get(w.tx, t.id)? {
                        workspaces::upsert(
                            w.tx,
                            &workspaces::TicketWorkspace {
                                base_sha: base.clone(),
                                head_sha: Some(head.clone()),
                                ..ws
                            },
                        )?;
                    }
                    Ok(())
                })?;
                Ok(Outcome::ok(serde_json::json!({
                    "ticket_id": t.key, "updated": true, "on": main, "head": head,
                    "committed": commit.sha,
                    "note": format!("Your branch now sits on the latest {main}; your changes are on top of it."),
                })))
            }
            agenium_vcs::RebaseOutcome::Conflict { files } => Ok(Outcome::ok(serde_json::json!({
                "ticket_id": t.key, "updated": false, "conflicts": files, "committed": commit.sha,
                "note": format!(
                    "Your branch and {main} change the same lines in these files; nothing changed in your worktree. \
                     Read {main}'s version (git show {main}:<file>), bring its change into your file, then call \
                     update_from_main again with keep_mine=true: the hunks that still conflict keep your version."
                ),
            }))),
        }
    }
}
