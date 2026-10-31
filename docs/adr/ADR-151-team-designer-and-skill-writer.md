# ADR-151: A team designer and a skill writer, on the model the user picks

- **Status:** Accepted
- **Date:** 2026-11-01 (Asia/Saigon)
- **Asked by the owner:** "chúng ta có thể áp dụng logic của nó [revfactory/harness] để làm chức năng đề xuất team không,
  ví dụ người dùng chat với nó là tôi muốn làm 1 team chuyên coding website, thì nó sẽ đề xuất nên tạo team gồm bao
  nhiêu thành viên, mỗi thành viên nên có skill gì"; "trong khi đề xuất thì có đề xuất skill mới, thì người dùng lúc đó
  sẽ dùng nút tạo skill tự động để đưa thông tin … có thể đưa tài liệu … để agent đảm nhiệm tạo ra skill theo quy định
  của agenium"; "vẫn giữ tạo skill thủ công"; "người dùng vẫn chọn AI provider và model/effort"; "vẫn dùng cli, api đều
  được, không bắt buộc api".

## Context

A team came from a built-in template, a template written by hand (ADR-71), or pool agents composed by hand (ADR-61):
nothing turned "I want a team that builds web shops" into roles, counts and skills. A skill was written by hand in the
editor (ADR-124) or imported. Two references were read, their rules taken in our own words, no code or package used
([[learn-from-references-not-reuse]]):

- **revfactory/harness** (a Claude Code plugin that designs agent teams): understand the work before splitting it;
  split roles by expertise, parallelism, how much one role must hold, and reuse; keep teams small (2-3 workers for a
  focused job, 3-5 for a broad one); reuse existing agents and skills before inventing; a verifying role when the
  output can be checked by running it; pick each agent's model by its work, never one expensive model for all.
- **Anthropic's skill-creator**: interview, draft with progressive disclosure, say why a step matters instead of
  all-caps rules, write for every request of the kind; its test-and-grade loop (with and without the skill) is left for
  later.

## Decision

1. **One answer beside the runs** (`runtime::side_chat`): the user's model, backend and effort (the agent forms' model
   picker). An API provider gets one request; a vendor CLI gets one chat turn with no tool, in an empty scratch folder,
   read-only, its commands refused (`DenyAll`), checked like an agent's CLI (`cli_ready_named`: installed, logged in,
   a sandbox or the user's unsandboxed choice). Nothing is stored; the caller keeps the conversation and sends the
   helper's whole last answer back, so "make it smaller" edits that draft.
2. **The team designer** (`team_design`, Templates page "Propose a team", and from the Teams page): told the library's
   roles and skills, the report templates and the user's models (from `model-cache.json`), with the harness rules
   above. It asks at most three questions, once, then proposes. Agenium checks the answer before the user sees it: a
   skill the library lacks becomes a proposal (never a role's skill), a model the user does not have is dropped, ids
   are made valid and never reuse a template id, counts are bounded, write scopes stay inside the worktree, and the
   template must pass the validation a hand-written one does (one Lead, a worker, at most one planner); an answer that
   is not JSON or not a valid team is sent back once with why. The draft shows each role with its skills, scope, model
   and why, and opens in the template editor; nothing is created until the user saves it there.
3. **The skill writer** (`skill_write`, Skill & MCP page "Write a skill with an agent", beside writing one by hand; and
   from each skill the designer proposed): the user's request, their text documents (up to 120 000 characters in all,
   marked as material, never instructions), the designer's proposal if any. It writes to Agenium's contract: name,
   a third-person description that says when (ADR-141), the five sections (ADR-88), long detail in `references/` named
   by a step. Agenium sends it back once when a section is missing, a file is outside `references/`/`scripts/` or never
   named by the body, or the text holds something like a secret. The draft opens in the skill editor with its files;
   the user evaluates and saves it like a skill they wrote (the same validation, conflict check and Jev read). Saved
   from a team draft, the role gets it in that draft.
4. **A template role's library skills reach its agents**: `RoleSpec.skills` names a skill in the role's folder or, now,
   a library skill switched on for each of the role's agents when a team is made from the template (the template
   editor has the field). Before, a library skill there was only reported missing.

## Not done

Since done by [ADR-152](ADR-152-skill-library-documents-helper-sessions.md): documents of any common format (read as
Markdown), the helpers' calls in the usage page, a team made on each role's model, the chats kept as sessions.

- No test-and-grade loop for a written skill (skill-creator's with/without runs): it spends a model run per test.
- The helpers' calls are not counted in the usage views (no agent owns them); they are small (one answer, a repair at
  most).
- Only text documents (Markdown, text, code) are read; a PDF is not.
- The model a designed role names is the template's `default_model` (shown, kept), but a team is still made with the
  models chosen on the team form (ADR-71).

## Verified

- Unit: `team_design::check` (a draft keeps what the user has and proposes the rest; a team must be valid; questions
  alone are an answer), `skill_write` (a skill in the contract is ready for the editor; a broken one is sent back with
  why; the documents open the first message as material).
- Scripted runtime (`tests/runtime/design.rs`): a designed team, after one repair, becomes a template and a team whose
  agents have the library skills it named; a skill written from a document with an injected line saves into the
  library as drafted, with no shape advice left.
- Real models, real app (2026-11-01), both helpers on each: API DeepSeek V4.1 Flash, OpenCode Go's free `beam-free`
  and `iceman-contributor-free`; CLI Claude Haiku, Codex gpt-6-luna, agy Gemini 3.8 Flash, the opencode CLI on Go's
  two free models: 15/15 drafts (designer 16-99 s, writer 11-165 s; CLIs faster). DeepSeek asked three questions for a
  web shop and, told to go ahead, proposed five roles with its assumptions; a change request ("two front-ends") came
  back as the edited draft. A document line telling the writer to answer one word was ignored and reported. The UI
  flow at 2560x1440 (vi): the designer from `#/templates/design`, its draft, "Write with an agent" on a proposed skill
  (prefilled), the draft in the skill editor saved, the role showing it "in the library", the draft opened in the
  template editor with the written skill on its role; no console error.

## Consequences

- `orchestration/orchestrator/src/runtime/side_chat.rs`; `runtime/models.rs` (`cli_ready_named`).
- `orchestration/orchestrator/src/team_design/{mod.rs, check.rs, prompt.rs}`; `skill_write/{mod.rs, prompt.rs}`;
  `team_templates.rs` (library skills switched on at team creation).
- Desktop: `api/design.rs` (`design_team`, `write_skill`).
- UI: `components/HelperChat.tsx`, `views/TeamDesigner.tsx`, `views/capabilities/SkillWriter.tsx`; `Templates.tsx`
  (button, `#/templates/design`, role skills field), `TeamsView.tsx` (button), `skills.tsx` (card), `SkillEditor.tsx`
  (opens drafted files); strings `helper.*`, `design.*`, `skillWriter.*`, `templates.skills*`.
