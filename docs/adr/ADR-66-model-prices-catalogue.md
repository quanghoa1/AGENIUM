# ADR-66: Model prices from a public catalogue; FREE models marked

Status: Accepted (2026-10-25, owner: "can we get model prices to compute the cost, I still see $0"; "tag free
models FREE"; "some free models only work from the opencode CLI, look into it")

## Decision

1. **Prices come from the models.dev catalogue** (`https://models.dev/api.json`, the public list OpenCode uses),
   trimmed to the costs and cached in `<root>/price-catalogue.json`. `pricing.toml` and the built-in Anthropic table
   still win. A configured provider is matched to a catalogue provider by its id, else by the host and path of its
   base URL (`https://opencode.ai/zen/go/v1` -> `opencode-go`). A model with no listed cache price is billed as
   plain input for cached tokens. Cost stays display only (decision 3k).
2. **Refresh.** The desktop app downloads the catalogue when it is missing or a week old (checked every six hours),
   and the Usage page has "Update prices". `[usage] price_catalogue_url = ""` turns it off.
3. **Earlier runs are priced again.** A finished API run with tokens and a cost of 0 (its model had no price) gets
   its cost when the catalogue arrives. Vendor CLI runs stay at 0 (subscriptions).
4. **FREE tag.** A model whose input and output price is 0 shows FREE in the model picker (the list also shows each
   model's price in / out per million tokens), on the agent's runtime badge and in Usage.
5. **OpenCode free models need OpenCode.** OpenCode's docs say "Free-tier chat models require OpenCode": the
   `*-free` models answer 403 to other clients, and Agenium does not pose as the OpenCode client. The only honest
   route is to drive the real `opencode` CLI as a vendor CLI backend, which the owner dropped on 2026-10-24; it
   needs the owner's decision.

## Consequences

- `agenium_llm::pricing::{Catalogue, Prices, fetch_catalogue}`; `agenium_orchestrator::prices` (`price_view`,
  `usage_cost`, `save_price_catalogue`, `price_unpriced_runs`, `refresh_price_catalogue`); runs priced through the
  app at the end of a run.
- Desktop IPC `prices`, `refresh_prices`; UI `lib/prices.ts`, `FreeTag`.
- Tests: `pricing::catalogue_prices_api_models`, `team.rs::the_price_catalogue_prices_earlier_runs`.
