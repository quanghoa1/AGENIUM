# ADR-85: The judge can skip memory extraction on routine work (memory gate)

Status: Accepted (2026-10-25; owner: "continue Jev" — a third typed use that saves the agent's tokens). Follows
[ADR-82](ADR-82-quick-skill-pick.md); same fail-open rule as [ADR-81](ADR-81-judge-two-modes.md).

## Context

After every ticket close/reopen Agenium runs the memory pipeline (`learn`): one API call on the agent's model (or the
`[cli]` fallback), `max_tokens` 4000, over a transcript up to 60k characters, to extract durable facts, preferences and
lessons. On routine tickets that produced nothing worth remembering, that call is pure cost — tokens, latency, and a
chance to add noise to memory.

## Decision

Add an optional, fail-open **memory gate**. When `[judge].memory_gate` is on, before the extraction call Agenium asks
the Jev judge one Noul — *does this episode hold anything worth remembering?* — and **skips extraction only when the
judge is confident the answer is no and there is nothing to reconcile**:

- Gated to episodes with **no signals, no memories shown, and no reopen** (`ep.signals.is_empty() && ep.used.is_empty()
  && !ep.reopened`), so the reuse-signal bookkeeping (confidence up/down for shown memories) never has anything to do
  and is not lost.
- `App::worth_remembering(episode)` returns `Some(false)` only when the Noul is at or below `1 - MET` (0.3);
  `Some(true)` at or above `MET` (0.7); `None` when unsure. The gate skips only on `Some(false)`.
- **Fail-open**: off by default, and `None` (so extraction runs as usual) whenever the judge is off/disabled, the flag
  is off, the verdict is unsure, or the call fails. A key/quota failure disables the judge (ADR-81).

## Consequences

- `foundation/config`: `JudgeSection.memory_gate` (off by default).
- `orchestration/orchestrator`: `App::worth_remembering`; `runtime/learn.rs` skips extraction (logging
  `{"skipped": ...}` and stamping the learned marker) when safe and the judge is confident.
- Test: `team.rs::memory_gate_skips_only_on_a_confident_no` (mock Jev: confident-no → skip, confident-yes/unsure → not).
- Savings scale with the agent model's price and the share of routine tickets; see
  [the Jev comparison](../jev-comparison.md). The quality benefit (less memory noise) is independent of price.
