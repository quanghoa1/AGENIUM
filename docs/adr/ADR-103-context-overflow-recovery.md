# ADR-103: A context overflow is recovered from, and compaction cannot make things worse

Status: Accepted (2026-10-26). Evidence: `agent/llm/src/error.rs` (`overflow_tests`), `agent/loop/tests/loop.rs`
(`an_overflow_clears_old_results_and_sends_again`, `an_overflow_with_nothing_to_clear_compacts_then_gives_up`,
`notes_cut_by_the_output_limit_are_not_used`, `a_large_new_result_counts_before_the_next_request`). Amends ADR-100.
Source: the pi coding agent's bug history (`docs/reference/PI-COMPARISON.md`): overflow detection per vendor, summaries
cut at the output limit stored as if whole, large tool results sent before the threshold was checked, and summary
requests larger than the context they summarised.

## Decision

1. **Overflow is recognised** (`agenium_llm::is_context_overflow`) from the wording of each vendor (Anthropic,
   OpenAI, Gemini, DeepSeek, OpenRouter, LiteLLM, Ollama, LM Studio, llama.cpp, vLLM). A 429, a throttle or a quota
   error is never overflow, nor is a bare 400/413 without such wording.
2. **One recovery per overflow:** clear long tool results except the last two (without the "frees a tenth" rule),
   or, with nothing to clear, write notes (ADR-100), then send again. A second overflow in a row fails the run with
   the provider's message.
3. **The threshold counts what was just added:** tool results appended after the last request are estimated at four
   characters a token before the next request is sent.
4. **Notes must be whole and must fit:** notes cut by the output limit are not used (the run checkpoints instead);
   the notes request carries no thinking and tool results cut to 2 000 characters.
5. **A wordless refusal of a huge request.** On the real model (OpenCode Go, deepseek-v4.1-flash, 2026-10-26) a request
   of about a million tokens came back as HTTP 400 with no error text (`{"model": "…"}`, once an empty body), while
   requests of 50 000 to 500 000 tokens were accepted. A 400 or 413 without error text counts as an overflow only when
   the request is estimated above 100 000 tokens; an ordinary bare 400 is still reported as it is.

Real-model checks of ADR-97..105: `quality/eval/examples/real_checks.rs`, 7/7 on deepseek-v4.1-flash
(`docs/reference/bench-raw/real-checks-deepseek-2026-10-26.txt`).

