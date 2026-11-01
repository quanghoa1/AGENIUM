# ADR-88: Skill shape, authoring checks, and a built-in starter skill library

Status: Accepted (2026-10-25; amended 2026-11-01: no skill ships, see below; owner: write agency-agents entries as skills, and "turn that thinking into a system that
helps when the user creates a skill or an agent distils / learns / upgrades its skill"). Reference input: Jason Haddix's
"Building Hackbots" (DEF CON 34) — methodologies written as one-job markdown skill files, each ending in a proof the
agent must produce. We take the thinking, not the files or the security domain, per [[learn-from-references-not-reuse]].

## Decision

Give **skills** the same kind of contract roles got in [ADR-64], and a starter library, so a skill the user writes and
a skill an agent upgrades follow one shape.

1. **Skill shape** (`skill_shape.rs`): a recommended SKILL.md body with five sections — **Job** (the one thing it does),
   **When to use** (the cue), **Steps** (the method), **Tools** (which tools to run and how to read their output), and
   **Proof** (the evidence the agent must produce before claiming done — the "hard stop"). `skill_skeleton()` returns the
   empty form.
2. **Advisory checks** (`lint_skill`): missing/empty sections, body over 8,000 chars (move detail into `references/` read
   on demand), and near-duplicate of another skill. Like the role checks, they are **advice, never a refusal**.
3. **Both surfaces use the same checks.** When the user imports a library skill, the result carries `shape_notes` the
   desktop shows. When an agent self-upgrades a skill (`propose_skill_update`, ADR-73), the same `lint_skill` runs and
   the notes are recorded in the overlay changelog — so agent-distilled skills are held to the shape too. This ties into
   the existing learning loop: validated lessons → overlay (ADR-73), base-update rebase (ADR-86), cross-agent pool
   (ADR-87).
4. **Built-in skill library**: twelve engineering skills in our own words, each in the shape above with a Proof section —
   `code-review`, `root-cause-debugging`, `perf-audit`, `a11y-audit`, `api-contract-testing`, `doc-writing`,
   `writing-tests`, `refactoring`, `commit-and-pr`, `migration-safety`, `secure-coding`, `release-readiness` (the generic,
   non-cyber methodologies the agency-agents catalogue expresses as personas; the specialist **roles** already landed as
   ADR-64, the **methodologies** land as skills). They ship as **real files** under
   `orchestration/orchestrator/templates/skills/<name>/` — SKILL.md plus `references/*.md` (longer checklists read on
   demand, tier 3) and `scripts/*` (two helpers: `commit-and-pr/scripts/check_commit.py` lints a commit message;
   `release-readiness/scripts/preflight.py` reports a clean-tree GO/NO-GO) — embedded at build time with `include_str!`.
   `App::seed_skill_library` writes a skill and all its files into `<root>/skills` on open, **idempotently**: it writes
   only a skill whose folder is absent, so a user's edits and removals are never overwritten. The body references each
   aux file, so skill validation confirms they exist.
   *Amended 2026-11-01 (owner: "mặc định skill và mcp không có sẵn trong agenium, khi upgrade version sẽ không chạm
   skill mcp của người dùng thêm vào, cũng như của agent đã học"):* **Agenium ships no skills and no MCP server.**
   Nothing is embedded or seeded: a new root's library and MCP list are empty, and hold only what the user adds
   (New skill, Import, Write with an agent, a proposal accepted from an agent's lesson) and what agents keep in their
   own copies. An install or upgrade replaces the program only; the root (skills, MCP servers in `config.toml`,
   agents' copies, memory, the database) is the user's and is never touched. The starter skills written for this ADR
   stay on the developer's machine under `orchestration/orchestrator/templates/skills/`, out of git (`.gitignore`) and
   out of every build; the shape, conflict and quality tests read them as a corpus there and skip in a clean clone,
   and the gates' tests use skills written for the tests (`tests/common/skills.rs`). Roots that were seeded before keep
   their copies: nothing removes them.

## Consequences

- `orchestration/orchestrator/src/skill_shape.rs`: `SKILL_SECTIONS`, `skill_skeleton`, `lint_skill`, the `Builtin`
  library, `App::{skill_skeleton, lint_skill, library_skill_body, seed_skill_library}`; seeding called in `App::open`.
- `skill_overlay.rs`: `apply_skill_update` records `shape_notes` on each agent self-upgrade.
- Desktop: `import_skill` returns `shape_notes` (shown after import); new IPC `skill_skeleton`, `lint_skill`; vi/en
  string `library.shapeNotes`.
- Tests: `skill_shape::tests` (skeleton flags sections, a built-in passes its own checks, near-duplicate); `team.rs`
  `the_starter_skill_library_is_seeded_and_valid`; `registry.rs` library-count assertions updated for the seeded skills.
- Where Haddix's other ideas already live in Agenium: generator/critic ≈ Lead reviews worker + Jev `review_assist`;
  hard stop / proof ≈ `submit_report` evidence per AC + skeptical review (ADR-64); findings-as-candidates ≈ review +
  reopen. No new mechanism was needed for those.
