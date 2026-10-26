# ADR-100: In-run context management — clear old tool results, then compact, then checkpoint

Status: Accepted (2026-10-26). Evidence: `agent/loop/tests/loop.rs` (`old_tool_results_are_cleared_when_the_context_fills`,
`a_full_context_is_summarised_and_the_run_goes_on`, `after_two_compactions_the_run_checkpoints`; the first two fail
when their feature is switched off). Amends decision 36 (the checkpoint at 85 % of the window).

## Context

A run whose context reached 85 % of the model's window stopped with a checkpoint: the model wrote a progress note,
the run ended, and the next run started from the note alone. Everything the agent had read was gone, a new run had to
be scheduled, and a long ticket could end in a chain of checkpoints. Most of a full context is old tool output (files
read, command output) that the agent no longer needs word for word.

## Decision

Three stages, by the share of the window the last request used (the model's real window when its provider reports
it, else 200 000 tokens):

1. **From 60 %: clear old tool results, no model call.** The content of tool results longer than 400 characters is
   replaced by `[cleared to save context: N characters; call the tool again if you need it]`, except in the last four
   tool-result messages. The call and its result stay paired. It is done only when it frees at least a tenth of the
   window, so the prompt cache is not broken for little.
2. **From 85 %: compact, and go on.** The model writes notes without tools: the task, what is done, what it found that
   it still needs, what remains, the next step. The run's conversation after the work order is replaced by
   `[Context compacted: …]` and the notes. The work order (system, memory, ticket, comments) is untouched. The run
   goes on, and what it keeps for a later run starts with the notes. At most `max_compactions` times per run
   (default 2; 0 = the old behaviour).
3. **Then: checkpoint**, as before (the note, the run ends, the next run resumes from it).

**Notes format (2026-10-26, learned from pi):** the model writes under fixed headings (Goal; Constraints and
preferences; Progress — Done / In progress / Blocked; Key decisions; Next steps; Critical context) and, on a second
compaction, updates its earlier notes instead of starting over. Agenium appends the files the replaced conversation
read and changed (`<read-files>`, `<modified-files>`, from its tool calls), carried over from compaction to compaction.

## Consequences

- Long tickets run further in one run and keep what they need; fewer runs end at a checkpoint.
- Clearing and compacting rewrite earlier messages, so the prompt cache misses once each time.
- The chat with the Lead reloads its whole history each turn; a very long chat still checkpoints its turn (not
  covered here).
- Measured with a scripted model only; not yet on a real long ticket.
