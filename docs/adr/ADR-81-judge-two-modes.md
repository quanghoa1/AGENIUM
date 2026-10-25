# ADR-81: The judge runs in two modes; a failure never stops Agenium

Status: Accepted (2026-10-25; owner: "two modes, with Jev and without; if Jev errors or runs out of quota the user
re-enters the key and re-enables, and meanwhile Agenium works normally").

## Decision

The TypeSafe Jev judge (ADR-75) is optional and **fail-open**:
- **Without Jev** (no `[judge]` config, or after a disabling failure) every judge call returns nothing and the work
  proceeds on the agent's own judgment. Agenium is fully functional with no judge.
- **With Jev**, a call that fails on the **key or the quota** (401/403, or a rate-limit) turns the judge off
  (`<root>/judge-health.json`) and raises a Needs-you asking the user to re-enter the key and re-enable it; it is not
  retried on every run. A one-off run/network error does not disable it.
- **Re-enable** (`App::enable_judge`, desktop Settings "Re-enable and test") clears the disabled state and makes a
  trivial probe request; it stays off if the probe still fails. Re-entering the key then re-enabling resumes Jev.
- The desktop Settings shows the judge state (not configured / enabled / turned off, with the reason).

## Consequences

- `JudgeHealth` in `<root>/judge-health.json`; `App::judge()` gated on it; `disable_judge` on key/quota failures in
  `review_evidence`; `App::enable_judge`. Desktop IPC `judge_status`, `enable_judge`.
- Agent export/import (memory, skills and overlays, skill/MCP enablement; no secrets) already existed (M15-5); it is
  now exposed on the desktop too (`export_agent`/`import_agent` IPC, an Export button on the agent page).
- Test: `team.rs::the_judge_is_optional_and_a_failure_turns_it_off`.

## Future Jev uses (optional, same fail-open rule)

Quick typed judgments where they save the agent's tokens or speed a decision: a skill/role pick (Choice) before a
run, a guardrail Noul on output, lesson-signal/cue classification. Each stays opt-in and off when the judge is off.
