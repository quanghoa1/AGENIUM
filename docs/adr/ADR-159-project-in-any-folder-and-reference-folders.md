# ADR-159: A project in any folder; reference folders

- Status: accepted (2026-11-11)
- Related: ADR-154 (the work folder chosen at install), ADR-121 (picks readable by every run), ADR-152 (attachments),
  ADR-53/54/55 (the sandbox's read roots), decision 7 (a project is a folder referenced by path), decision 19 (the
  user may edit main while the project is paused)

## Context

Users asked to make a project in a folder of their choice, which may be empty or already hold their files or a
repository, instead of only `<root>/projects/<name>` or a clone there. They also asked for more than one folder: some
are reference material only (an SDK, documentation, another codebase) that the agents should read to do the work.
The core already took a full path (the CLI's `--path`), but it did not handle a repository on a branch other than
`main` (it refused it), would have committed the user's uncommitted changes in its first commit, and took a folder
inside another repository for that repository.

## Decision

1. **The project form takes a full path** (typed or picked with *Choose…*, the system's folder dialog) beside a
   folder name or a repository URL. Before creating it, the form says what will happen, from `inspect_folder`:
   - **empty folder**: the project starts there, as in the projects folder;
   - **a folder of files, not a repository**: `git init`, a `.gitignore` for what is rebuilt (`node_modules/`,
     `target/`, `dist/`, `build/`, `.venv/`…) when the folder has none, and the files become the first commit; the
     form shows how many files and how many bytes (an estimate that skips those folders). A secret found by the
     staged-changes scan (decision 34) stops it: the repository made a moment ago is removed, the files stay as they
     were, so the user can leave the file out and try again;
   - **a repository**: taken as it is, on the branch checked out (it becomes the project's main branch; no branch
     checked out is refused with a message). Only Agenium's own files are committed (`agenium.toml`, the layout's
     `.gitkeep`s): the user's staged and unstaged changes stay theirs. Merges reach that working tree as before
     (`merge --ff-only`, which git refuses rather than overwrite a changed file);
   - **a folder inside another repository** is refused, naming that repository: pick it, or a folder outside it;
   - **Agenium's own data** (the root, or a folder of it other than the projects folder) is refused.
   Nothing of a picked folder is ever deleted by Agenium; worktrees go in its `.agenium/` (excluded locally, as in
   every project).
2. **Reference folders**: a project setting (`references`: full path and an optional note), set in the form and on
   the project's card, checked when saved (a folder that exists; not the project's folder or one holding it; not
   Agenium's data; duplicates dropped). Every run of the project (chat, planning, work, review) gets them as read
   roots of its file tools and of the sandbox, so a command may read them too; writes stay refused there (the tools'
   write check and the sandbox's writable roots, which are the worktree only). The manifest names them, notes
   included, with "read-only … your work goes in your worktree". They are not copied, not committed, not pushed.
3. **The team's branch** (the owner, 2026-11-11). A name alone makes the project in the projects folder on `main`, as
   before; a folder that is no repository yet starts on `main`. For a repository, cloned or picked, the form asks:
   - **its own branch** (the clone's default branch, the branch checked out in a picked folder): merged work lands on
     it, in that folder; agents start from its last commit;
   - **a new branch** (default `agenium-work`; `agenium` and `agenium/…` are refused, git cannot have a branch
     `agenium` beside the tickets' `agenium/<KEY>`): a clone checks it out (the folder is Agenium's); a picked repository keeps the
     user's branch checked out and gets Agenium's branch, made from the current commit, in a worktree of its own, `.agenium/base`. The layout is committed there and
     `agenium.toml` stays local (`.git/info/exclude`), so the user's folder and branch are as they were. The Lead's
     and the planner's runs and `AGENTS.md` read that checkout.
   A merge moves the team's branch where it is checked out (`merge --ff-only` in the project folder or in
   `.agenium/base`); checked out nowhere (the user switched the project folder to another branch), only the ref moves
   (`update-ref` from the commit it was rebased on), and the user's folder is left alone. Before, it fast-forwarded
   whatever the project folder had checked out. As before, when the repository has an `origin`, the team's branch
   is pushed there each time a ticket closes: with *its own branch* that is the user's branch, with *a new branch*
   only Agenium's.
4. **One working folder per project.** Work in two repositories at once (worktrees, merges and reviews over several)
   is a larger change, left until it is asked for.
5. `Repo::is_repo` is true only at a repository's top level (a folder inside one is not a repository of its own).

## Consequences

- A project can be made where the user's code already is; their uncommitted work is not touched.
- Agents can read an SDK or docs in place without anything being copied into the project.
- The user can keep coding on their own branch in their own folder while the team works on its branch beside it.
- The CLI's `--path` follows the same rules (a relative path is made absolute).
