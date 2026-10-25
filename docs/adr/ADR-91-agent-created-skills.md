# ADR-91: Agents may create a new skill — only through strict gates, contained, and flagged when it stops holding

Status: Accepted (2026-10-25; owner: "do it, research it carefully — it affects agents; agents must not learn and
create skills carelessly, or the knowledge will drift later"; and "don't let the system depend on Jev — both with and
without"). Extends skill overlays (ADR-73), the skill shape (ADR-88) and the Jev quality read (ADR-90).

## Research

- **hermes-agent (NousResearch)**: the agent creates a skill after complex work and skills improve during use; memory
  is nudged periodically. Its README does not document how a skill is distilled or how a bad one is caught.
- **harness2 (Google Research)**: Propose → Probe → Compose — candidate harness edits (skills among them) are tried on
  the task and only what holds is kept; an experience reflector carries *evidence-supported lessons* forward; weights
  stay fixed.

What we take: creating a skill is worth it (hermes), but only from evidence that already held, and only kept while it
keeps holding (harness2). What we add, because neither documents it: hard limits against sprawl and an explicit way for
a skill to lose its standing when experience turns against it.

## Finding: the self-upgrade tool was never offered

`propose_skill_update` (ADR-73) was excluded from every run kind since M0 (work, review, chat, inbox; the plan run
allows two tools only) — the exclusion predates overlays and was never lifted, so the overlay loop only ran in tests
that call `apply_skill_update` directly. Fixed here with one offering policy for both skill-writing tools.

## Decision

1. **`propose_new_skill`** (Shared protocol tool): name, one-line description, SKILL.md body, reason, and the lesson it
   distils. It is created in the agent's **own** skills folder only after **every** gate passes; a refusal names the gate:
   - **Evidence** — the lesson is live, a Lesson, **trusted** (not from a web page, file or tool output) and held in
     **two distinct contexts**; **one lesson yields at most one skill**.
   - **No sprawl** — the name is not a skill the agent already has (library, role, own); the body is **not a
     near-duplicate** of any of them (then it must improve that skill with `propose_skill_update`); at most
     **12 created skills** per agent.
   - **Well-formed** — the full shape is filled (Job, When to use, Steps, Tools, Proof) and the body is short: the
     shape check (ADR-88) must come back **clean** — advice for a user, a hard gate for an agent. A SKILL.md only:
     an agent never writes scripts or reference files into a skill (a body that refers to one fails validation).
   - **Safe** — no injected instructions, no secrets; validated as a skill in a staging copy before it is swapped in.
2. **Contained** — it lives only in the agent's folder: it never changes the library, a role, or another agent. Sharing
   still goes only through the existing gated paths (shared pool ADR-87, promote).
3. **When the tools are offered** (both `propose_skill_update` and `propose_new_skill`): **work runs only, and only when
   the agent holds at least one qualifying lesson** (live, trusted, two contexts). Then the first message names those
   lessons and says to write a skill down **only after the ticket's work, never instead of it**, to improve an existing
   skill first, and to do nothing if no lesson is a reusable procedure. Review, chat, inbox and plan runs never get them.
4. **Anti-drift** — a created skill keeps the id of its lesson. After every learning pass (and when the user retires a
   fact) Agenium checks each created skill: if its lesson was **deleted, retired or replaced**, or its **confidence fell
   below 0.3** (each reopen it misled costs 0.15), the skill is **flagged** and the user gets one Needs-you to review or
   delete it.
5. **Accountable** — every creation and removal is in the audit trail; the Memory page lists the agent's created skills
   with the lesson behind each, the flag, and **Delete**.
6. **With and without Jev** — nothing above needs the judge. With `[judge].skill_quality` on, the agent also gets the
   judge's calibrated read of its new or updated skill (ADR-90) in the tool result; without it, or on any failure, the
   deterministic gates decide alone.

## Consequences

- `foundation/core`: `forms::ProposeNewSkill`; `ProtocolTool::ProposeNewSkill`, `Intent::ProposeNewSkill`
  (Shared group: 5 tools).
- `orchestration/orchestrator/src/skill_create.rs`: `create_agent_skill`, `audit_created_skills`,
  `remove_created_skill`, `created_skills` (record kept in the agent's memory meta `created_skills`).
- `runtime/mod.rs`: the qualifying-lesson check, the offering policy and the hint; `runtime/bridge.rs` routes the tool
  (plus the fail-open quality read); `runtime/learn.rs` runs the audit after each learning pass.
- Desktop: `memory` returns `created`; `memory_action` `remove_skill`, and `retire` re-runs the audit; Memory page
  section with lesson, flag and Delete; vi/en strings.
- Tests: `team.rs::an_agent_creates_a_skill_only_through_every_gate` (each gate, containment, one-lesson-one-skill,
  retire → flag once, delete), `team.rs::a_created_skill_is_flagged_when_its_lesson_loses_confidence`;
  `runtime.rs` asserts the tools are withheld without a qualifying lesson and offered (with the hint) with one.
- Known limit: a new skill that *contradicts* an existing one with different words passes the near-duplicate check; it
  stays contained to its agent, and the flag catches it once reopens show its lesson is wrong.
