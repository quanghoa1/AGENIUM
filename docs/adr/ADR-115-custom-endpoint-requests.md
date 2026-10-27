# ADR-115: What a user may add to a custom endpoint's requests, and what stays Agenium's

Status: Accepted (2026-10-27). Extends ADR-31 (hand-set model capabilities) and ADR-114 (effort levels). Evidence:
- `agent/llm/src/custom.rs` (the rules and their unit tests);
- `agent/llm/tests/adapters.rs::a_custom_endpoint_sends_the_users_headers_and_effort`;
- `orchestration/orchestrator/tests/registry.rs::custom_endpoint_headers_and_model_settings_are_stored`,
  `extra_request_fields_follow_the_rules`, `a_custom_endpoint_receives_what_the_user_set` (saved through the
  registry, built by the runtime, received by a local server);
- the provider form in the desktop app (section "Advanced").

## Context

Gateways and self-hosted endpoints speak the OpenAI chat API with differences: a tenant or routing header, a token
of their own, sampling settings, an effort level under another name (`reasoning.effort`, `thinking.effort`), models
Agenium knows nothing of. Without a way to say so, such an endpoint cannot be used well, and the effort the user picks
for an agent is never sent (an unknown model gets no effort field).

## Decision

For an OpenAI-compatible provider the user may set, all optional:

1. **Extra headers.** A secret one is stored in the vault (`provider/<id>/header/<name>`) and the configuration holds
   a `${secret:...}` placeholder, as MCP servers do; its value never returns to the interface.
2. **Per-model settings** (`[[providers.model_caps]]`): tools, images, context window, and the model's effort levels
   with a default. The agent form offers those levels; the chosen one is sent.
3. **Extra body fields** (`[[providers.body]]`): a dotted path, a value (JSON when it parses, else text) and
   optionally the one model it is for. Fields for every model apply first, the model's own after (so they win).
4. **The effort field**: where the effort level goes instead of `reasoning_effort`.

Nothing set means Agenium's own request; anything set is sent as given. The interface suggests common fields; it
never adds one by itself.

### What stays Agenium's (checked when saved, and again when each request is built)

The file can be edited by hand, so the request builder applies the same rules as the save and leaves out what breaks
them:
- **Body fields Agenium sets** cannot be named: `model`, `messages`, `input`, `instructions`, `prompt`, `tools`,
  `tool_choice`, `functions`, `function_call`, `stream`, `stream_options`, `n`, `response_format`, the token limits
  and `prompt_cache_key`. Replacing them would change what the agent sends or how its answer is read (a forced
  `tool_choice`, a non-streamed reply, an injected message).
- **Names** are 1 to 4 parts of letters, digits, `_` and `-`; at most 32 fields; a value at most 2 KB.
- **No placeholder in a body value** (`${secret:...}`, `${cred:...}`): a secret is only ever sent in a secret
  header, so a field cannot carry a vault secret to a place that might echo it; no control characters.
- **Headers the HTTP layer owns** cannot be set (`Host`, `Content-Length`, `Content-Type`, `Transfer-Encoding`,
  `Connection`, `Proxy-*` ...), and a header value is one line (no CR, LF or NUL): no request smuggling or header
  injection.

Only the user sets these, through the app or the configuration file; agents cannot reach `config.toml` (it is
outside every worktree) and no protocol tool writes provider settings.

## Consequences

- `foundation/config`: `ProviderConfig.headers`, `.body`, `.effort_field`; `ModelCaps.efforts`, `.default_effort`.
- `agent/llm`: `custom` module; `OpenAiCompat::customized`; `ProviderSpec.custom`.
- `orchestration/orchestrator`: `ProviderInput` (headers, model_caps, body, effort_field), the checks in
  `put_provider`, the runtime resolving secret headers and building the provider's customisation.
- Desktop: the "Advanced" section of the provider form; the model list includes the models the user named, with
  their levels, even when the endpoint cannot list models.
- Not done: the same for Anthropic and Gemini endpoints (their wire formats differ; no request so far).
