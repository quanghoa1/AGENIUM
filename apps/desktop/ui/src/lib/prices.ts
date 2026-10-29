// Model prices (display only): one load per app refresh, shared by the model pickers, runtime badges and Usage.
import { useEffect, useState } from "react";
import { api } from "./api";
import type { Price, PriceView } from "./types";

let cached: Promise<PriceView> | null = null;
let cachedTick = -1;

/** Prices per configured provider; `tick` reloads them (pass `app.tick`). */
export function usePrices(tick: number): PriceView | null {
  const [v, setV] = useState<PriceView | null>(null);
  useEffect(() => {
    if (!cached || cachedTick !== tick) {
      cached = api.prices().catch(() => ({ providers: {} }));
      cachedTick = tick;
    }
    cached.then(setV);
  }, [tick]);
  return v;
}

export function priceOf(v: PriceView | null, provider: string, model: string): Price | undefined {
  return v?.providers[provider]?.[model];
}

/** The model's deprecation, when the catalogue marks it deprecated: the model to use instead (`null`: none known). */
export function deprecation(v: PriceView | null, provider: string, model: string): { instead: string | null } | undefined {
  const d = v?.deprecated?.[provider];
  return d && model in d ? { instead: d[model] } : undefined;
}

export const isFree = (p?: Price) => !!p && p.input === 0 && p.output === 0;

/** "$0.15 / $0.60" per million tokens in / out. */
export const priceLabel = (p: Price) => `$${p.input.toFixed(2)} / $${p.output.toFixed(2)}`;
