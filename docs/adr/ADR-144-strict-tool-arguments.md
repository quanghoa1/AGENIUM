# ADR-144: The forms that move work are decoded strictly by Claude's and OpenAI's APIs

Status: Accepted (2026-10-29). Owner: "về 2 thì tôi nghĩ chúng ta nên theo hướng 2 vendor lớn là openai và claude"
(structured outputs from `platform.claude.com/docs`, and OpenAI's equivalent).

## Context

- Claude's and OpenAI's APIs can decode a tool call against its schema (`strict: true`): the arguments always parse.
  Other providers ignore or refuse the field.
- Agenium reads a model's arguments leniently (ADR-105, ADR-142): repaired shapes, aliases, words for numbers. That
  keeps every provider working; strict decoding removes the guessing at the source where it is offered.
- Claude takes a subset of JSON Schema (every object `additionalProperties: false`; no numeric, length or pattern
  constraints) and per request at most 20 strict tools, 24 optional parameters and 16 parameters with alternatives,
  refusing the request past them ("Schema is too complex for compilation"). OpenAI wants every property in
  `required`, an optional one taking `null`.

## Decision

1. **Which tools**: `submit_report`, `review_ticket`, `create_ticket`, `create_plan`, `propose_spec`,
   `assign_ticket`, `submit_plan_draft` (`ToolSpec::strict`). Together 23 optional parameters, 6 with alternatives, 7
   tools: within Claude's limits whatever the role (a test holds it).
2. **Fitting** (`agenium_llm::strict`): constraints the decoders do not take are dropped (the tool checks them as
   before); `oneOf` becomes `anyOf`; every object gets `additionalProperties: false`; an object with keys of its own
   choosing (the report's template `extension`) is asked as a JSON object written as text, which the argument repair
   reads back as the object. Free-form arguments or `$ref` have no strict form: the tool goes plain.
3. **Where**: Anthropic's own API (not a relay of its wire) — the tool sent `strict: true` without input streamed ahead
   of it, admitted in order while within the limits; OpenAI's own API and Azure's — `strict: true` with every property
   required and the optional ones nullable. The argument repair reads an optional argument sent as `null` (or an
   empty text for one that is not a text) as not given. Every other provider gets the tools as before.
4. **A refusal costs one request**: a 400 about the schema from either API sends the request again without strict
   tools, and every later one to that endpoint and model in the process.

## Verified (2026-10-29)

- Tests: `strict::tests` (fitting and its cost, JSON text for free-form objects, Claude's admission within its
  limits, OpenAI's required and nullable); `the_strict_protocol_tools_fit_claude_s_limits_together`;
  `strict_tools_go_strict_only_to_claude_and_openai` (relays, DeepSeek, OpenCode and local servers unchanged);
  `a_refused_strict_schema_is_sent_again_plain` (both APIs: refused once, the request again plain, and the next one);
  `coerce::tests::an_optional_argument_sent_as_null_is_not_given`.
- Not yet run against the live APIs: no OpenAI API key, and the Anthropic test key is not in this session. A refusal
  falls back as above.

## Consequences

- Strict schemas are compiled by the API on first use (cached a day): the first request of a new schema is slower.
- Agenium's own work tools and MCP tools stay plain; a tool joins the strict set only if the set still fits.
