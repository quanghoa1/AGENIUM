# ADR-111: Azure OpenAI, Amazon Bedrock and Cloudflare as presets; Vertex and Copilot not yet

Status: Accepted (2026-10-26). Extends [ADR-107](ADR-107-vendor-presets.md). Evidence:
`agent/llm/tests/adapters.rs::azure_openai_sends_the_api_key_header`,
`foundation/config/src/presets.rs::presets_are_well_formed`,
`orchestration/orchestrator/tests/registry.rs::providers_keep_their_keys_in_the_vault`, and the running app (2026-10-26:
"More vendors…" → Azure OpenAI fills the URL with `<resource>`, shows the hint, and saving it as is is refused). No call
with a real key: none of the three keys is available.

## Context

ADR-107 left out the vendors that "need another sign-in". Their documentation (2026-10) shows that three of them
now serve the OpenAI Chat Completions protocol with a plain key:

- **Azure OpenAI** v1 API: `https://<resource>.openai.azure.com/openai/v1`. No `api-version` is needed. The key goes in
  Azure's `api-key` header; the OpenAI SDKs send it as a Bearer token, which the v1 API also accepts. Models are the
  user's deployment names.
- **Amazon Bedrock**: `https://bedrock-mantle.<region>.api.aws/v1` (AWS's recommended endpoint) with a Bedrock API key
  as the Bearer token (`AWS_BEARER_TOKEN_BEDROCK`). The Chat Completions path serves Bedrock's open-weight models
  (gpt-oss, DeepSeek, Qwen, Mistral, ...), not Claude or Nova. Those need the SigV4-signed Converse API, which
  Agenium does not speak.
- **Cloudflare Workers AI**: `https://api.cloudflare.com/client/v4/accounts/<account-id>/ai/v1` with an API token.

**Google Vertex** has an OpenAI-compatible endpoint too, but it takes an OAuth access token that lasts about an hour.
Agenium would have to mint and refresh it (gcloud or a service account). **GitHub Copilot** needs its own device
sign-in and token exchange.

## Decision

1. Three presets: `azure-openai`, `bedrock`, `cloudflare`. Each is an OpenAI-compatible provider, and its URL keeps the
   part the user must fill in as a `<placeholder>`.
2. A base URL that still has a `<placeholder>` is refused when saved, by the app (`put_provider`) and by
   `agenium provider add` (which then asks for `--base-url` and repeats the preset's note). The app's form shows a
   hint under the URL while a placeholder is left.
3. An Azure host (`*.openai.azure.com`, `*.cognitiveservices.azure.com`, `*.services.ai.azure.com`) is its own
   flavor. It gets the `api-key` header besides the Bearer token, and OpenAI's reasoning-effort levels.
4. Vertex and Copilot stay out until Agenium can refresh a short-lived token. That would be a provider "key command",
   decided separately.
