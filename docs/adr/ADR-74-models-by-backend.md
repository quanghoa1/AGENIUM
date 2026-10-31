# ADR-74: The model list follows the backend

Status: Accepted (2026-10-25, owner: "with API list only the models usable through the API, with CLI only the models
that CLI can use; the same for every vendor").

## Decision

1. **API.** A provider lists what its API serves: for OpenCode (`opencode.ai`), its free models (`*-free`, or priced 0
   in the catalogue) answer only to OpenCode itself and are left out of the provider's list and of its hand-written
   model list; an agent set to one of them on the API backend is refused before the run with the way out (the opencode
   CLI backend, ADR-68). Other providers list what their endpoint reports, as before.
2. **CLI.** Each driven CLI lists what it runs with the user's login, without a model turn: codex through its
   app-server `model/list` (hidden models left out, the default first; scratch `CODEX_HOME` like a run), agy through
   `agy models`, Claude Code its `--model` aliases and full ids (it has no list command), opencode OpenCode's free
   models still served (ADR-68). Listed at most every 5 minutes in the desktop app.
3. **FREE tag** on the CLI side only for the opencode list (the other CLIs run on their subscriptions).

## Evidence

- `agy::tests::lists_models`; real app: codex 7, agy 13, Claude 7, opencode 9; OpenCode Go API list 36 models, none
  free.

## Amendment (2026-10-31)

The opencode CLI's list is the user's OpenCode account (ADR-68, amended): only its `-free` models and `big-pickle` are
tagged FREE, the others say whether the Go subscription or Zen's credit pays. The lists are kept and reloaded as
ADR-150 says.
