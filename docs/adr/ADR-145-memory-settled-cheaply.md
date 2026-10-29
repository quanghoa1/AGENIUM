# ADR-145: An agent's memory is settled by its vectors, the judge only where they cannot tell

Status: Accepted (2026-10-30). Owner: "3 thì có vẻ tốn token, có cách nào tốt hơn không?" then "okay thử đi, verify cho
kỹ". Instead of having a model reread the whole memory on a schedule (the "dreams" of other agents), which costs
hundreds of thousands of tokens a pass.

## Context

- Writing a fact already reconciles it with the memories retrieved next to it (decision 2: ADD / UPDATE / DELETE / NOOP
  by the extraction model), and consolidation folds copies of the same text (decision 35). What slips between them:
  a paraphrase written in another episode, a figure that changed while the old fact was not retrieved.
- Every fact has a vector from the local embedding model (bge-small-en-v1.5, ADR-47): comparing them costs nothing.
- Learning is gated (ADR-91, ADR-92): nothing new is made up here, and nothing depends on the judge.

## Decision

1. **The pass** (`agenium_memory::settle`): live facts of the same kind, scope, project and trust are compared pair by
   pair by cosine similarity, closest first:
   - from `SAME` (0.93) with the same figures (numbers, number words, negation) and the same names (hosts, paths,
     identifiers, capitalised words such as GitHub / GitLab or English / Vietnamese): the same fact — folded into the
     more confident one (then the one validated in more contexts, then the newer), its history kept, its validated
     contexts moved over (`AgentStore::fold`);
   - from `RELATED` (0.75): the judge, when `[judge] memory_settle` is on, says same (folded), contradicting (for the
     user) or both hold (kept); at most 20 questions a pass, each pair once (`fact_pair`, agent.db migration 7);
   - without the judge (or past its questions): nearly the same words with other figures, names or a negation
     ("port 8080" / "port 9090", which the vectors read only 0.89 alike) are a conflict for the user; the rest is left.
   A contradiction is never settled without the user: the Memory page lists them ("May contradict each other") and
   the user keeps one (the other is folded into it) or says both hold.
2. **When**: after an episode is learned, once the agent learned 20 facts since the last pass (`settled_upto`), and
   when the user tidies an agent's memory. The run's log says what the pass did.
3. **The judge use** `memory_settle` (on by default, fail-open, its own switch in the judge settings): two typed
   questions per pair (the same? contradicting?), about 410 input tokens.

## Measured (2026-10-30)

`quality/eval/examples/memory_settle_eval.rs` on 70 labelled pairs written as agent memories
(`agent/memory/tests/data/settle-pairs.json`): 20 that say the same, 20 that contradict (another value, version,
tool, place or rule), 20 on one subject that both hold, 10 unrelated. bge-small similarity: same 0.89–0.98 (median
0.93), contradicting 0.72–0.95 (0.90), both hold 0.60–0.88 (0.77), unrelated 0.46–0.59.

| | folded (of 20 same) | contradictions found (of 20) | both hold kept | wrong folds | judge |
|---|---|---|---|---|---|
| vectors and words only | 10 | 8 (+1 extra: iOS 16 / Android 12, for the user) | — | **0** | 0 tokens, 6 ms |
| with the judge (two runs, the same result) | 19 | 18 | 11 of 11 judged | **0** | 39 questions, 16,000 tokens |

- Without the name check, a contradiction read 0.943 alike with the same figures ("staging at
  staging.shop.example.com" / "at stage.shop.example.net") — 0.007 under a fold. With it, no wrong fold down to a
  threshold of 0.90; 0.93 was kept with that margin.
- `RELATED` 0.85 found 13 contradictions of 20 with the judge, 0.80 16, 0.75 18 (for 29, 38 and 42 questions).
- "Retries three times" / "retried 3 times" were a false conflict until number words counted as figures.

## Verified (2026-10-30)

- Tests: `settle::tests` (figures, number words, negations, names, words); `agent/memory/tests/settle.rs` (folds,
  conflicts without a judge, the user's decision, nothing judged twice, the judge's questions within their limit, an
  unsure judge, a dry run that changes nothing); runtime `the_memory_is_settled_once_enough_was_learned` (not before
  20 new facts, not twice, a manual pass asks nothing again).
- Desktop UI smoke: an agent imported with three memories of its dev server (two the same, one on port 9090) — Tidy
  folds the two and shows the other port as a possible contradiction ("Tidied: 1 folded, 1 to decide"); the user's
  "This one holds" keeps port 9090 and folds the other.

## Consequences

- A judge question costs about 410 tokens; a pass costs at most 20 of them, against hundreds of thousands to reread
  the memory. Without the judge the pass still folds near-identical facts and catches changed figures.
- The thresholds are measured on bge-small-en-v1.5; another embedding model needs them measured again (the eval takes
  `--same` and `--related`).
- Pairs of facts in different scopes, projects or trust are never compared; a contradiction across them stays.
