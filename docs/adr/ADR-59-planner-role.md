# ADR-59: The planner — a team member that only drafts plan v1

Status: Accepted (2026-10-24, owner's decision, M4)

## Context

The owner asked whether a team should have an agent dedicated to planning: it would work with the Lead at the start
to turn the user's request into a detailed plan and have no task afterwards; changes to the plan during the work stay
with the Lead. Planning benefits from a strong model and a clean context, but a planner that owns the plan and then
leaves splits responsibility and loses the reasons behind the plan when the Lead must answer questions, review
against acceptance criteria and revise it.

The owner also set: **no subagents in team mode; only Solo agents may call agents** (ADR-56 addendum). The planner is
a declared team member started by the scheduler, not a subagent an agent spawns.

## Decision

1. **Role.** A team template may declare one `planner = true` role (kind worker, count 1); built-in coding, pentest
   and research templates have one. The flag lives in `<root>/roles/<role>/role.toml`. The planner never takes
   tickets (`assign_ticket` refuses it; `get_team` marks it) and can get its own model
   (`agenium team create --planner-model`).
2. **When.** The scheduler runs the planner when the latest spec is approved, the project has no plan, and the
   planner has not drafted for that spec yet. The run has read-only tools on the project, the spec, and the roles of
   the team's workers; its only protocol tool is the new `submit_plan_draft` (milestones, items, rationale), which is
   validated like `create_plan` against the spec and ends the run. Two failed runs for the same spec go to the user.
3. **Lead owns the plan.** The draft is stored as a `PlanDrafted` event (no new table). The Lead's inbox gets a
   Planning item with the draft and its rationale; the Lead reviews and adjusts it and calls `create_plan` itself;
   the user approves plan v1 as before. Later revisions are the Lead's (`revise_plan`); the planner's rationale stays
   in the Lead's project state as context.
4. **Without a planner**, an approved spec without a plan becomes a Planning item for the Lead (create the plan), so a
   team no longer needs another chat turn after the user approves the spec.
5. **Tool catalogue.** `submit_plan_draft` is an Agenium addition to the design's 24 tools (Worker group; offered only
   in planner runs and accepted only from the planner).

## Consequences

- Tests: `team.rs::planner_drafts_and_the_lead_creates_the_plan`,
  `team.rs::approved_spec_without_planner_asks_the_lead_for_a_plan`,
  `runtime.rs::planner_drafts_then_the_lead_creates_the_plan`, `protocol.rs::catalogue_matches_design_index`.
- Only the initial plan is drafted by the planner: a new spec version on a project that already has a plan is the
  Lead's to revise.
