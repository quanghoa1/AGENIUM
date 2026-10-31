# Plan: skill library, documents, helper sessions (ADR-152)

Agreed with the owner on 2026-11-01. Each step lands with its tests; the end-to-end check runs on real models, API and
CLI, free to cheap. Status: `[ ]` to do, `[x]` done.

## Rules agreed

- Skills written by the skill-writer agent go to the **system library**. Worker, Lead and Solo agents see only the
  skills enabled for them, change only **their own clone**, and **propose** new skills from experience (a lesson that
  held in two contexts); a proposal waits in **Needs you**.
- The skill-writer agent sees the **whole library**. The local embedding model (bge-small, already loaded for memory,
  no API tokens) finds duplicates and near-duplicates **before** it writes, so it merges or writes a new skill knowingly.
- Needs you, for an agent's proposal: **approve by hand** (skill editor), **discuss with the skill-writer agent** (one
  session, seeded with the proposal and its evidence), or **decline**.
- Every helper chat is a **session** kept in the root: model, turns, attachments, draft, result, cost; reopened later.
- Every model choice is **remembered** in the root: agent form, team form, skill writer, template designer (separately).
- Documents reach models as **Markdown**: Agenium's own converters, or markitdown when installed (better quality),
  run in the sandbox; Agenium installs, updates (self-tested) and removes its own markitdown on request.
- Skill quality: 12 criteria checked **locally** (no model call); real effectiveness measured from ticket outcomes.
- Dropped: the paid "test a skill" loop (with/without runs).

## Steps

0. [x] Stabilize what is written (documents crate, sweep, helper usage, proposals, designer attachments): build, tests.
1. [x] **Remembered model choices** — `preferences` in the root (core db), keys `model.agent`, `model.team`,
   `model.skill_writer`, `model.designer`; desktop get/set; the forms and helper chats start from them; a remembered
   model that is gone shows empty with a note.
2. [x] **Helper sessions** (backend; the chat's history list lands with the UI pass) — table `helper_session` (kind, title, model, turns, attachments, draft, source, status,
   result, timestamps); `helper_call.session`; list/open/continue; a session per "discuss a proposal".
3. [x] **Skill similarity** — cached subject vectors of library skills (by content hash); `similar_skills(subject, k)`
   with words and meaning scores and a level (duplicate ≥ ADR-92 bars, near ≥ 0.70 meaning / 0.30 words).
4. [x] **Agent proposals** — a proposal like a library skill the agent does not have is accepted and labelled with the
   similar skills (not refused); like a skill it has, it improves its clone as before. Similar skills stored with it.
5. [x] **Skill writer: merge** (backend; the badges, merge button and diff land with the UI pass) — the 3 closest skills given in full before writing; `extends` drafts edit that skill;
   a draft shows its similar skills, "merge into X" asks again with X; the editor opens X with a before/after diff.
6. [x] **Needs you: skill proposals** — approve by hand / discuss with the skill writer (session seeded) / decline;
   the saved skill is switched on for the proposing agent.
7. [x] **Attachments** — one component: pick files, drop files (top-most chat wins over page drop targets), paste a
   path; read as Markdown by the backend; in the Lead/Solo chat, the template designer and the skill writer; shown as
   chips in the chat history.
8. [x] **`read_file` reads documents** — PDF/Office/EPUB/notebooks to Markdown for every agent (built in; markitdown
   when installed and a sandbox works), paged like text, marked as converted.
9. [x] **markitdown in Settings** — status (Agenium's own / on PATH / none, version, formats); install into
   `<root>/tools/markitdown` (uv, else Python venv, else a checksummed standalone uv), pinned version, self-test on
   generated samples; weekly update check (notify only); update side by side with self-test and rollback; remove.
10. [x] **Team from a template uses each role's model** — option on the team form (on when the template names models);
    a role's model must be usable, else the form's model; remembered (step 1).
11. [x] **Usage page counts helper calls** — rows for the designer and the skill writer by day and model, with cost.
12. [x] **Skill quality** — scorecard of the 12 criteria in the editor's Evaluate, the writer's draft and a proposal;
    effectiveness from tickets (loads of the skill, reopens, against the agent's other tickets), on the library card,
    flagged when clearly worse with enough tickets.
13. [x] **Docs** — ADR-152 (amends ADR-91, ADR-151), CHANGELOG, VERIFICATION, PROGRESS, this plan ticked.
14. [x] **Verify** — unit and integration tests per step; local CI in full (fmt, clippy, tests, schemas, vendor,
    ratchet, dlls, deny, golden, UI smoke); the Linux build of the sandbox change (VM); end to end in the real app:
    - designer and skill writer with attachments (PDF, DOCX, XLSX) and merges, on API DeepSeek V4.1 Flash, Go's free
      `beam-free` and `iceman-contributor-free`, and the CLIs Claude Haiku, Codex gpt-6-luna, agy Gemini Flash,
      opencode on Go's free models;
    - a skill proposal from a scripted agent, discussed in a session with real models and saved;
    - `read_file` on a PDF and a DOCX inside real agent runs (API and CLI);
    - markitdown installed into a throwaway root, self-tested, used, updated, removed;
    - a team from a designed template with per-role models; remembered choices after a restart;
    - UI at 2560x1440 in en and vi, no console error; everything cleaned up after.
