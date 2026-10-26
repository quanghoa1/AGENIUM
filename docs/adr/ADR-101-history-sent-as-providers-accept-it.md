# ADR-101: The history is sent as providers accept it; cut tool calls are not run

Status: Accepted (2026-10-26). Evidence: `agent/llm/src/history.rs` tests, `agent/loop/tests/loop.rs`
(`calls_cut_by_the_output_limit_are_not_run`, `thinking_is_kept_for_the_model_that_wrote_it`), the vendor-switch test in
`orchestration/orchestrator/tests/runtime.rs`. Source: the bug history of the pi coding agent
(`docs/reference/PI-COMPARISON.md`), where "poisoned sessions" — one stored block that makes every later request
fail with HTTP 400 — were the most damaging class.

## Decision

1. **A pass over the copy sent with every request** (`agenium_llm::history::prepare`); the stored history keeps
   everything:
   - thinking stamped with another model is dropped (a signature is valid for the model that wrote it; the loop stamps
     each block with the model of the run). Unstamped blocks from older history replay as before;
   - a tool call with no result gets one (`not run: …`, an error), placed right after the call; a result whose call is
     not in the history is dropped;
   - empty text blocks and messages left empty are dropped; an empty tool result is sent as `(no output)`;
   - tool-call ids are made safe for every provider (`[A-Za-z0-9_-]`, at most 64; changed ids get a hash suffix so
     they stay unique), identically on the call and its result.
2. **Calls in an answer cut by the output limit are not run.** The last call's arguments may be incomplete (a half
   written file). Each call gets an error result telling the model to send it again in smaller pieces; it counts as an
   invalid call, so three in a row stop the run.

## Consequences

- A vendor switch, a cancelled turn or a run that died between a call and its result can no longer make the next
  request fail.
- Thinking is not carried across a model switch (it was dropped by the other vendors' adapters already; now also
  between two models of the same vendor).
