//! Verification records (ADR-147): what checked a ticket's work, read from its runs' logs — never from the report's
//! own words. A criterion whose evidence cites a check command (`cargo test`, `npm run build` …) is backed only when
//! that command ran in the ticket's runs after their last change and exited 0 — the last change is read from the edit
//! tools and from the worktree's tree logged with each check (ADR-148). Otherwise the reviewer approves it only
//! with their own word on how they verified it, as for a Partial criterion. A criterion that cites no check is a
//! judgement criterion: the review decides it, as before. The records go into the review task and the handover report.

use agenium_core::forms::{AcStatus, WorkReport};
use agenium_core::TicketId;
use agenium_store::core::runs::{self, LogEntry};
use agenium_store::StoreError;
use rusqlite::Connection;
use serde::Serialize;

use crate::judge::done_check::{is_check, EDIT_TOOLS};

/// What backs one criterion's evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Backing {
    /// The evidence cites no check command: the reviewer's judgement.
    Judgement,
    /// Every check it cites passed after the last change.
    Passed { command: String },
    /// A cited check failed the last time it ran after the last change.
    Failed { command: String, code: Option<i64> },
    /// A cited check ran only before the last change: it says nothing about the work as it is.
    Stale { command: String },
    /// A cited check never ran in the ticket's runs.
    NotRun { command: String },
}

impl Backing {
    /// The evidence cites a check the record does not back.
    pub fn unverified(&self) -> bool {
        matches!(
            self,
            Backing::Failed { .. } | Backing::Stale { .. } | Backing::NotRun { .. }
        )
    }

    /// In words, for the reviewer and the report.
    pub fn text(&self) -> String {
        match self {
            Backing::Judgement => "no check cited: the review decides".into(),
            Backing::Passed { command } => format!("`{command}` passed after the last change"),
            Backing::Failed { command, code } => format!(
                "`{command}` failed after the last change ({})",
                code.map_or("no exit code".to_owned(), |c| format!("exit code {c}"))
            ),
            Backing::Stale { command } => {
                format!("`{command}` ran only before the last change")
            }
            Backing::NotRun { command } => format!("`{command}` never ran in this ticket's runs"),
        }
    }
}

/// One criterion of the report and what backs it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CriterionRecord {
    pub ac_id: String,
    pub status: AcStatus,
    pub backing: Backing,
}

/// The ticket's verification record.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Verification {
    /// Commands run after the last change, with their exit code (`None`: timed out or no code).
    pub checks_after: Vec<(String, Option<i64>)>,
    /// Changes to files made in the ticket's runs.
    pub changes: usize,
    pub criteria: Vec<CriterionRecord>,
}

impl Verification {
    /// The record as the reviewer reads it.
    pub fn text(&self) -> String {
        let mut s = String::from("\n## Checks on record (from the runs' logs, not the report)\n");
        if self.checks_after.is_empty() {
            s.push_str("- No command ran after the last change.\n");
        }
        for (cmd, code) in &self.checks_after {
            let code = code.map_or("no exit code".to_owned(), |c| format!("exit code {c}"));
            s.push_str(&format!(
                "- `{}` -> {code}\n",
                cmd.chars().take(200).collect::<String>()
            ));
        }
        for c in &self.criteria {
            s.push_str(&format!(
                "- {} ({:?}): {}\n",
                c.ac_id,
                c.status,
                c.backing.text()
            ));
        }
        s
    }
}

/// One shell command of the logs: the command, its exit code, and whether it ran after the last change.
struct Ran {
    command: String,
    code: Option<i64>,
    after: bool,
}

/// The worktree tree an entry records: a check's snapshot, or what a run left (`worktree_state`).
fn tree_of(e: &LogEntry) -> Option<&str> {
    match e.kind.as_str() {
        "tool" | "worktree_state" => e.data["tree"].as_str(),
        _ => None,
    }
}

/// The shell commands of the logs. A command runs after the last change when no edit tool ran after it and, when it
/// was a check (ADR-148), its snapshot is the latest tree on record: a file changed by a later shell command, a
/// formatter or a generator makes it stale too, though no edit tool ran.
fn commands(logs: &[LogEntry]) -> (Vec<Ran>, usize) {
    let latest = logs.iter().rev().find_map(tree_of);
    let mut ran: Vec<(String, Option<i64>, usize, bool)> = Vec::new();
    let (mut last_change, mut changes) = (None, 0);
    for (i, e) in logs.iter().enumerate().filter(|(_, e)| e.kind == "tool") {
        let tool = e.data["tool"].as_str().unwrap_or_default();
        if EDIT_TOOLS.contains(&tool) && e.data["ok"].as_bool().unwrap_or(false) {
            last_change = Some(i);
            changes += 1;
        } else if tool == "shell" {
            let code = e.data["result"]
                .as_str()
                .and_then(|r| r.strip_prefix("exit code "))
                .and_then(|r| r.split_whitespace().next())
                .and_then(|c| c.parse().ok());
            let cmd = e.data["args"]["command"].as_str().unwrap_or_default();
            let current = tree_of(e).zip(latest).is_none_or(|(t, l)| t == l);
            ran.push((squash(cmd), code, i, current));
        }
    }
    let ran = ran
        .into_iter()
        .map(|(command, code, i, current)| Ran {
            command,
            code,
            after: current && last_change.is_none_or(|l| i > l),
        })
        .collect();
    (ran, changes)
}

fn squash(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Programs a check command starts with (its first word, without a path or an extension).
const RUNNERS: &[&str] = &[
    "cargo",
    "npm",
    "npx",
    "pnpm",
    "yarn",
    "bun",
    "node",
    "deno",
    "python",
    "python3",
    "py",
    "pytest",
    "uv",
    "poetry",
    "tox",
    "nox",
    "go",
    "make",
    "cmake",
    "ctest",
    "mvn",
    "gradle",
    "gradlew",
    "dotnet",
    "tsc",
    "eslint",
    "ruff",
    "flake8",
    "mypy",
    "pyright",
    "jest",
    "vitest",
    "mocha",
    "rspec",
    "bundle",
    "rake",
    "php",
    "phpunit",
    "composer",
    "swift",
    "sh",
    "bash",
    "pwsh",
    "powershell",
    "just",
    "bazel",
    "zig",
    "mix",
    "stack",
    "cabal",
];

/// The check commands an evidence text cites: its code spans that read as a check command — a program that runs one
/// (`cargo test -p x`, `pytest`, `python check.py`) — not a file, a test name or an output (`test_parser.py`,
/// `test_add passed`).
pub fn cited_checks(evidence: &str) -> Vec<String> {
    evidence
        .split('`')
        .skip(1)
        .step_by(2)
        .map(squash)
        .filter(|s| {
            let first = s.split(' ').next().unwrap_or_default().to_lowercase();
            let program = first
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or_default()
                .split('.')
                .next()
                .unwrap_or_default()
                .to_owned();
            is_check(s) && RUNNERS.contains(&program.as_str())
        })
        .collect()
}

fn backing(evidence: &str, ran: &[Ran]) -> Backing {
    let cited = cited_checks(evidence);
    if cited.is_empty() {
        return Backing::Judgement;
    }
    for c in &cited {
        let last = ran
            .iter()
            .rev()
            .find(|r| r.command.contains(c.as_str()) || c.contains(r.command.as_str()));
        let command = c.clone();
        match last {
            None => return Backing::NotRun { command },
            Some(r) if !r.after => return Backing::Stale { command },
            Some(r) if r.code != Some(0) => {
                return Backing::Failed {
                    command,
                    code: r.code,
                }
            }
            Some(_) => {}
        }
    }
    Backing::Passed {
        command: cited.join(", "),
    }
}

/// The record of `report` against the logs of the ticket's runs, in order.
pub fn record(report: &WorkReport, logs: &[LogEntry]) -> Verification {
    let (ran, changes) = commands(logs);
    Verification {
        checks_after: ran
            .iter()
            .filter(|r| r.after)
            .map(|r| (r.command.clone(), r.code))
            .collect(),
        changes,
        criteria: report
            .acceptance
            .iter()
            .map(|a| CriterionRecord {
                ac_id: a.ac_id.clone(),
                status: a.status,
                backing: backing(&a.evidence, &ran),
            })
            .collect(),
    }
}

/// The logs of every run on `ticket`, in order (the worker's runs and the reviewer's: a check the reviewer ran counts).
pub fn ticket_logs(c: &Connection, ticket: TicketId) -> Result<Vec<LogEntry>, StoreError> {
    let mut out = Vec::new();
    for r in runs::for_ticket(c, ticket)? {
        out.extend(runs::log_entries(c, r.id)?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(tool: &str, args: serde_json::Value, ok: bool, result: &str) -> LogEntry {
        LogEntry {
            n: 0,
            kind: "tool".into(),
            data: serde_json::json!({ "tool": tool, "args": args, "ok": ok, "result": result }),
            at: String::new(),
        }
    }

    fn shell(cmd: &str, code: i64) -> LogEntry {
        tool(
            "shell",
            serde_json::json!({ "command": cmd }),
            true,
            &format!("exit code {code}\nout"),
        )
    }

    fn edit() -> LogEntry {
        tool(
            "edit_file",
            serde_json::json!({ "path": "src/a.rs" }),
            true,
            "edited",
        )
    }

    #[test]
    fn evidence_cites_commands_not_file_names() {
        assert_eq!(
            cited_checks("`cargo test -p parser` passes; see `tests/test_parser.py` and `pytest`"),
            vec!["cargo test -p parser".to_owned(), "pytest".to_owned()]
        );
        assert!(cited_checks("the page shows the title").is_empty());
        assert!(cited_checks("`README.md` has the section").is_empty());
        assert!(cited_checks("`test_add passed` in the output").is_empty());
        assert_eq!(
            cited_checks("`./gradlew test` and `python check.py`"),
            vec!["./gradlew test".to_owned(), "python check.py".to_owned()]
        );
    }

    #[test]
    fn a_cited_check_is_backed_only_when_it_passed_after_the_last_change() {
        let passed = [edit(), shell("cargo  test -p parser", 0)];
        let (ran, changes) = commands(&passed);
        assert_eq!(changes, 1);
        assert!(matches!(
            backing("`cargo test -p parser`: 12 passed", &ran),
            Backing::Passed { .. }
        ));
        assert_eq!(backing("the doc reads well", &ran), Backing::Judgement);

        let stale = [shell("cargo test -p parser", 0), edit()];
        let (ran, _) = commands(&stale);
        assert!(matches!(
            backing("`cargo test -p parser`", &ran),
            Backing::Stale { .. }
        ));

        let failed = [
            edit(),
            shell("cargo test -p parser", 0),
            edit(),
            shell("cargo test -p parser", 101),
        ];
        let (ran, _) = commands(&failed);
        assert_eq!(
            backing("`cargo test -p parser`", &ran),
            Backing::Failed {
                command: "cargo test -p parser".into(),
                code: Some(101)
            }
        );
        assert!(matches!(
            backing("`npm run build`", &ran),
            Backing::NotRun { .. }
        ));
        // Two cited checks: both must pass.
        let both = [edit(), shell("cargo build", 0), shell("cargo test", 1)];
        let (ran, _) = commands(&both);
        assert!(backing("`cargo build` and `cargo test`", &ran).unverified());
        // A failed edit changes nothing.
        let (ran, changes) = commands(&[
            shell("cargo test", 0),
            tool("edit_file", serde_json::json!({}), false, "no match"),
        ]);
        assert_eq!(changes, 0);
        assert!(matches!(
            backing("`cargo test`", &ran),
            Backing::Passed { .. }
        ));
    }

    fn snapshot(cmd: &str, tree: &str) -> LogEntry {
        let mut e = shell(cmd, 0);
        e.data["tree"] = tree.into();
        e
    }

    fn left(tree: &str) -> LogEntry {
        LogEntry {
            n: 0,
            kind: "worktree_state".into(),
            data: serde_json::json!({ "head": "h", "tree": tree }),
            at: String::new(),
        }
    }

    #[test]
    fn a_check_is_tied_to_the_files_it_ran_on() {
        // A shell command changed a file after the check (no edit tool ran): the check is stale.
        let logs = [
            edit(),
            snapshot("cargo test", "t1"),
            shell("sed -i s/a/b/ src/a.rs", 0),
            left("t2"),
        ];
        let (ran, _) = commands(&logs);
        assert!(matches!(
            backing("`cargo test`", &ran),
            Backing::Stale { .. }
        ));
        // Nothing changed after it: the run left the tree the check ran on.
        let (ran, _) = commands(&[edit(), snapshot("cargo test", "t1"), left("t1")]);
        assert!(matches!(
            backing("`cargo test`", &ran),
            Backing::Passed { .. }
        ));
        // A later check on the final files backs it again; the reviewer's run left the same tree.
        let logs = [
            snapshot("cargo test", "t1"),
            left("t2"),
            snapshot("cargo test", "t2"),
        ];
        let (ran, _) = commands(&logs);
        assert!(matches!(
            backing("`cargo test`", &ran),
            Backing::Passed { .. }
        ));
    }
}
