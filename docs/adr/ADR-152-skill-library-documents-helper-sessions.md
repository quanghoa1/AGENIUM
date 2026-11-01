# ADR-152: One skill library, documents as Markdown, helper sessions, remembered models

- **Status:** Accepted
- **Date:** 2026-11-01 (Asia/Saigon)
- **Amends:** ADR-91 (an agent's new skill becomes a proposal), ADR-151 (the helpers keep sessions, read documents,
  see the library).
- **Asked by the owner:** "agent tạo skill là tính vào skill hệ thống, còn agent solo và trong team thì dùng lại và chỉ
  update/sữa chữa trên bản clone của agent đó nhé, và người dùng cũng có thể tinh chỉnh skill của hệ thống"; "agent
  worker, lead, solo không thể thấy tất cả skill của hệ thống trừ những skill được enable cho chúng, chúng chỉ đề xuất
  skill mới hay update skill thông qua kinh nghiệm làm việc. còn agent skill sẽ thấy được tất cả skill … model embedded
  sẽ đánh giá các skill trùng, gần trùng để đưa cho agent tạo skill xem xét và gộp/tạo skill mới … người dùng cũng có
  thể chọn là chuyển qua agent tạo skill phân tích và có thể bàn bạc … mỗi lần chuyển qua agent tạo skill duyệt là 1
  session, lịch sử chọn vendor/model/effort … phải được ghi nhớ"; "tôi nghĩ việc đính kèm file nên được thêm vào khung
  chat với solo AI và lead AI luôn"; "người dùng kéo thả vào hoặc paste path của file vào thì tự resolve thành file";
  on markitdown: "Kết hợp" (Agenium's converters and markitdown), installed from Settings, adapting when it updates;
  "chúng ta cần những skill thật sự chất lượng". The paid "test a skill" loop was dropped ("Bỏ").

## Context

A skill lived in three places: the library (the user's, the starter skills, the skill writer's drafts once saved), a
role's folder, and an agent's own folder, where ADR-91 let an agent create a skill by itself after its gates. Agents
saw only what was enabled for them, but nothing told the user an agent had added a skill, and two skills on one job
could grow apart. The helpers of ADR-151 forgot each conversation when closed, read only text files, and the models
picked in a form were picked again every time. Documents (PDF, Office) reached no model: `read_file` refused binary
files and the chats took no attachment.

## Decision

1. **One library; agents propose.** Skills written by the skill writer, by hand or imported go to the library, which
   the user edits whole. Worker, Lead and Solo agents see only the skills enabled for them and change only their own
   copy (ADR-73 overlays). `propose_new_skill` keeps every ADR-91 gate (a lesson that held in two contexts, one at a
   time, at most five waiting, the shape, safety and conflict checks) but no longer writes a skill: it files a
   **proposal** (`skill_proposal`, core db) shown in **Needs you**. A proposal like a skill the agent *has* is still
   refused (improve that skill); one like a library skill the agent does *not* have is kept and labelled with it
   (`similar`), as are directive conflicts with such skills (`notes`): they are the user's to settle. A proposal whose
   lesson loses confidence is withdrawn (`withdraw_doubtful_proposals`, after each learning pass). Skills an agent
   created before stay its own, still revised and audited (ADR-91).
2. **Needs you, three ways:** *review by hand* (the skill editor opens the proposed SKILL.md), *discuss with the skill
   writer* (a session that starts from the proposal, its lesson and the tickets it held in, attached as documents),
   or *decline*. Whatever is saved goes into the library and is switched on for the proposing agent
   (`decide_skill_proposal`).
3. **The skill writer sees the library.** Its prompt lists every library skill; the three closest to the request
   (`skills_for_request`: subject words and meaning with the local bge-small model already loaded for memory, no API
   token, vectors cached by text) are given in full. It may write a new skill or set `extends` to improve one, which
   then keeps that skill's name; Agenium checks `extends` names a library skill. A new draft carries the library
   skills close to it (`similar_skills`: *duplicate* at the ADR-92 bars, *near* at 0.30 words / 0.70 meaning) with a
   "Merge into X" button that asks again with `merge_into`; an `extends` draft opens in the editor on that skill with a
   before/after diff.
4. **Helper sessions.** Every chat of the team designer and the skill writer is a session (`helper_session`): model,
   turns, transcript, attachments, last draft, source (`{seed}`, `{proposal}`), status and what was saved from it.
   It is made on the first message, saved after each answer, listed under History (with its cost from
   `helper_call.session`) and reopened where it stopped. It is called by its draft (the skill's or the team's name)
   once it has one, else by its first message; the user can name it (History, or the chat's header), and that name
   stays through later turns (`core_007`: `named`); an empty name gives it its own title back. A turn that only asks
   keeps the session's draft.
5. **Remembered models.** The last model picked is kept in the root (`preference`): the agent form, the team form
   (with the Lead's and planner's models and the per-role option), the skill writer and the team designer, each
   apart. One that cannot run here any more leaves the picker empty with a note; it is never swapped silently.
6. **Documents as Markdown** (`agenium-documents`): built-in converters for PDF (text layer, page by page), Word,
   PowerPoint (slides with notes), Excel, CSV/TSV, HTML, EPUB, notebooks and text; Microsoft's markitdown when
   installed, for the formats it reads better or only it reads (old Office files, Outlook `.msg`), run on a copy in an
   empty folder **inside the sandbox** (its install readable, no network; without a sandbox only by the user's
   unsandboxed choice, ADR-117), falling back to the built-in reading with a note. Up to 30 MB a file.
   - `read_file` reads a document of the worktree as Markdown for every agent, paged like text, marked as a
     converted, read-only copy (the edit tools refuse it).
   - **Attachments** in the Lead/Solo chat, the team designer and the skill writer: a picker, a drop on the window
     (one listener; the top-most chat takes the drop, not the page under it) and a pasted path or file resolve to
     documents the backend reads; they are sent as one `<attachments>` block of material, never instructions, and
     shown as chips in the history.
7. **markitdown in Settings.** Status (Agenium's own copy, one on the PATH, or none); install at a pinned version
   (0.1.9, the document readers only) into `<root>/tools/markitdown`, with uv on the PATH, else a virtual environment
   of a Python 3.10-3.13 found, else a uv Agenium downloads (0.13.1, size and SHA-256 pinned per platform, checked
   before anything runs); uv's Python and cache stay in that folder. A copy is kept only when it reads Agenium's
   sample documents (Word, Excel, PowerPoint, PDF) through the same sandboxed path agents use (the self-test). Once a
   week Agenium looks on PyPI and only says so; an update installs side by side, is self-tested, and the copy in use
   stays when it fails. Remove deletes the folder. The tools prefer Agenium's copy.
8. **A team on each role's model.** The team form can make each role run on the model its template names
   (`default_model`) where this computer runs it (an API provider in the config, an agent CLI found); otherwise the
   role takes the team's model and the user is told. The Lead's and planner's models picked on the form win. Each
   agent runs on its model's backend (`create_team_with_models`).
9. **The helpers' calls count.** Each helper call is recorded (`helper_call`: kind, model, API or CLI, session,
   usage); the usage page shows them as the designer's and the writer's runs.
10. **Skill quality on 12 criteria, locally** (`skill_quality`, no model call): one job; a description that says what
    and when; concrete steps (or a `references/` checklist); proof; tools and how to read them; reasons over rules in
    capitals; compact; general (not tied to one ticket or path); no duplicate or contradiction; safe (no injected
    instruction, no secret); grounded (by the user, the starter library, the user's documents, or a lesson that held
    in two tickets); and **effective**: from the run log, the closed tickets whose runs loaded the skill, how many were
    reopened, against the same agents' other tickets (all others when too few); judged from five tickets on each side,
    flagged when 20 points worse. Shown in the editor's Evaluate, with the writer's draft and a proposal; the effect on
    each library card. Starter skills pass the shape criteria (a test).

## Not done

- No paid with/without test of a skill (dropped by the owner): effectiveness is measured on real work instead.
- A scanned PDF (no text layer) is reported, not OCR'd.
- markitdown's audio, YouTube and Azure readers are not installed (they need the network).
- Effectiveness covers the days the run log keeps (`run_log_keep_days`).

## Verified

- Unit: documents (12 converter tests and the samples), `markitdown::own`, uv archive pins and unpacking,
  `skill_similar`, `skill_quality` (scorecard, effect, the starter library), `newer` versions.
- Integration: `agent/tools/tests/documents.rs` (`read_file` on PDF/DOCX, paged, read-only); orchestrator
  `runtime/design.rs` (remembered choices outlive a restart; a session reopens; close skills shown and merged),
  `team/skills.rs` (proposal → approve → library skill switched on; withdrawn on lost confidence; labelled near a
  skill the agent lacks; a skill loaded in reopened tickets is flagged), `team/teams.rs` (per-role models: API and
  CLI used, an unusable one falls back, the form's Lead model wins).
- Real installs (`tests/markitdown_setup.rs`, run with `--ignored`): with the host's Python, 0.1.9 installed in 93 s
  and read all four samples through the sandbox, `read_document` used it, removed cleanly; with the downloaded uv
  (checksum matched), 0.1.8 installed, the weekly check found 0.1.9, the update ran side by side and the old
  environment was removed, an uninstallable 0.0.0 left 0.1.9 in use, removed cleanly.
- Ubuntu VM (bubblewrap): the crates' lint and tests, and both real installs (the second through the whole fallback:
  no uv, a system Python without `ensurepip`, then the downloaded uv). A link between uv's venv and its Python is not
  mounted by the sandbox, so the folder holding it is read whole.
- End to end in the real app on three API models (DeepSeek V4.1 Flash, `beam-free`, `iceman`) and five CLIs (Claude
  Haiku, Codex gpt-6-luna, agy Gemini Flash, opencode on two free models): helpers with attachments, sessions and
  merges; a proposal discussed, saved and switched on; `read_file` on PDF/DOCX in real runs; markitdown installed and
  used; a team on its roles' models; the UI in Vietnamese and English; remembered choices after a restart. Four
  defects it found are fixed (see `docs/VERIFICATION.md`, 2026-11-01). The opencode CLI on `iceman` answered "Model
  unavailable" to about half its calls: each run's opencode started with an empty cache and its built-in model list
  lacks that model. The root now keeps models.dev's catalogue whole and each opencode run reads its copy
  (`OPENCODE_MODELS_PATH`): 8/8 after.

## Consequences

- New crate `agent/documents` (`lib.rs`, `md.rs`, `pdf.rs`, `ooxml.rs`, `web.rs`, `samples.rs`,
  `markitdown/{mod.rs, own.rs}`).
- Orchestrator: `documents.rs`, `skill_similar.rs`, `skill_proposals.rs`, `skill_quality/{mod.rs, effect.rs}`,
  `helper_sessions.rs`, `preferences.rs`, `markitdown_setup/{mod.rs, install.rs, uv.rs}`, `skill_create.rs`
  (proposals), `skill_write` (library, `extends`, `merge_into`), `team_templates.rs` (`create_team_with_models`,
  `usable_model`), `views.rs` (`SkillProposal`), `maintenance.rs` (run temp sweep), `runtime/side_chat.rs`
  (sessions, usage).
- Store: `core_006.sql` (`helper_call`, `skill_proposal`, `preference`, `helper_session`), `core/skill_use.rs`.
- Tools: `read_file` reads documents. Sandbox: `prepare_temp_dir`, `sweep_run_temp_dirs`. Config:
  `RootLayout::tools`.
- Desktop: `api/{design.rs, markitdown.rs, skill_quality.rs}`, `create_team` (`role_models`), the weekly markitdown
  check in the 15-minute loop.
- UI: `components/{Attachments, HelperChat, HelperHistory}.tsx`, `lib/drops.ts`, `views/{SkillProposalActions,
  MarkitdownSettings}.tsx`, `views/capabilities/{Scorecard, SkillWriter, SkillEditor}.tsx`, `ChatPane.tsx`,
  `Needs.tsx`, `TeamsView.tsx`, `UsageView.tsx`, `skills.tsx`; strings `strings/*/helpers.ts`.
