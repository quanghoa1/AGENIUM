# ADR-44: agenium-vcs drives the git CLI; journal-first intents; in-process fault injection

- **Status:** Proposed — deviates from REPO-LAYOUT (git2/gix); needs owner confirmation
- **Date:** 2026-10-23 (Asia/Saigon)
- **Context / decision / consequences:**

- Chosen: `git` CLI (min version checked by the setup wizard) instead of git2/gix. Reasons: identical behaviour to what users/agents see in the sandbox, worktree + rebase + ff-only merge semantics are first-class, no libgit2/OpenSSL build burden, SSH via `GIT_SSH_COMMAND` with a temp 0600 key (ed25519 generated with `ssh-key`).
- Every mutating operation is journal-first: `<op>` intent event, then git action(s) tagged with the commit trailer `Agenium-Intent: <op>`, then `<op>:end`. `reconcile()` replays from journal + git state on open (M0-10).
- Merge queue = a lock around rebase -> CheckRunner -> ff-only merge. The pre-merge check runs unsandboxed in M0 (hook exists).
- Fault injection (`FaultPlan`) is **in-process** (error at a named point), covering 24 injection cases (8 spike points x variants). It is not a real `kill -9`; only the push child is genuinely orphaned in tests.
