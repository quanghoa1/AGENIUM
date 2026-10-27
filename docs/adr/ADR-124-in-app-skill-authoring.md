# ADR-124: Author skills in the app (tiers 2 and 3)

Status: Accepted (2026-10-28; owner asked to finish the skill system in the app: "sửa/tạo skill trong app"; and, on the
first cut: "nên có nút đánh giá bên cạnh nút lưu, thay vì lưu ngay"; "reference sẽ thêm ở đâu?").

## Context

The skill library could be read and imported in the app, but not authored there — a new or changed SKILL.md meant
editing files on disk and re-importing. The three evaluation layers (shape lint ADR-88, conflicts ADR-92, Jev quality
ADR-90) ran only at import. And a skill's depth lives in tier-3 files (`references/`, `scripts/`, progressive
disclosure, ADR-61/88), which an import-only flow could not create in the app.

## Decision

1. **Monaco editor in the Skill & MCP view.** "New skill" starts from the section skeleton with a frontmatter template;
   "Edit" on a card opens the skill's SKILL.md. The editor is a dialog sized to the content (it is rendered through the
   overlay, not the fixed-width modal, so Monaco does not overflow).
2. **Tier 2 and tier 3 in one editor.** A file list shows `SKILL.md` and the skill's `references/` and `scripts/`
   files; each opens in Monaco with syntax highlighting for its type. "Add file" creates a new `references/…` or
   `scripts/…` file (enabled once the skill exists on disk — the folder is the SKILL.md frontmatter name).
3. **Evaluate, then save.** An "Evaluate" button runs the shape lint and (judge on) the Jev quality read on the
   *unsaved* text, so the author sees the advice before committing. "Save" writes the active file and shows the checks;
   saving SKILL.md also runs the conflict check. The folder of a new skill is its frontmatter `name`.
4. **Writes are validated and contained.** `Registry::write` validates the whole folder in a staging copy before
   replacing SKILL.md (bad content never overwrites a good file), and requires the frontmatter name to equal the target.
   Reference/script paths are restricted to `references/` and `scripts/` with safe segments (no traversal, no absolute).

## Verified

- Create a skill, add `references/notes.md`, read it back, remove it — all through the commands the buttons call.
- Evaluate (dry run) returns the shape and Jev scores without saving; it flagged vague steps ("actionable steps: no").
- `Registry::write` test: create, refuse a second create, edit in place, refuse a name mismatch, a bad body does not
  overwrite the good file, refuse editing a missing skill.
- Workspace 446 tests, desktop tests, clippy and UI smoke pass; the editor checked on the running app (layout, dry-run
  evaluate, create + reference).

## Consequences

- `agent/skills/src/lib.rs` (`Registry::write`, `Origin: Copy`), test `write_create_and_edit`.
- `orchestration/orchestrator/src/registry.rs` (`save_library_skill`).
- `apps/desktop/src/manage.rs` (`skill_skeleton`, `save_skill`, `evaluate_skill_text`, `skill_file`,
  `save_skill_file`; `skill_text` returns `raw`), `main.rs` registrations.
- UI: `components/MonacoEditor.tsx` (new), `views/Capabilities.tsx` (`SkillEditor`), `lib/api.ts`, i18n.
