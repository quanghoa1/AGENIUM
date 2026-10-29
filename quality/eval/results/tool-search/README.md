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

Files: `20261029-round1-all-arms.json`, `20261029-round2-names-compact-args.json`, `20261029-round3-tuned.json` (one
entry per run: the request, the tool called, the calls made, turns, tokens, cost).
