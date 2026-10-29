# Agenium's `tool_search` against Claude Code's own tool search (ADR-139), 2026-10-29

`quality/eval/examples/tool_search_vs_cli.py`: Claude Code 2.1.312 with Claude Haiku 5.5, the 123 tools of
`agent/mcp/data/many-tools.json`, the 58 requests of `many-tools-queries.json` (worded unlike the descriptions). Each run
is told the tools are simulated and to call the one tool that does the request, once. A run is right when the tool it
calls is the one labelled. Input tokens count every turn of the run (Claude Code's own system prompt included).

| Arm | What the model has | Round | Right tool | Tool calls / run | Input tokens / run |
|---|---|---|---|---|---|
| native | Claude Code's `ToolSearch`; the 123 tools deferred, their names listed | 1 | 56/58 | 2.0 | 20,213 |
| native | (the same, again) | 2 | 57/58 | 2.0 | 20,191 |
| upfront | all 123 tools listed (`ENABLE_TOOL_SEARCH=false`) | 1 | 57/58 | 1.0 | 37,993 |
| agenium | `tool_search` + `call_tool`, all 123 names in the system prompt, the judge on, arguments as raw JSON schema | 1 | 57/58 | 1.9 | 17,341 |
| agenium-nojev | the same without the judge | 1 | 55/58 | 1.9 | 17,276 |
| agenium | arguments one line each | 2 | 55/58 | 1.9 | 17,025 |
| agenium-nonames | the same without the names in the system prompt | 2 | 57/58 | 2.0 | 14,355 |
| **agenium** | **as shipped: arguments one line each, past 30 tools each server summed up by groups (`github 20, slack 11, …`)** | 3 | **57/58** | 2.2 | **15,796** |
| agenium-nojev | as shipped, without the judge | 3 | 57/58 | 2.4 | 17,155 |

Reading it:

- Every arm missed the same few requests, each with two fitting tools ("show me the stack trace of that crash":
  `sentry_get_issue_details` also returns it; "logs of the container that crashed before the restart": Docker's or
  Kubernetes'); between two rounds an arm moves by about two. Choice is even: Agenium's search is as right as Claude
  Code's own and as listing every tool.
- Agenium as shipped takes about 22 % fewer input tokens per run than Claude Code's search and 58 % fewer than listing
  every tool. Listing the 123 names cost about 2,700 tokens a run and changed no choice; the groups keep what the
  servers are about at a fraction of it.
- The judge saves searches: without it the model searched again more often (2.4 tool calls a run against 2.2) for the
  same choices, and read more.
- Times are not comparable: each Agenium run started its search server (an app root, the embedding model) in a debug
  build.

## Names that say nothing (round 4)

The same with the tools named `tool_000` … `tool_122` (`--opaque`; descriptions kept), so a tool is found only by what
its description says:

| Arm | Right tool | Tool calls / run | Input tokens / run |
|---|---|---|---|
| native (Claude Code's `ToolSearch`) | 55/58 | 2.2 | 21,345 |
| agenium (the judge on) | 57/58 | 2.0 | 14,476 |
| agenium-nojev | 57/58 | 2.1 | 15,396 |

Claude Code's search gave up once ("the error is fixed, mark it done in sentry": two searches, no call) and paged the
wrong team once; Agenium's arms missed only the stack-trace request every arm misses.

## DeepSeek over the API

`quality/eval/examples/tool_search_api.rs`: DeepSeek v4.1 flash (OpenCode Go), the same 58 requests and wording, a loop
of up to six turns over Agenium's own tools (`tool_search` + `call_tool` as a run gets them, or all 123 listed):

| Arm | Plain names | Opaque names | Tool calls / run | Input tokens / run |
|---|---|---|---|---|
| upfront: all 123 listed | 55/58 | 55/58 | 1.0 | ≈ 12,000 |
| bm25: `tool_search`, words only (no embedding model) | **57/58** | 56/58 | 2.1–2.3 | ≈ 1,900–2,400 |
| bge: words + meaning | 56/58 | **57/58** | 2.0–2.4 | ≈ 1,800–2,500 |
| jev: words + meaning + the judge | 57/58 | 56/58 | 2.0–2.4 | ≈ 1,700–2,200 |

Listing every tool was the least right (the model took a Slack message for "ask alice to review my PR", Docker's logs for
Kubernetes') and took five to six times the tokens.

## Why words alone do about as well

The ranking alone (`examples/tool_search_eval.rs`, right tool first / in the 5 a search returns):

| Ranking | Plain names | Opaque names |
|---|---|---|
| words (BM25) | 30 / 47 | 27 / 44 |
| + meaning (bge) | 36 / 52 | 33 / 49 |
| + the judge | 55 / 57 | 55 / 55 |

The judge ranks far better, yet the runs end even: the model reads the 5 tools a search returns with their
descriptions and picks one, so the right tool has only to be among them; when it is not, the model words the search
again (2 to 2.4 tool calls a run). The model is the last reranker, which is how Claude Code's words-only `ToolSearch`
keeps up. Names matter little: the descriptions carry the words (three places lost at ranking, none in the runs). The
judge saves a search now and then and some tokens; it is worth more with a weaker model, thin descriptions or requests
sharing no word with any description.

Files: `20261029-round4-opaque.json`, `20261029-deepseek-plain.json`, `20261029-deepseek-opaque.json`,
`20261029-round1-all-arms.json`, `20261029-round2-names-compact-args.json`, `20261029-round3-tuned.json` (one
entry per run: the request, the tool called, the calls made, turns, tokens, cost).
