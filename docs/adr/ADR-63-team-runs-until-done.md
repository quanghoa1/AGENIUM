# ADR-63: From the agreed requirements to the end, the team runs on its own

Status: Accepted (2026-10-25, owner: "once the Lead and the user agree on the requirements, let the team run:
plan, assign, everything; no autopilot switch and no 'run one or two rounds' — what we need is the project done;
stop only on a big problem or when the user stops it"; "each agent is in one team and one project")

## Decision

1. **Run state of a project** (`App::run_state`): `Agreeing` (no locked spec yet: the user and the Lead talk),
   `Running` (from the locked spec on), `Paused` (the user stopped it; also a cancelled project) and `Done` (every
   plan item done, no open ticket, no spec or plan waiting). The project status follows: the team loop sets
   `Completed` when the work is done and `Active` again when new work appears (a new spec, new tickets).
2. **The agreement is the spec.** A team's spec still waits for the user (decision 3h); a Solo agent's spec locks at
   once (decision 6). The plan built from a locked spec is the Lead's: plan v1 and every revision are approved by
   the Lead (each item traces to the agreed spec), the user is informed. New requirements need a new spec, which
   the user approves. This replaces "plan v1 and scope-changing revisions wait for the user" (decision 3h as implemented in M3).
3. **The team loop** replaces the desktop autopilot switch and the "run a round" button: every few seconds every
   project gets a round (`Runtime::work`), one at a time per project, projects side by side; a round only runs when
   the project is `Running`. Work that needs the user (a question escalated to the user, a permission outside the
   allowlist, a command approval, a new spec) waits in "Needs you" while the rest goes on; the project stops by
   itself when it is done or when nothing can go on without the user.
4. **Stop and resume per project.** Each project's runs get their own stop flag (`Cancel::child` of the kill switch):
   `pause_project` stops its runs at the next step and starts none; `resume_project` lets it go on. The kill switch
   stops every run and pauses every running project. Chat with the Lead is never blocked by a pause.
5. **One team, one project per agent** (decisions 3 and 25, ADR-61): unchanged and enforced — an agent is in at most
   one team, a team works on at most one project, a project has at most one team.

## Consequences

- Desktop: the header shows the selected project's state (agreeing on requirements / team working / stopped /
  project done) with Stop or Resume; the settings page has no autopilot; the board has no "run a round". CLI:
  `project pause|resume`, `project list` prints the run state, `project create --repo`; `run --rounds` stays for
  scripted use.
- Tests: `a_project_runs_from_the_agreed_spec_and_the_user_stops_it`, the end-to-end Solo test checks `Done` and
  `Completed`; team tests now expect the Lead's plan to be approved.
