# ADR-94: Memory measured on labelled sets — resolution keys, name vectors, a relevance-ordered graph channel

Status: Accepted (2026-10-26). Evidence: `docs/memory-eval.md` (2 152 labelled mentions, 112 labelled questions, the
memory-reuse scenario on DeepSeek n = 5); calibrates the initial bands of ADR-51 §4.

## Context

ADR-51 left entity resolution with "initial values, not calibrated yet" and planned a labelled resolution set and a
>= 100-question multi-hop set (§13). Building and running them (and re-running memory reuse on a real model) found
defects in code that every agent's memory goes through:

1. `canonical()` dropped `+` and `#`: **C#, C++ and C were one key**, so a mention of C++ resolved to C# — and the
   upsert would have merged them even if resolution had not.
2. "String-similar" (one canonical name contains the other, or token Jaccard >= 0.5) let **siblings** auto-merge at
   cosine >= 0.90: `Pixel 8 Pro phone` → Pixel 8, `Nighthawk R7000P router` → R7000, `gRPC-Web` → gRPC. Measured
   precision 0.987, 7 false merges in 2 152.
3. A **zero vector** (the hashing embedder on `C#`) made sqlite-vec return a NULL distance, and one such stored row broke
   every vector search of the table (facts or entities) with a type error — all recall for that agent.
4. Entity vectors embedded "name. description", compared with a bare mention: the description hid the name (recall 0.60).
5. The ambiguous-band verdicts are positional; DeepSeek answered 9 verdicts for 10 pairs in 2–4 of 57 groups, which
   shifts every later verdict onto the wrong pair.
6. The graph channel was ordered by graph score, so facts of the queried entity itself (hop 0) filled it and buried the
   two-hop answer: with the graph on, two-hop MRR fell from 0.83 to 0.45.
7. An extraction answer cut off by the 4 000-token budget lost the whole episode; and extracted "facts" carried episode
   details (agent ids, commit hashes, byte counts).

## Decision

- **Keys keep `+`/`#` after a word** (`c#`, `c++`, `ds920+`). Databases written before are re-keyed once at open
  (`agent_meta.canonical = 2`); scope values normalised before cannot be recovered and stay.
- **Same name = only noise differs** (`resolve::same_name`): equal once spaces are ignored, or every word one name has
  and the other lacks is generic (`the`, `server`, `library`, …) or known about the stored entity (its description,
  type, identity scope — `TP-Link … router` for a TP-Link router). A qualifier (`Pro`, `Galera`, `R7000P`) sends the pair
  to the model, never to an auto-merge.
- **Entity vectors from the name alone** (`resolve::entity_text(name)`, `reindex_entities` likewise). Existing entity
  vectors are rebuilt once (`agent_meta.entity_vector_text = name`). Descriptions still reach the model in the
  ambiguous band. Measured: recall 0.75 → 0.92, precision 1.000 → 0.996–0.998 with DeepSeek deciding the band (target
  >= 0.98). The bands themselves (auto >= 0.90, ask >= 0.80) stay: on name vectors 0.95+ is 93 % same, 0.80–0.90 is
  ~56 %, below 0.80 ~38 % — the band is where the model is needed.
- **A verdict miscount trusts none** (all "different", recorded in the learn report).
- **Zero or non-finite vectors are never indexed**; a query without a direction skips the vector channel; NULL distances
  from older databases are skipped. The hashing embedder keeps symbols when a text has no two-letter word.
- **The graph channel proposes, relevance orders**: candidates reachable from the query's entities (up to 4 × the pool)
  are ranked by cosine similarity to the query, graph score breaking ties (`AgentStore::similarity_to`). Result with bge:
  recall@8 0.99, MRR 0.92 over 112 questions (no graph: 0.88 / 0.62), zero scope leaks (no graph: 38 of 40 traps).
- **Extraction**: a deterministic filter refuses facts that are ids/ULIDs, commit hashes or exact byte sizes
  (`pipeline::ephemeral`), and `EXTRACT_RULES` names them; the budget is 8 000 tokens; a cut-off answer keeps the facts
  it finished (`pipeline::complete_items`).
- **The labelled sets are part of the repository** (`agent/memory/examples/data/`, generated deterministically by
  `make_datasets.py`, labels by construction) with their evaluators, so a change to resolution or retrieval can be
  re-measured in minutes.

## Consequences

- `foundation/store`: `graph::canonical`, `agent::{usable, rekey_names, similarity_to}`, `knn`/`entity_candidates`
  skip unusable vectors and NULL distances, `reindex`/`reindex_entities` never index zero vectors, entity vectors from
  names. Tests: `zero_vectors_are_not_indexed_and_do_not_break_search`,
  `symbol_names_get_their_own_keys_and_old_keys_are_rewritten`.
- `agent/memory`: `resolve::{same_name, context_words, entity_text(name)}`, `refresh_vectors` rebuilds pre-ADR-94
  entity vectors, `pipeline::{ephemeral, same_line, SAME_RULES (pub), complete_items}`, the miscount rule,
  `retrieve::recall` relevance ordering, `HashEmbedder` symbol fallback. Tests: `resolve::tests::bands_and_hard_rules`
  (qualifiers, context words), `pipeline::tests::{episode_details_are_not_memories,
  a_cut_off_extraction_keeps_its_complete_facts}`.
- `orchestration/orchestrator/src/runtime/learn.rs`: extraction budget 8 000.
- Evaluators: `agent/memory/examples/{entity_resolution, graph_qa}.rs`, `quality/eval/examples/entity_ask.rs`;
  `quality/eval/src/memory_reuse.rs` redesigned (rule covers `reports/`, measured ticket writes there, the report enters
  the plan only when it is ticketed, files and ticket counts recorded).
- Cost: name vectors double the share of mentions put to the model (13 % → 26 %); they go in the one batched call per
  learn that already existed. 55 of 97 one-letter typos still become a new entity (a duplicate, not a wrong merge).
- Not done: the same reuse scenario on the CLI models; the adjacency cache; typo-tolerant matching.
