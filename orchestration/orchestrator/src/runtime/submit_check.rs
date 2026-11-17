//! The project's check at submit (ADR-162): a worker's `submit_report` on a ticket with code changes runs the
//! project's `pre_merge` check in the ticket worktree first. A failure refuses the report and hands the output back
//! in the same run, so the worker fixes it while it still has the context, instead of the merge queue reopening the
//! ticket after a review run. After [`MAX_REFUSALS`] refusals in a run the report goes through and the ticket says
//! the check still fails, for the Lead to see. The merge queue still runs the check as before (ADR-79).
//!
//! It never blocks a submit it cannot judge: no check, no code change, or a check that cannot run unattended (no
//! sandbox backend, a check that asks the user first) lets the report through, and the ticket says why it did not run.

use agenium_core::forms::WorkReport;
use agenium_store::core::{runs, workspaces};

use super::bridge::{RunKind, RunProtocol};

/// Refusals in one run before the report goes through marked as failing the check.
const MAX_REFUSALS: usize = 2;
/// Characters of the check's output (its tail, where failures are) returned to the worker and logged.
const OUTPUT_CHARS: usize = 4000;
/// Run log entry kind and `use` of this check.
const KIND: &str = "check";
const USE: &str = "submit_check";

impl RunProtocol {
    /// `Err` is the refusal handed back to the worker as the tool result.
    pub(super) async fn submit_check(&self, r: &WorkReport) -> Result<(), String> {
        if self.kind != RunKind::Work {
            return Ok(());
        }
        let app = &self.app;
        let (Ok(t), Ok(p)) = (
            app.ticket(self.project, &r.ticket_id),
            app.project(self.project),
        ) else {
            return Ok(());
        };
        let Ok(manifest) = app.manifest(&p) else {
            return Ok(());
        };
        let cmd = manifest.checks.pre_merge.trim().to_owned();
        let ws = app
            .store()
            .read(|c| workspaces::get(c, t.id))
            .ok()
            .flatten();
        let Some(ws) = ws.filter(|w| !w.removed) else {
            return Ok(());
        };
        let root = std::path::PathBuf::from(&ws.worktree);
        if cmd.is_empty() || !has_changes(&app.repo(&p), &root, &ws.base_sha) {
            return Ok(());
        }
        let check = app
            .checks
            .for_project(&p, &manifest, app.config().sandbox.network);
        if let Err(why) = check.ready() {
            self.log(serde_json::json!({ "use": USE, "ran": false, "why": why }));
            self.note(
                &t,
                format!(
                    "The pre-merge check did not run at submit: {why}. The merge queue runs it."
                ),
            );
            return Ok(());
        }
        let refused = self.refusals();
        let out = tokio::task::spawn_blocking(move || check.run(&root)).await;
        let Ok(out) = out else {
            return Ok(());
        };
        let log = tail(&out.log);
        let refuse = !out.ok && refused < MAX_REFUSALS;
        self.log(serde_json::json!({ "use": USE, "ran": true, "ok": out.ok, "refused": refuse, "log": log }));
        if out.ok {
            return Ok(());
        }
        if refuse {
            return Err(format!(
                "The project's pre-merge check `{cmd}` fails in your worktree, so the report was not accepted \
                 (refusal {} of {MAX_REFUSALS}; after that the report goes through marked as failing the check). Fix \
                 the cause, not the check, then call submit_report again.\n```\n{log}\n```",
                refused + 1
            ));
        }
        self.note(
            &t,
            format!(
                "Submitted with a failing pre-merge check: `{cmd}` still failed after {MAX_REFUSALS} refusals in the \
                 run.\n```\n{log}\n```"
            ),
        );
        Ok(())
    }

    /// Refusals this check already made in this run, from the run's log.
    fn refusals(&self) -> usize {
        self.app
            .store()
            .read(|c| runs::log_entries(c, self.run))
            .unwrap_or_default()
            .iter()
            .filter(|e| e.kind == KIND && e.data["use"] == USE && e.data["refused"] == true)
            .count()
    }

    fn log(&self, data: serde_json::Value) {
        let _ = self.app.run_log(self.run, KIND, data);
    }

    /// A system comment on the ticket: the Lead's review and the ticket page show it beside the report.
    fn note(&self, t: &agenium_core::Ticket, text: String) {
        let r = self.app.store().commit(None, "system", |w| {
            crate::engine::system_comment(w, t, text)
        });
        if let Err(e) = r {
            tracing::warn!(ticket = %t.key, error = %e, "submit check note not written");
        }
    }
}

/// Whether the worktree holds work since its base: uncommitted changes or commits. Unknown counts as yes.
fn has_changes(repo: &agenium_vcs::Repo, root: &std::path::Path, base: &str) -> bool {
    repo.status(root).map_or(true, |s| !s.is_empty())
        || repo
            .changed_files(root, base, "HEAD")
            .map_or(true, |f| !f.is_empty())
}

fn tail(s: &str) -> String {
    let n = s.chars().count();
    if n <= OUTPUT_CHARS {
        return s.to_owned();
    }
    let kept: String = s.chars().skip(n - OUTPUT_CHARS).collect();
    format!("[... {} characters cut ...]\n{kept}", n - OUTPUT_CHARS)
}
