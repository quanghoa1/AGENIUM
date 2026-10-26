# ADR-114: Each model's own effort levels, by the vendor's names

Status: Accepted (2026-10-26). Replaces the effort mapping table of decision 13. Evidence:
- `agent/llm/src/effort.rs` (the per-model table and its tests);
- `agent/llm/tests/adapters.rs::each_model_gets_its_own_effort_level_on_the_wire`,
  `claude_models_carry_their_effort_levels`, `openrouter_models_carry_their_effort_levels`;
- `agent/cli-backend/src/{codex,agy,claude,grok}.rs` tests;
- the agent form in the desktop app;
- golden tasks through three CLIs at effort low (below).

## Context

Agenium had one ladder of four levels (low, medium, high, max) and mapped it onto every vendor. The models do not
share one ladder:

| Model | Levels | Default |
|---|---|---|
| Claude 5.x | low, medium, high, xhigh, max | Haiku and Opus 5.5 medium, Sonnet 5.5 high |
| Claude Sonnet / Opus 4.6 | low, medium, high, max | high |
| Claude Opus 4.5 | low, medium, high | high |
| DeepSeek V4 | low, high, max | high |
| Gemini 3 Pro | low, high | high |
| Gemini 3.8 Flash | low, medium, high | medium |
| GPT-5.5 Pro | medium, high, xhigh | high |
| GPT-5.5 | none, low, medium, high, xhigh | medium |
| Grok 4.7 | low, medium, high, xhigh | high |
| Codex gpt-6.1-sol | low, medium, high, xhigh, max, ultra | low |

On top of that, the app offered xhigh, which the core could not read, and CLI agents never received the effort.

Some vendors report the levels, and they are believed first:
- Anthropic's Models API, `capabilities.effort` (read on 2026-10-26).
- OpenRouter's model list, `reasoning.supported_efforts` and `default_effort`.
- Codex's `model/list`, `supportedReasoningEfforts` and `defaultReasoningEffort`.
- Antigravity's model names (`gemini-3.8-flash-low`).

OpenAI, Gemini, xAI and DeepSeek do not report them. Their documentation does, per model (studied 2026-10-26, with
sources).

## Decision

1. **An agent's effort is one of its model's own levels**, by the vendor's name: `low`, `high`, `max`, Codex's
   `ultra`, OpenAI's `none`. The type is a level name. Well-known names have an order (none, minimal, low, medium,
   high, xhigh, max, ultra), used only to resolve a level for another model.
2. **Each model's levels come from its vendor when it reports them, else from a per-model table**
   (`agenium_llm::effort`). The table holds Claude (as the Models API reports it), OpenAI, Gemini, xAI and DeepSeek
   (from their documentation). Model lists carry each model's levels and default: the API providers' lists, and each
   CLI's (Codex `model/list`, Antigravity's variants grouped per model, Claude Code and Grok from the table).
3. **The agent form offers exactly those levels.** A newly chosen model starts at its default. A model without effort
   control shows "no effort". A model whose levels are unknown shows "model default": Agenium then sends no effort,
   and the vendor's default applies.
4. **The request carries the model's own level.** A level the model does not have is resolved to its nearest level;
   on a tie the higher one wins, so a level is never quietly lowered. For example, medium on DeepSeek becomes high,
   as DeepSeek itself maps it. The wire is the vendor's own:

   | Route | Where the level goes |
   |---|---|
   | Anthropic | `output_config.effort` |
   | Anthropic models that think within a budget | `budget_tokens`, from Agenium's budgets |
   | OpenAI / Azure, DeepSeek (also behind OpenCode), xAI | `reasoning_effort` |
   | Responses API | `reasoning.effort` |
   | OpenRouter | `reasoning.effort`, its own levels as listed |
   | Gemini 3+ | `thinkingLevel` |
   | Gemini 2.5 | `thinkingBudget` |
   | Local servers | nothing |

5. **CLI agents get it in the CLI's words**:
   - Claude Code: `--effort`.
   - Codex: `-c model_reasoning_effort="<level>"`.
   - Antigravity: the model variant, so `gemini-3.8-flash` at low runs `gemini-3.8-flash-low`.
   - Grok: `--reasoning-effort`.
   - OpenCode's levels are named per model in its own config, so it keeps its setting.

## Result

- In the desktop app's agent form (Agents → Create agent), the effort list showed each model's own levels, with its
  default selected:
  - Codex gpt-6.1-sol: low (default) … ultra.
  - Codex gpt-6-luna: low … max, default medium.
  - Claude Code sonnet: default high.
  - Antigravity gemini-3.8-flash: low, medium, high; gpt-oss-120b: medium only.
  - DeepSeek V4.1 Flash through OpenCode Go: low, high (default), max.
- The golden task `hello-file` at effort low passed through three CLIs, each receiving the level
  (`quality/eval/results/effort/`):
  - Claude Code `haiku`: 91.7 % of input cached.
  - Codex `gpt-6-luna`: 62.0 %.
  - Antigravity `gemini-3.8-flash`, run as `gemini-3.8-flash-low`.
