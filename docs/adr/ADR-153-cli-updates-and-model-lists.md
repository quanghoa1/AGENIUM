# ADR-153: Agent CLIs updated on request; opencode's model list and lost answers

- **Status:** Accepted
- **Date:** 2026-11-01 (Asia/Saigon)
- **Asked by the owner:** "có nên thêm nút update version cho các cli không" then "triển khai hết giúp tôi vì cái nào
  cũng cần, cũng như kiểm tra kĩ các cli khác có bị tương tự như opencode không"; before that, "fix issues opencode".

## Context

The opencode CLI on OpenCode Go's `iceman-contributor-free` answered "Model unavailable" to about half its runs. Probed
outside Agenium: each run's opencode starts with an empty cache and fetches its model list itself; a prompt sent before
that fetch landed found only the list built into opencode, which lacks that model (`provider.no-route`). Every CLI keeps
such a list in its user home (Claude `cache/model-catalog`, Codex `models_cache.json`, Grok `models_cache.json`), and
Agenium's per-run homes carry none of them. A version of a CLI newer than the one Agenium checked may change the flags
or the server API Agenium drives; the user had no way to see versions or update from the app.

## Decision

1. **opencode gets the model catalogue.** The weekly price refresh keeps models.dev's catalogue whole in the root
   (`models-dev.json`; fetched at once for a root without it), and each opencode run reads a copy in its own home
   (`OPENCODE_MODELS_PATH`).
2. **An answer that left no text event is read back.** opencode sometimes ends a step with output tokens and
   `finish: stop` but no text event (`iceman`, about one turn in ten); the run completed with an empty answer. The bridge
   follows each turn's steps (`TextWatch`) and, when the last one showed no text, reads the session's newest assistant
   message (`GET /api/session/<id>/message`) and sends its text before the turn's end. When that message is empty too
   (`content: []`: the model returned no text), the model is asked once more in the same turn, and the turn fails if it
   answers nothing again, rather than completing with an empty answer.
3. **The other CLIs were checked, not changed.** In the real app, Claude Haiku and Sonnet, Codex gpt-6-luna and
   gpt-6.1-sol, agy Gemini Flash, Grok 4.7 and opencode `beam-free` each answered six chat turns (three in one Solo,
   three Solo agents at once), every run in a fresh per-run home, with their stderr logged: no failure and no word of an
   unknown or unavailable model. Their lists load before a turn or are built in; seeding their caches is not needed.
4. **Updating a CLI** (Providers & CLIs, the version column): the version installed, the newest release (npm's `latest`
   for Claude, Codex and opencode; `grok update --check --json`; agy cannot tell), and the version Agenium last verified
   end to end (`CliKind::verified_version`), flagged when the installed or offered version is newer. **Update** asks
   first and runs the CLI's own updater (`claude update`, `codex update`, `agy update`, `opencode upgrade`, `grok
   update`): the install is the user's, set up their way, so Agenium never installs over it. It waits while a run of
   that CLI is in progress (its program is in use). Afterwards the CLI is found again and asked one short question on a
   light model (a helper call, `cli-check`, counted in Usage); when that fails and the updater can install a given
   version (`claude install <v>`, `opencode upgrade <v>`, `grok update --version <v>`), the version it had is offered
   back.
5. **Diagnostics:** the opencode bridge logs every server event, and every CLI's non-event output (stderr) is logged, at
   debug level (`AGENIUM_LOG=agenium_cli_backend::opencode::events=debug,agenium_cli_backend::stderr=debug`).

## Not done

- No automatic update: an update is the user's decision, with a check after.
- Codex and agy cannot install a given version through their updater: after a failed check the user goes back by hand.

## Verified

- Unit: the update commands per CLI and what each cannot do, version comparison, the verified versions, the light model
  a check asks; `TextWatch` on the event orders captured from the real runs (a turn with text, tool steps then text, the
  turn without text) and the message read-back; the run home's catalogue copy.
- End to end: before the catalogue, `iceman` through the opencode CLI failed about half its calls; after, 8/8 helper
  calls and no "Model unavailable" in 36 chat turns; 2 of 18 `iceman` turns still got no text twice and now fail
  visibly (`beam-free` 18/18); the stress of point 3 on every CLI, twice. The update flow in the real app: `claude
  update` (already newest), Claude found again, its check answered. Probes outside Agenium reproduced both opencode failures (no catalogue:
  "Model unavailable" on the first turn every time; the resumed turns' event orders).

## Consequences

- `agent/cli-backend/src/updates.rs` (new), `opencode/mod.rs` (`use_models_catalogue`), `opencode/bridge.rs` (split out
  of `mod.rs`: the bridge, `TextWatch`, `final_text`, the event log), `run/process.rs` (stderr log); `agent/llm/src/pricing.rs` (`fetch_models_dev`); `foundation/config/src/root.rs`
  (`models_dev`).
- `orchestration/orchestrator/src/cli_updates.rs` (new), `prices.rs` (keeps the catalogue), `app.rs`.
- Desktop: `api/cli_updates.rs` (`cli_updates`, `update_cli`), the price loop refreshes a root without the catalogue.
- UI: `views/capabilities/CliUpdates.tsx`, `ProvidersView.tsx`; strings `cli.*`, `usage.helper.check`.
