# Memory measured: entity resolution, multi-hop recall, reuse on a real model (ADR-94)

Three questions about agent memory that the store benchmarks (`docs/reference/STORE-SCALE.md`) cannot answer, each
measured on the production code paths, 2026-10-25/26 on Windows 11 (10 cores). Every finding below that changed code is
in [ADR-94](adr/ADR-94-memory-measured.md).

| Question | Set | Evaluator |
|---|---|---|
| Does a mention resolve to the right entity? | 115 stored entities, **2 152 labelled mentions** | `agent/memory/examples/entity_resolution.rs`, model share: `quality/eval/examples/entity_ask.rs` |
| Does the graph channel find what text alone cannot place? | 214 facts in 36 episodes, **112 labelled questions** | `agent/memory/examples/graph_qa.rs` |
| Does memory make the agent better on the next ticket? | two-ticket scenario, DeepSeek, n = 5 per condition | `quality/eval/src/memory_reuse.rs` (`--memory-reuse on/off`) |

Both sets are synthetic and labelled by construction (`agent/memory/examples/data/make_datasets.py`, deterministic):
no model wrote a label. Raw outputs: `docs/reference/bench-raw/windows-{entity-resolution-*,entity-ask-deepseek,graph-qa*}.txt`,
`quality/eval/results/memory-reuse/`.

## 1. Entity resolution — 2 152 mentions

**Set.** 115 entities as memory holds them (software, languages, OS releases, vendors, products with a vendor identity
scope, internal services, errors, concepts), each with a description and aliases. Mentions, by kind:
- *same entity* (874): exact, lowercase, uppercase, separator (`Archer-C7`), joined (`ArcherC7`), generic suffix
  (`Redis cache`), article (`the Redis`), vendor prefix (`TP-Link Archer C7`), one transposed letter, the stored aliases;
- *different entity* (1 278): sibling products and spin-offs (`Pixel 8 Pro`, `MariaDB Galera`, `Webpacker`, `C++` next to
  `C#`), siblings with a suffix or a typo, other versions (`Ubuntu 24.04`), the same name under another vendor scope,
  the same name as another type, and outsiders that are not stored at all (`CockroachDB`, `Valkey setup`).

Every mention runs the pipeline's path: embed the mention, `AgentStore::entity_candidates` (exact key, alias, FTS,
vector), `resolve::decide` (same / ask the model / new).

**Found, as the code was** (bge-small-en-v1.5): auto-merge precision 0.987 with **7 false merges** — `C++` and `C` merged
into `C#` (all three were keyed "c"), and `Pixel 8 Pro phone`, `UniFi Dream Machine Pro gateway`, `Nighthawk R7000P
router`, `gRPC-Web RPC framework`, `RabbitMQ Management Plugin message broker` merged into their siblings (a containing
name counted as "string-similar"). With the hashing embedder the run crashed: `C#` has no word of two letters, its
vector was all zeros, sqlite-vec returned a NULL distance, and one such row breaks every vector search of the table.

**After ADR-94:**

| Embedder, entity vector | auto-merge precision | auto-merge recall | sent to the model |
|---|---|---|---|
| hashing (no model installed) | **1.000** (509/509) | 0.58 | 5.3 % |
| bge, "name. description" (before) | **1.000** (525/525) | 0.60 | 13.2 % |
| bge, name alone (**now**) | **1.000** (689/689) | **0.79** | 26.1 % |

Per kind (bge, name alone): every alias, exact, case and separator mention merges; no sibling, other version,
conflicting scope, other type or outsider is ever merged by the rules. What the rules do not settle goes to the model:
197 of 345 siblings, 54 suffix and 42 typo mentions, 26 of 112 outsiders. 55 of 97 typos and 8 suffix mentions become a
new entity (a duplicate, not a wrong merge).

**The model's share** (the ambiguous band put to deepseek-v4.1-flash with the pipeline's own question):

| | model said "same" for same pairs | model said "same" for different pairs | resolution as a whole: precision | recall |
|---|---|---|---|---|
| name + description vectors | 134/135 | 0/150 | 1.000 | 0.754 |
| **name vectors (now)**, run A | 114/119 | 2/443 | **0.998** | **0.919** |
| name vectors, run B | 112/119 | 3/443 | 0.996 | 0.916 |

Name vectors raise recall from 0.75 to 0.92 for 0.2–0.4 points of precision (ADR-51 target: >= 0.98). The model's
mistakes: "Go kit" = "Go", "Extreme Networks" = "Juniper Networks", "Vualt Agent" = "Vue". It also answered **9 verdicts
for 10 pairs** in 2 and 4 of 57 groups: the pipeline applied verdicts by position, so one skipped pair shifted every later
verdict onto the wrong pair. A miscount now trusts none of them.

## 2. Multi-hop recall — 112 questions

**World.** 12 services, each using 3 of 24 components (`billing-api uses Redis 7.2 for its session cache`); every
component has one known issue with its fix (a lesson) and three pieces of trivia; services have four facts each
(deploy day, latency budget, log retention, on-call rotation); six teams own two services each; four network vendors
with two products each (identity scope `vendor`) and five symptoms whose fix differs per vendor (lessons scoped to the
vendor). Learned through the real `pipeline::learn` (a stub model returns the ideal extraction), so resolution, links
and edges are the production ones; the same facts learned without entities and relations give the no-graph store.

**Questions.** single-hop (the component is named, 24); **two-hop** (only the service is named: "billing-api keeps
rebalancing its consumers…" → the Kafka 3.6 lesson, 36); two-hop through a team ("a service owned by the payments
team…", 12); **scope traps** (a product is named; the same symptom's fix for the other three vendors must not appear, 40).

**Results** (recall@8 / MRR; scope leaks = questions with a forbidden fact in the top 8):

| bge-small-en-v1.5 | single-hop | two-hop | via team | scope trap | all | leaks |
|---|---|---|---|---|---|---|
| no graph | 1.00 / 1.00 | 0.92 / 0.83 | 0.92 / 0.81 | 0.75 / 0.16 | 0.88 / 0.62 | **38** |
| graph, as it was | 1.00 / 1.00 | 0.86 / **0.45** | 0.92 / 0.60 | 1.00 / 0.63 | 0.95 / 0.65 | 0 |
| **graph, relevance-ordered (now)** | 1.00 / 1.00 | **0.97 / 0.84** | **1.00 / 0.82** | **1.00 / 0.97** | **0.99 / 0.92** | **0** |

| hashing embedder | single-hop | two-hop | via team | scope trap | all | leaks |
|---|---|---|---|---|---|---|
| no graph | 1.00 / 0.91 | 0.81 / 0.62 | 0.83 / 0.70 | 0.40 / 0.09 | 0.71 / 0.50 | 23 |
| graph, as it was | 1.00 / 0.94 | 0.67 / 0.29 | 0.83 / 0.30 | 0.95 / 0.48 | 0.86 / 0.50 | 0 |
| **graph, relevance-ordered (now)** | 1.00 / 0.87 | 0.81 / 0.59 | 0.83 / 0.61 | 0.93 / 0.77 | 0.89 / 0.71 | **0** |

- **Scope is where the graph matters most.** Without it, a question about the Nighthawk R7000 does not know the router
  is a Netgear, so the TP-Link, Cisco and Ubiquiti fixes for the same symptom rank with it: 38 of 40 traps leaked. The
  named product's identity scope makes them conflict and they are filtered: 0 leaks.
- **The graph as it was hurt two-hop questions.** Its channel was ordered by graph score, so the facts of the entity the
  question names (hop 0: billing-api's deploy day, its latency budget) filled it, and fused with their text matches they
  pushed the two-hop answer down (MRR 0.83 → 0.45). Ordered by relevance to the query, with the graph as the candidate
  generator, the graph helps or is neutral everywhere (bge).
- The one bge miss left: "hangs when publishing messages" vs a lesson that says "blocks every publisher".

## 3. Memory reuse on a real model — DeepSeek, n = 5

**Scenario.** A pre-merge check enforces a rule the agent cannot know upfront: every team document under
`deliverables/` or `reports/` starts with `Owner: docs-team`. Ticket 1 (`deliverables/hello.txt`) meets it the hard way
(merge fails, the message states the rule, the agent fixes it; closing the ticket runs the memory pipeline). Ticket 2
(`reports/weekly.txt`) is measured, memory on or off. The first design measured `deliverables/bye.txt`, and both
conditions passed by imitating the file next to it; in the second, the Lead ticketed the weekly report itself after
ticket 1 (the plan already held it), so the measured ticket was its third and a `reports/` file was already on main.
Now the report enters the plan only when the measured ticket is created.

| Ticket 2 | first merge passed | steps (median, range) | tokens (median / mean) | runs |
|---|---|---|---|---|
| **memory on** | **5/5** | 9 (9–10) | 96 k / 97 k | 2 each |
| memory off | 3/5 | 11 (8–22) | 112 k / 143 k | 2, 2, 2, 4, 4 |

- Memory makes the outcome **predictable**: always right the first time, ~97 k tokens. Without it the agent either
  happens to copy the owner line from `hello.txt` (79–89 k tokens, cheaper than with memory, which adds the recalled
  facts to the prompt) or fails the merge and pays twice (202–235 k).
- After ticket 1, memory held 5–11 items, 1–3 of them stating the owner-line rule (as a fact, a lesson or both), in
  every run; recall showed 3–8 items to ticket 2.
- Extraction quality: the deterministic filter keeps ids, commit hashes and byte counts out. Still stored: environment
  facts of limited reuse ("the shell is PowerShell", "worktrees live under …") and the same rule several times as fact
  and lesson — consolidation merges near-duplicates later (ADR-72).
- One extraction answer (DeepSeek) was cut off by the 4 000-token budget and the whole ticket taught nothing; the budget
  is 8 000 now and a cut-off answer keeps the facts it finished.
- One run of the first batch was lost to the machine running out of disk and memory (`git worktree add`: out of memory;
  the page file could not grow on a full system drive); it was replaced.

Not run yet: the same scenario on the CLI models (Claude Haiku/Sonnet/Opus, gpt-6-luna, gpt-6.1-sol).
