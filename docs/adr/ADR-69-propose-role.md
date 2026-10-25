# ADR-69: The Lead proposes roles, the user decides

Status: Accepted (2026-10-25; decision 3l of the architecture: "the Lead proposes a role through `propose_role`; an
agent is created only when the user approves"). Closes the open item of ADR-61.

## Decision

1. **Proposal.** In team mode the Lead calls `propose_role` with a role id (one of `get_team`'s `role_library`, the
   specialist presets of ADR-64, or a new id), a title, what the role does and why the plan needs it (at least 20
   characters), optional tags, library skills and a model hint. Refused: Solo agents, workers, a role a member
   already has (the work goes to that member), the same role while a proposal for it waits. A proposal is a row of
   `role_proposal` (core.db migration 4) and a `RoleProposed` event; nothing else changes.
2. **The user decides** in Needs you (new kind `RoleProposal`: Approve / Decline), or with the CLI
   (`agenium role proposals`, `agenium approve role <id>`, `agenium decline role <id> --note`).
   - Approve: a new role id gets a ROLE.md in the section shape of ADR-64 from the proposal's text; an agent named
     after the role (next free number when taken) is created with the Lead's model and backend (the user changes
     it on the agent's page), the proposal's library skills, and joins the Lead's team.
   - Decline: the reason is kept.
3. **The Lead sees the outcome** in its project state ("Role proposal RP-... (api-tester): approved, api-tester
   joined the team" or "declined by the user: ..."). The Lead rules, the Plan inbox text and the Lead-role refusal of
   `create_plan` point at `propose_role` when no worker role fits.

## Evidence

- `team.rs::the_lead_proposes_a_role_and_the_user_decides` (refusals, nothing created before the decision, library
  and new role, ROLE.md passes the role check, decline keeps nothing).
- Real Lead (DeepSeek v4.1 Flash) on the test root: asked in Vietnamese, it read `get_team`, proposed the library
  role `api-tester` with a reason; Approve in Needs you created `api-tester` in the Research team.
