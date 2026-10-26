# ADR-112: Monaco for side-by-side diffs and xterm.js for a run's shell session, loaded on demand

Status: Accepted (2026-10-26). Implements the UI libraries of decision 3a (Monaco for code/diff, xterm.js for the agent's
terminal) that ADR-60 left open. Evidence: the running app (2026-10-26): a ticket's Changes tab opens
`deliverables/hello.txt` side by side, and a run with 22 shell steps shows its session in the terminal; no console
errors.

## Decision

1. **Diffs.** The unified-diff view stays the default. Each file of a ticket's Changes tab has a "Side by side"
   button that opens Monaco's diff editor. Syntax colouring comes from Monaco's basic tokenizers; there are no language
   services, so only the plain editor worker is shipped. The two sides come from a new command, `ticket_file`:
   the file at the ticket's base commit, and the worktree's file (the merge commit's, once the worktree is gone).
   The command accepts only a normal relative path. A binary file or one over 2 MB is not shown.
2. **Terminal.** A run page with shell steps has a "Terminal" button. It shows the run's whole shell session (each
   command after a green prompt, its output, a red `[failed]` mark) in a read-only xterm.js terminal with 20 000 lines
   of scrollback. The terminal keeps the commands' own colours and follows new steps.
3. **Bundled, on demand, pinned.** Monaco (0.57.0) and xterm (6.0.0, addon-fit 0.11.0) are npm dependencies with
   exact versions, both MIT. They are bundled with the app (Monaco is never fetched from a CDN) and each loads in
   its own chunk the first time it is opened: Monaco is about 2.7 MB, xterm about 330 kB. The main chunk is unchanged
   (about 740 kB).

## Not done

Editing skill files with Monaco (decision 3a lists it) waits for a skill file editor in the app.
