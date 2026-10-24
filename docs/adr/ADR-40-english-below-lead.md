# ADR-40: Language policy: any language for the user, English from Lead downward

- **Status:** Accepted (owner decision, 10:49)
- **Date:** 2026-10-23 (Asia/Saigon)
- **Context / decision / consequences:**

- User <-> Lead/Solo may use any language (setting `app.user_language`, free-form BCP-47, default `en`; collected in the setup wizard).
- Everything from Lead downward is **English only**: tickets, plans, handoffs, comments, memory, skills, inter-agent messages, tool-call text.
- Lead/Solo translate user input to English for artifacts and reply in the user's language.
- Consequences: memory/embeddings are English-only (English embedding models are fine); the prompt rule lives in `agenium_core::language` (single source for all prompts); UI language (`app.language`: en|vi) is independent of `user_language`.
- Not enforced mechanically in M0 (no agents yet): enforcement/eval for 'English below Lead' belongs to M1 (golden task check).

## Amendment (2026-10-25, owner: "no language setting for the Lead; the Lead or Solo agent works it out")

- `app.user_language` and the wizard's "language you talk to the Lead in" are removed. The Lead and Solo agents
  answer in the language the user writes in (`agenium_core::language::USER_FACING`); everything they write for other
  agents (tickets, plans, comments, handoffs) stays English (`ENGLISH_BELOW_LEAD`). Old `config.toml` files with
  `user_language` still load (the key is ignored and dropped on the next save).
- The UI language stays `app.language`, English or Vietnamese for now.
