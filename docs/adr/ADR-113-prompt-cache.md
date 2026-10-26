# ADR-113: Prompt caching across vendors: stable prefix first, one route per agent, notes on the cached prefix

Status: Accepted (2026-10-26). Extends decision 13 (provider caching) and ADR-100/103 (compaction). Evidence:
`agent/loop/tests/loop.rs::a_full_context_is_summarised_and_the_run_goes_on`,
`notes_fall_back_to_a_smaller_request_when_the_prefix_does_not_fit`;
`agent/llm/tests/adapters.rs::tools_listed_but_not_callable_in_every_wire`,
`openrouter_marks_cache_breakpoints_for_anthropic_models`, `cache_routes_reach_openrouter_and_xai`; the golden tasks on
DeepSeek before and after (below).

## Context

Every vendor bills prompt-cache hits at a fraction of the input price: about a tenth for DeepSeek, Anthropic and
OpenAI. An agent run sends the same growing conversation step after step, so a high hit rate cuts the user's input cost
by up to about 90 %.

Measured before this ADR: 89.3 % of all input tokens on the OpenCode Go test root (48 real runs), but only 49–79 % in
short runs, and 77.7 % over the five golden tasks. Every run's first request missed. Three causes were in Agenium:

- **The prefix changed before its stable part ended.** The capability manifest names the ticket's worktree and came
  before the project's guidance, so only the rules and the agent's SOUL/ROLE were shared between runs.
- **The cache route was per ticket.** `prompt_cache_key` and OpenCode's `x-opencode-session` were `agent:ticket`, so
  each ticket could start on a server that held nothing.
- **Notes requests rebuilt the prompt.** Compaction (at 85 % of the window, about 100 k tokens) and checkpoint notes
  were sent without tools and with results cut. That is a different prefix, so the whole request was a miss.

The rules a harness must follow to keep a provider's prefix cache warm are the same everywhere (studied from DeepSeek's
cache guide and the pi `pi-deepseek-cache` / `pi-reasonix` notes; our own code):
- system, tools and history must be byte-identical up to the new part;
- the history only grows;
- stable content goes first and per-run content last;
- the same route is kept;
- a summary call replays the original prefix and appends only its instruction.

## Decision

1. **Stable first, and nothing per-run in the system.** The work order's system blocks are, in order:
   - the rules;
   - SOUL/ROLE;
   - the project guidance (AGENTS.md);
   - the capability manifest, holding only what is the same across the agent's runs.

   The run's own lines (worktree, team state) open the run's newest user message. Tools stay sorted by name.

   A probe (`quality/eval/examples/cache_probe.rs`, DeepSeek via OpenCode Go) showed why. On the wire the tools come
   right after the system. One block appended to the system kept 61 % of the prompt cached instead of 99 %, and one
   tool replaced kept 76 %. So a per-run line in the system costs the whole tool section on every run. Another session
   key kept 99 %: DeepSeek's disk cache is not tied to a route.
2. **One route per agent.** The cache key is the agent's id. It goes to OpenAI and the Responses API as
   `prompt_cache_key`, to OpenCode as `x-opencode-session`, to OpenRouter as `x-session-id` (its sticky provider
   routing), and to xAI as `x-grok-conv-id`.
3. **Notes on the cached prefix.** Compaction and checkpoint notes send the run's exact prefix: the same system, the
   tools listed but not callable, and the same history, with the instruction appended. Each wire states "not
   callable" in its own words: `tool_choice: none`; Anthropic `{"type": "none"}`; Gemini
   `functionCallingConfig.mode = NONE`. When the prefix plus room for the notes (4 096 tokens) would not fit, or the
   provider says it does not, the smaller request of ADR-103 is sent instead.
4. **Explicit-cache models behind OpenRouter.** Anthropic and Gemini models cache only at `cache_control` breakpoints:
   one after each of the first two stable system blocks and one on the last message (1 h TTL for long-running
   Anthropic agents). DeepSeek, OpenAI, xAI and Gemini direct cache automatically. Anthropic direct already had its
   breakpoints (decision 13).
5. **A cache break is visible.** A step served from the cache for less than half of the previous request's prompt is
   logged in the run's log (`control: cache_miss`). The log starts once the provider has shown it caches at all. A
   compaction or a clearing resets it, because those change the prefix on purpose.

## Per vendor

| Vendor | How it caches | What Agenium does |
|---|---|---|
| DeepSeek (direct, via OpenCode Go or OpenRouter) | automatic, on disk, exact prefix from token 0 | stable prefix, one route, notes on the prefix |
| OpenAI / Azure OpenAI | automatic from 1 024 tokens; routed by `prompt_cache_key` | key per agent |
| Anthropic (direct) | explicit breakpoints (max 4), 5 min or 1 h TTL | tools, two stable system blocks, last message |
| Anthropic / Gemini via OpenRouter | explicit `cache_control` | breakpoints as above; `x-session-id` |
| Gemini (direct) | implicit on 2.5+ (explicit `cachedContents` not used) | stable prefix |
| xAI Grok | automatic per server; `x-grok-conv-id` keeps the server | header with the agent key |
| Vendor CLIs (Claude Code, Codex, Grok, …) | the CLI caches its own prompt | Agenium does not touch it |

Not done: OpenAI's extended retention (`prompt_cache_retention` / `prompt_cache_options`). The field and its allowed
values differ by model generation, and a wrong value is a 400. Gemini's explicit `cachedContents` is not used either.

## Result (DeepSeek v4.1 Flash via OpenCode Go, `quality/eval/results/prompt-cache/`)

| Scenario | Before (5385ffe) | After | Uncached input per run | per step |
|---|---|---|---|---|
| Golden tasks (5 tasks, 10 runs, all PASS) | 77.7 % cached | **90.2 %** | 11 274 → 5 719 (−49 %) | 2 301 → 1 003 (−56 %) |
| Learning chain (one agent, 4 tickets, 10 runs, PASS) | 80.3 % | **90.3 %** | 10 987 → 6 307 (−43 %) | 2 073 → 1 051 (−49 %) |

In the chain, tickets 2 and 3 of the same agent were served 96–97 % from the cache. Cache reads cost about a tenth of
uncached input, so the input bill of these runs drops by roughly a third. The remaining misses are mostly each step's
new tool results, which no cache can serve.
