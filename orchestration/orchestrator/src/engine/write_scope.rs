//! The write scope of a ticket: what its assignee may change, and the changes on its branch that fall outside it
//! (checked when the worker submits its report).

use super::*;

impl App {
    /// Write scope of the assignee on `t` (architecture page "Ép phạm vi"): the ticket's `scope_paths` and the
    /// role's allowed paths must both match; approved path permissions of the ticket and always-allowed project
    /// paths widen it. Returns (lists that must all match, granted patterns).
    pub fn write_scope(&self, t: &Ticket, role: &str) -> Result<(Vec<Vec<String>>, Vec<String>)> {
        let p = self.project(t.project)?;
        let mut granted: Vec<String> = allowlist(self, &p)
            .into_iter()
            .filter_map(|e| e.strip_prefix("path:").map(str::to_owned))
            .collect();
        for g in self
            .store
            .read(|c| runs::active_grants(c, t.project, Some(t.id)))?
        {
            if let Ok(r) = serde_json::from_value::<PermissionRequest>(g.request) {
                if r.kind == PermissionKind::Path {
                    granted.extend(r.paths);
                }
            }
        }
        Ok((
            vec![t.scope_paths.clone(), self.role_allowed_paths(role)],
            granted,
        ))
    }

    /// Files changed on the ticket's branch (committed since its base, and uncommitted) that fall outside the write
    /// scope of the assignee. Covers shell commands and CLI backends, whose writes the file tools never see.
    pub(super) fn out_of_scope_changes(
        &self,
        project: &projects::Project,
        t: &Ticket,
        role: &str,
    ) -> Result<Vec<String>> {
        let (all_of, granted) = self.write_scope(t, role)?;
        let scope =
            agenium_tools::paths::WriteScope::new(all_of, granted).map_err(OrchError::Invalid)?;
        if scope.is_open() {
            return Ok(vec![]);
        }
        let Some(ws) = self
            .store
            .read(|c| workspaces::get(c, t.id))?
            .filter(|w| !w.removed)
        else {
            return Ok(vec![]);
        };
        let repo = self.repo(project);
        let wt = std::path::Path::new(&ws.worktree);
        let base = crate::effects::work_base(&repo, wt, &project.main_branch, &ws.base_sha);
        let mut files = repo.changed_files(wt, &base, "HEAD")?;
        for line in repo.status(wt)? {
            // Porcelain v1: "XY path" or "XY from -> to". Both sides of a rename count: moving a file out of the
            // scope deletes it there.
            let path = line.get(3..).unwrap_or_default();
            files.extend(path.split(" -> ").map(|p| p.trim_matches('"').to_owned()));
        }
        files.sort();
        files.dedup();
        Ok(files
            .into_iter()
            .filter(|f| !f.starts_with("artifacts/tool-output/") && !scope.allows(f))
            .collect())
    }
}
