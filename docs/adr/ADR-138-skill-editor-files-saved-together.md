# ADR-138: The skill editor writes a skill with its references, scripts and assets in one step

Status: Accepted (2026-10-29). Owner: "tôi thấy UI phần thêm skill hơi xấu, bạn check lại thử, cũng cần verify lại phần
thêm skill, rồi thêm reference, script của skill nữa". Amends ADR-88/90 (skill authoring in the app).

## Context

The editor in Skills & MCP (Monaco on a SKILL.md):
- left an empty band under an editor of fixed height, and had the frontmatter (name, description, tags) typed as YAML;
- added a reference or script through the browser's `prompt` box, only after the skill was first saved, and saved
  only the file on screen: an edit of two files needed two saves, and leaving lost the other silently;
- could not save a new SKILL.md that names its reference (`references/api.md`) before that file existed: the library
  refuses a skill whose named files are missing, and the files could only be added after the skill was saved;
- had no way to remove a file, to bring one from disk, or to keep `assets/`.

## Decision

1. **One save, one validation**: `Registry::write_with_files` writes SKILL.md together with the files added or changed
   and the files removed. The folder as it would be after the save is validated in a staging copy first (named files
   present, sizes within limits, the frontmatter name the folder's); only then is the real folder changed, each file
   through a temporary file renamed over it, SKILL.md last. A file goes under `references/`, `scripts/` or `assets/`
   with plain names (`skills::file_path`). The app's `save_skill` takes the files (text, or base64 for one that is not
   text) and the removed paths; the per-file save is gone.
2. **The editor** (`SkillEditor.tsx`): name, description and tags as fields (other frontmatter kept as it is), the
   instructions in Monaco filling the dialog, and the files grouped as References, Scripts and Assets, each with a new
   file named in place, an upload from the computer (up to 2 MB; text kept as text) and a delete. A file the
   instructions do not mention is flagged; images are shown; unsaved changes are counted, Save writes them all, and
   leaving with changes asks first. A new skill takes files before its first save.
3. **The library page** offers writing a skill and importing one side by side.

## Consequences

- A skill is authored in the app as it is used: the agent loads its instructions, reads its references and runs its
  scripts from the skill's folder (read-only) through its shell, under the sandbox.

## Verified (2026-10-29)

- Test: `a_skill_is_written_with_its_references_and_scripts` (a new skill with the reference it names; add, change and
  remove together; paths out of the three folders refused; a removal the instructions still need refused, nothing
  changed).
- UI (`ui-tests/skills.mjs`, the real app): 16/16 — the editor fills the dialog; fields; a reference and a script added
  in place and an image uploaded before the first save; one save; the files on disk; the image shown; editing shows the
  files, removes the script; the unsaved-changes guard; the skill deleted; no page error. Typing in Monaco is checked
  with real clicks and keys (this Monaco takes input through a native edit context, not its textarea).
- Real agents (`agenium-eval --task use-skill`, opt-in): the release code exists only as the output of the skill's
  script and the format only in its reference. DeepSeek (API), Claude Code Haiku and Codex gpt-6-luna each loaded the
  skill, ran `scripts/stamp.ps1` from the skill's folder through Agenium's sandboxed shell, and wrote the note in the
  reference's format: PASS.
