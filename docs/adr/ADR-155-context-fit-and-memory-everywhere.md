# ADR-155: A conversation fits the model it runs on; the context is shown; memory in every run

- **Status:** Accepted
- **Date:** 2026-11-01 (Asia/Saigon)
- **Asked by the owner:** whether agents remember their skills and MCP, what they are doing and how far they got;
  whether an agent shows its context, how full it is and compaction; what happens when an agent moves from a 1M-token
  model to a 256K one ("đôi khi người dùng không còn model nào khác nên đành chuyển về 256k, liệu có cơ chế compact
  rồi nạp lại cho model 256k không, xem cách các cli khác xử lí ra sao"); then "làm cả 4".

## Context

A review of the code found:

- Skills (names and descriptions in every run's system prompt, bodies by `load_skill`) and MCP (bindings resolved per
  run, tools listed or found with `tool_search`) were already there.
- A ticket's progress carried over through its comments and the replayed conversation.
- **Gaps:**
  - Memory was recalled only in a fresh work run, so the Lead's chat facts (ADR-98) were never read back.
  - Workers never saw the plan.
  - A CLI checkpoint could leave an empty note, and the next run started with nothing.
  - No context gauge: the UI's "tokens in" summed every request of a run.
  - Moving to a smaller model broke a resumed ticket:
    - the replayed conversation was sent unchecked;
    - in-run compaction could not shrink it (it folded only the run's own messages);
    - after the provider's refusal old tool results were cleared, which saved a history of tool results; a history
      of long writing still did not fit, the run ended `Failed`, and every stall nudge replayed it.
  - A model nothing described was assumed to have 200 000 tokens.
  - The chat history budget (50k) did not follow the model.
  - A CLI session was resumed on another model.

How others do it:

- **opencode** compacts when the conversation reaches the window minus a buffer (a share of each model's own window,
  so a switch moves the point), summarises under fixed headings, keeps about 15 000 tokens of recent conversation, and
  compacts and retries once when the provider refuses a request.
- **Gemini CLI** compresses at a share of the current model's limit (0.5 by default), keeping the recent part.
- **Codex** caps its auto-compact limit at 90 % of the current window.
- **Claude Code** keeps the history on a model switch and relies on auto-compaction (and `/compact`) to bring it
  under the new window.

None reads a history longer than the new model's window in one request. We take the shape (a window-relative
threshold, a fixed-heading summary, a verbatim recent tail, a preflight check) and add what a smaller window needs:
folding piece by piece.

## Decision

1. **The window:** the provider's model list or a hand-set `context_window`, else the models.dev catalogue the root
   keeps (`model_windows.rs`; a CLI's model through its vendor, an alias such as `haiku` through its family), else
   200 000 for an API model and "not known" for a CLI.
2. **Fit before sending (API loop, `agent/loop/src/fit.rs`).**
   - Before a run's first request its prompt is estimated. Over 85 % of the window, everything after the work order,
     the resumed conversation included, except a recent tail of 15 % of the window, is folded into notes.
   - The notes go piece by piece: each piece of the transcript (tool output cut to 3 000 characters) is sized to the
     window, and the notes so far are carried into the next piece.
   - The notes and the files read and changed replace that part, and the run's stored conversation is replaced by the
     result, so the next run resumes it as it is.
   - In a run, compaction now also covers the resumed conversation. When even the cut copy of the older part would not
     fit, compaction folds piece by piece instead of failing.
   - A chat turn keeps folding only its own messages (the chat history has its own summary).
3. **CLI.**
   - A kept session resumes only on the model that wrote it (`cli:model`).
   - A conversation shown again as text that is over 70 % of the model's window is folded by the agent's own model
     through the CLI (`fit_cli_order`), the same way.
   - Claude's and opencode's per-request usage feeds the gauge.
4. **Chat history.**
   - The budget is at most a fifth of the Lead's window (`budgets`) and the verbatim tail at most 6 %.
   - The summary is written by the Lead's own model (API or CLI), piece by piece.
5. **Gauge.**
   - Each run records the context of its last request and its window (`UsageSummary::context_used/context_window`,
     kept at the run's end).
   - The run page, the Live dock and the chat page show "used / window · %" (amber from 60 %, red from 85 %).
   - The run page reads compaction, folding and the other controls as sentences.
6. **Memory in every run.**
   - A chat turn recalls by the user's message. The memories go in the turn's message, stored and replayed as sent
     like the project state; a memory the history already carries is not repeated. A system block that changed every
     turn would make the whole chat history a cache miss at every turn.
   - The inbox recalls by its items, a review by its ticket, the planner by the spec. Each is logged and credited.
   - A fresh work run also gets the plan's outline, with each item's tickets and the worker's own item marked.
7. **A checkpoint with next to no note** (under 40 characters) resumes the conversation instead.

## Verified

Measured with real models (`quality/eval/examples/context_switch.rs`, raw results in
`docs/reference/bench-raw/context-switch-2026-11-02`). An agent's ticket history, built on OpenCode Go's free
`beam-free` (1M), is resumed on `iceman-contributor-free` (262 144). There are 50 log files with 100 error codes, 96
of them only in the history.

| Case | Before (c119d77) | After |
|---|---|---|
| History of long writing (419 414 tokens by the provider's count) | Refused at every run, the ticket stuck (1 of 1) | Done, 100/100 codes, 96/96 from the history (2 of 2); about 466K uncached input, the history read once to fold it |
| History of tool results | Done 3/3; mean 310K tokens, 60K uncached; 1 run stopped on a loop | Done 3/3; mean 269K tokens (-13 %), 54K uncached (-11 %); no run stopped |
| Chat, a preference in memory (4 turns, `beam-free`) | Used in the first answer in 1 of 3, in the last in 0 of 3 | Used in the first and the last answer in 4 of 4; about 21K tokens a chat either way |

Found by measuring and fixed before these numbers:

- Keeping four tool results in the preflight cost a third more uncached input than the two the refusal path keeps.
- The token estimate counts log-like text at two thirds of the provider's count, so after a refusal the fold goes
  first.
- The free model ran two `read_file` calls together (`{"path": "a"}{"path": "b"}`). Answered as invalid JSON, it
  repeated the call until the run stopped on the repeat. Such calls are now taken apart (`anthropic::split_args`,
  used by the OpenAI-compatible adapter).
- The chat memory had been a per-turn system block, which made the history a cache miss at every turn.

Unit tests:

- Unit tests:
  - a conversation of about 60 000 tokens resumed on a 20 000-token window is folded in at least 3 pieces;
  - every request fits the window, and the stored conversation is replaced;
  - the gauge goes from over the window to under 85 %;
  - window lookup by provider, CLI alias and model id;
  - the chat budgets;
  - the earlier compaction and overflow tests still pass.

## Consequences

- `agent/loop`:
  - `fit.rs` (new);
  - `lib.rs` (preflight, compaction scope, `context`/`replaced` reports);
  - `context.rs` (`cut_for_notes`; the fold as compaction's last resort);
  - `contract.rs` (`Observer::context`, `Observer::replaced`).
- `agent/cli-backend`: `Event::Context`, sessions keyed by model, `SessionStore::would_resume`.
- `foundation`: `UsageSummary` context fields; `AgentStore::replace_messages`.
- `orchestration/orchestrator`:
  - `model_windows.rs`;
  - `runtime/{cli_fit,recall}.rs`;
  - `chat_history.rs`;
  - `models.rs` (`window_of`);
  - `bridge.rs`, `execute.rs`, `context.rs`, `lead_runs.rs`, `ticket_runs.rs`, `scheduling.rs`, `sched.rs`.
- UI:
  - `components/ContextGauge.tsx`;
  - the run page (`ControlStep`), the Live dock and the chat page;
  - strings `context.*`, `control.*`.
