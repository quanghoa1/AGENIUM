# ADR-72: Lessons with cue and signal; rules or hints by scope; forgetting and consolidation

Status: Accepted (2026-10-25, M4.5, decision 35 and ADR-51 §8).

## Decision

1. **A lesson needs a cue and a signal.** The extractor gives each lesson a `cue` (when it applies); a lesson without
   one is dropped. Lessons come only from episodes with a signal, which Agenium derives from the ticket: the review
   reopened it, it was accepted after reopens, a merge or pre-merge check failed, or the user commented on it (that last one removed, ADR-158 decision 13).
   Without a signal only facts and preferences are kept. Evidence stays the episode's outcome.
2. **Rule or hint by scope.** A recalled lesson whose every scope key matches the work's context is shown as
   "Rule (lesson[, held in N contexts]): ... (when: cue)"; from a nearby context (a key the work does not set) as
   "Hint from another context, verify first: ..."; a conflicting key still keeps it out (ADR-51 GM-2).
3. **Validated contexts.** A lesson shown to a run whose ticket closes cleanly records that context
   (`<project>/ticket:<key>`, agent.db migration 3, column `validated`); two or more make it a candidate for the
   agent's skill overlay (decision 35, next ADR).
4. **Forgetting and consolidation** after every learning episode: the weight of each live fact halves for every 90
   days it goes unused (since its last use, else its creation) and below 0.2 it is no longer retrieved (kept, never
   deleted); live facts with the same kind, scope and text (up to case and spacing) fold into the most trusted copy,
   the others superseded by it.
5. **Not done here:** sharing between agents (ADR-51 §10: a knowledge note through the ticket, or project knowledge
   packs) waits for the owner's decision; Lead praise is not a signal yet (an approval carries no praise field).

## Evidence

- `agent.rs::validation_decay_and_consolidation`; `graph_memory.rs::lessons_need_a_cue_and_a_signal_and_are_rules_only_in_their_scope`;
  GM-1..GM-5, GM-7 still pass with signals and cues in their fixtures.
