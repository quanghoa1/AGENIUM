# ADR-145: An agent's memory is settled by its vectors and words, the judge only where they cannot tell

Status: Accepted (2026-10-30). Owner: "3 thì có vẻ tốn token, có cách nào tốt hơn không?", then "okay thử đi, verify cho
kỹ", then "test các bài giới hạn đi, chúng ta nên cố định model". Instead of having a model reread the whole memory on a
schedule (the "dreams" of other agents), which costs hundreds of thousands of tokens a pass.

## Context

- Writing a fact already reconciles it with the memories retrieved next to it (decision 2: ADD / UPDATE / DELETE /
  NOOP by the extraction model), and consolidation folds copies of the same text (decision 35). What slips between them:
  a paraphrase written in another episode, a value that changed while the old fact was not retrieved.
- Every fact has a vector from the local embedding model; Agenium fixes it to bge-small-en-v1.5 (ADR-47). Comparing
  vectors costs nothing.
- Learning is gated (ADR-91, ADR-92): nothing new is made up here, and nothing depends on the judge.

## Decision

1. **The pass** (`agenium_memory::settle`): live facts of the same kind, scope, project and trust are compared pair by
   pair (cosine, off the async threads); pairs from `RELATED` (0.75) are considered:
   - **folded without the judge** only when they use the same words (figures, negations and `do` aside), the same
     figures and the same names, and read at least `SAME` (0.93) alike — the same sentence reordered or repunctuated;
   - **a conflict without the judge** when they use the same words with other figures or a negation on one side ("port
     8080" / "port 9090", which the vectors read only 0.89 alike);
   - **parallel facts are left alone**: two facts that differ by one word on each side (or one word more on one side)
     where the rest of the sentence is filled by three or more such words are about as many subjects ("the orders /
     shipping / billing service keeps its logs for N days"), not one fact twice or contradicted;
   - **the rest goes to the judge** when `[judge] memory_settle` is on (fail-open): same (folded into the more confident
     one, its history kept, its validated contexts moved), contradicting (for the user) or both hold (kept). At most 20
     questions a pass, each pair once (`fact_pair`, agent.db migration 7). Pairs that swap one word for another
     ("pnpm" / "npm", "Fridays" / "Mondays") are asked first, then the closest.
   A contradiction is never settled without the user: the Memory page lists them ("May contradict each other"); the
   user keeps one (the other is folded into it) or says both hold.
2. **When**: the free part (vectors and words) after every episode learned; the judge once the agent learned 10 facts
   since it last read (`settled_upto`; ADR-146: four tickets learn about ten), and when the user tidies an agent's memory. Only with bge-small-en-v1.5 (`SETTLE_MODEL`): the bands were measured on
   it, and the hashing stand-in used while the model is not installed reads other similarities — the pass waits for
   the model (exact-text consolidation runs as before).
3. **The judge use** `memory_settle` (on by default, its own switch in the judge settings): two typed questions per pair
   (the same? contradicting?), about 410 input tokens.

## Measured (2026-10-30)

Two evals with the real bge-small-en-v1.5 and the real judge (Jev 1.14):

- `memory_settle_eval`: 70 labelled pairs written as agent memories (`agent/memory/tests/data/settle-pairs.json`: 20
  that say the same, 20 that contradict, 20 on one subject that both hold, 10 unrelated), each pair in a project of its
  own. Similarity: same 0.89–0.98, contradicting 0.72–0.95, both hold 0.60–0.88, unrelated 0.46–0.59.
- `memory_settle_scale`: the same pairs among 1,000 true facts of one project about 40 services (each with its port,
  language, database, owner…: many facts read alike and differ by a name or a figure, and all hold), every fact in one
  group as in an agent's memory. A label-knowing judge (`--oracle`) measures the order the judge reads pairs in without
  spending its tokens.

| | Folded (of 20 same) | Contradictions found (of 20) | Wrong folds | Both-hold pairs flagged | Judge |
|---|---|---|---|---|---|
| 70 pairs, no judge | 0 | 4 | 0 | 0 | — |
| 70 pairs, the judge | 17 | 18 | **0** | 0 | 45 questions, 18,500 tokens |
| crowded, no judge | 0 | 4 | **0** | **0** | — |
| crowded, the judge, 5 passes (100 questions) | 11 | 11 | **0** | **0** | 41,000 tokens a run |

What the crowded memory showed (and changed):

- The first rules folded any two facts at least 0.93 alike with the same figures and names. In the crowded memory they
  made **1,863 wrong decisions** ("the orders service keeps its logs for 14 days" folded into "the shipping service
  …", channels of two services flagged as contradicting). Folding now needs the same words; looser word tests (three
  in four words, one sentence holding all the other's) still folded "the support service is called by the gateway"
  into "the gateway service …".
- Of 18,000 related pairs, 7,400 are parallel facts; 10,300 are left for the judge, so a pass of 20 questions reaches
  only the first of them. With a perfect judge: by similarity alone the first 20 questions found 2 of the 20
  contradictions, by the share of rare words 0, swaps first **7** (and 10 of the 20 paraphrases, as before). The rest
  are found over later passes (a pass every 20 facts learned): 11 within 2,000 questions.
- The real judge flagged "the X service is called by the gateway" against "the gateway service is called by the
  gateway" as contradicting (4 of 100 questions) until parallel facts counted a subject that is also a word of the
  sentence; after that, none.
- Not reached: contradictions read less than 0.75 alike ("retries three times" / "never retries", 0.745; "run the
  tests in parallel" / "do not run them in parallel: they share one database", 0.72).
- A pass over 1,040 facts takes about 8 s in a debug build (the comparisons on a blocking thread).

## Verified (2026-10-30)

- Tests: `settle::tests` (figures, number words, negations, names, words); `agent/memory/tests/settle.rs` (folds only of
  the same words, conflicts without a judge, the user's decision, nothing judged twice, the judge's questions within
  their limit, an unsure judge, a dry run, parallel facts not asked and swaps first); orchestrator
  `the_judge_reads_every_ten_new_facts`, runtime `the_memory_is_not_settled_without_the_embedding_model`.
- Desktop UI smoke: an agent imported with three memories of its dev server (two the same reordered, one on port
  9090): Tidy folds the two and shows the other port as a possible contradiction; "This one holds" keeps port 9090.

## Consequences

- Without the judge the pass is conservative: it folds only reworded copies and flags changed figures; paraphrases and
  other contradictions wait for the judge.
- In a crowded memory the judge reaches pairs gradually, 20 a pass; a pass costs at most about 8,000 tokens.
- Pairs of facts in different scopes, projects or trust are never compared.
