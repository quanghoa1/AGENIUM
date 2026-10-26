# ADR-107: Known model vendors are presets — pick a name, paste a key

Status: Accepted (2026-10-26). Evidence: `foundation/config/src/presets.rs` (tests), the registry test
`every_vendor_preset_builds_an_adapter`, an endpoint probe of every preset (below). Source: the owner noted that pi
supports many vendors; pi's provider list (`packages/ai/src/providers`) was read for the endpoints.

## Context

Agenium's adapters already speak the APIs most vendors offer — Anthropic Messages, Gemini, OpenAI Chat Completions
(and OpenAI Responses through OpenCode) — but the user had to know each vendor's base URL; the app offered eight
buttons and the CLI none.

## Decision

1. **One list of presets** in `agenium-config` (`presets::PRESETS`): id, name, adapter kind, base URL, the vendor's
   usual key variable, a note. 29 entries: Anthropic, OpenAI, Google Gemini, DeepSeek, OpenRouter, OpenCode Zen/Go,
   xAI, Mistral, Groq, Cerebras, Together, Fireworks, Hugging Face router, NVIDIA NIM, Baseten, Moonshot (global/China),
   Kimi Coding, Z.ai GLM (API, coding plan, China coding plan), Qwen (Alibaba Model Studio), MiniMax (global/China,
   through its Anthropic-compatible endpoint), Xiaomi MiMo, Vercel AI Gateway, Ollama and LM Studio on this machine.
2. **CLI:** `agenium provider presets` lists them (no root needed); `agenium provider add <id>` takes a preset by its
   id, or `--preset <name>` with another id; kind and base URL come from it, and the vendor's usual key variable is
   read when it is set (never printed).
3. **App:** the provider form lists every preset ("More vendors…") from the same list, and the base URL can be set for
   every kind (MiniMax needs it on the Anthropic adapter).

## Verification

Every preset builds an adapter. Without a key, `GET <base>/models` answered 401/403 (or 200 for public model lists) for
all 25 OpenAI-compatible endpoints, so each path is right; MiniMax's `/anthropic/v1/messages` answered 411 (the route
exists); `api.minimaxi.com` (China) was not reachable from this machine. No vendor except OpenCode Go has been called
with a key: the owner has no keys for them.

Not here yet (another sign-in or request signing): Azure OpenAI, Amazon Bedrock, Google Vertex, GitHub Copilot,
Cloudflare Workers AI (account id in the URL), Mistral's own Conversations API.
