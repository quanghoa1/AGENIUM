# ADR-64: Commands run in the sandbox without asking; role shape and library; skeptical review

Status: Accepted (2026-10-25, owner: "no more shell command approval once there is a sandbox; decide and carry on
with your proposals": the Lead-role plan items, the review loop and a ROLE.md shape learned from a public catalogue
of agent personas, rules only, our own text)

## Decision

1. **Approval policy `sandboxed` (the new default).** A command an agent runs through Agenium's tools runs without
   asking when a sandbox backend works on this machine (MXC on Windows, bubblewrap on Linux: writes only in the
   ticket's worktree, reads limited to toolchains, no network). Without a backend it goes to the user, as before
   (M1-7, fail-closed). Commands a vendor CLI runs on the host (claude, agy, checked through their hooks) are not
   contained and still go to the project allowlist or the user. `on-request` (ask for every command not on the
   allowlist) and `never` (YOLO, refused without a sandbox) stay. Existing roots keep the value written in their
   config; the desktop Settings page switches it ("Run in the sandbox / Ask me every time / Sandbox only, never ask")
   and shows whether this machine has a working sandbox.
2. **Command approvals belong to a project.** The approver gets the project; "Needs you" lists a waiting command
   under its own project, not under every project.
3. **No plan item for the Lead's own role in team mode.** The Lead takes no tickets, so `create_plan`, `revise_plan`
   and the planner's draft refuse an item whose role is the Lead's (unless a worker has that role too), naming the
   workers' roles. The Lead's planning task lists those roles, and the Lead rules say so. The research template gets
   a `synthesist` worker for the combined answer the Lead used to plan for itself.
4. **Workers get the requirements they serve.** A work run and a review run carry "What this ticket serves": the spec
   goals, the requirements the plan item traces to with their acceptance criteria, and the constraints. Workers
   could not read the spec (it is not in their worktree) and asked the user for it.
5. **Skeptical review.** The Dev/QA loop already exists (a report needs evidence per criterion, Reopen needs feedback
   per unmet criterion, more than three reopens escalate to the user). The Lead's rules and review task now say to
   treat a report as a claim and check each criterion's evidence (the test, the changed lines, the file) before
   counting it as met.
6. **ROLE.md shape.** Every role text has the sections Mission, Rules, Deliverables, Workflow, Done when, kept short
   (procedures go in skills). A check gives advice with each role (missing or empty section, more than 6000
   characters, nearly the same words as another role); it never refuses a save. The built-in team templates are
   rewritten in that shape (version 1.1.0; role folders already written stay the user's). A role library adds
   specialist presets usable in any team: code-reviewer, bugfixer, codebase-explorer, software-architect, sre,
   appsec-auditor, api-tester, performance-engineer, accessibility-auditor, technical-writer.
7. **Not taken from the catalogue:** personality and emoji, long prompts, free-text orchestration (ours is tools and
   tickets), roles without path or tool limits. No text or file of it is copied.

## Consequences

- Config: `[sandbox] approval = "sandboxed"` by default. CLI `doctor` explains the policy with the backend.
- Desktop: Settings "Agent shell commands"; Needs you groups commands by project; the role picker has a "Specialist
  roles" group, shows the advice, and a new role starts from the section skeleton.
- Tests: `runtime.rs::sandboxed_commands_run_without_asking`, the denied-command test sets `on-request`;
  `team.rs` checks the Lead-role refusal and the requirements text; `role_library` unit tests check every built-in
  role against the shape and against each other.
