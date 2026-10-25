# ADR-78: Markdown project report

Status: Accepted (2026-10-25, M5).

## Decision

`agenium report --project <id> [--out FILE]`, the desktop Overview "Report" button (copies Markdown), and
`App::project_report_md` produce a readable Markdown summary of a project from the stored state (no model call): the
run state, the agreed spec version and plan version, the goals and requirements with acceptance criteria, the plan
items as a checklist with their tickets, and a ticket table (key, title, status, assignee, reopens) with a
closed/total line.

## Consequences

- `agenium_orchestrator::report::project_report_md`; CLI `report`; desktop IPC `project_report`.
- Test: `team.rs::project_report_is_markdown_from_state`.
