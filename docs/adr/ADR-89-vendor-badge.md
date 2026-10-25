# ADR-89: Show the AI vendor behind each agent's model

Status: Accepted (2026-10-25; owner: "show the vendor's logo on the agents by the model they use — agent A on Claude
shows Claude; pick sensible places").

## Decision

Show a small **vendor badge** wherever an agent's model is visible, so you can see at a glance which AI runs an agent
(Claude, OpenAI, Gemini, DeepSeek, OpenCode, …).

- `lib/vendors.ts` `vendorOf(provider, model)` detects the vendor from the model string first (it names the family —
  `claude`/`haiku`/`sonnet`, `gemini`, `gpt`/`codex`, `deepseek`, `qwen`, `llama`, …) and then the provider id / CLI name,
  returning a name, a brand colour and a one/two-letter initial. Unknown providers get a generic badge from their initial.
- `components/VendorBadge.tsx` renders a monogram (brand colour + initial) with the vendor name as its tooltip and
  accessible label. It is **our own mark, not the trademarked logo** — a recognisable, honest badge; a project that wants
  the official logos can drop them in later.
- Placed in the **Live inspector** next to the model, and drawn as a small chip beside each agent's name in the
  **pixel office** (a canvas, so the badge is drawn, not a DOM element).

## Consequences

- New `apps/desktop/ui/src/lib/vendors.ts` and `components/VendorBadge.tsx`; `VendorBadge` used in `Live.tsx`;
  `vendorChip` drawn per agent in `PixelOffice.tsx`.
- No backend change: detection is from the agent's existing `model.provider` / `model.model`.
- Not done: shipping official vendor logos (trademark/asset concern); a badge in every secondary list — added where the
  model is the point (inspector, office).
