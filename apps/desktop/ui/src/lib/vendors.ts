// The AI vendor behind an agent's model, for a small badge next to the agent (ADR-89). We draw our own monogram in the
// vendor's brand colour with the vendor's name as the label — a recognisable mark, not the trademarked logo. Detection
// reads the model string first (it names the family), then the provider id (API id or CLI name).

export interface Vendor {
  key: string;
  /** Shown as the badge's accessible name and tooltip. */
  name: string;
  /** Brand-ish colour for the monogram. */
  color: string;
  /** One or two letters for the monogram. */
  initial: string;
}

const VENDORS: Record<string, Vendor> = {
  anthropic: { key: "anthropic", name: "Claude (Anthropic)", color: "#D97757", initial: "C" },
  openai: { key: "openai", name: "OpenAI", color: "#10A37F", initial: "O" },
  google: { key: "google", name: "Gemini (Google)", color: "#1A73E8", initial: "G" },
  deepseek: { key: "deepseek", name: "DeepSeek", color: "#4D6BFE", initial: "D" },
  opencode: { key: "opencode", name: "OpenCode", color: "#6366F1", initial: "OC" },
  openrouter: { key: "openrouter", name: "OpenRouter", color: "#64748B", initial: "OR" },
  mistral: { key: "mistral", name: "Mistral", color: "#FA520F", initial: "M" },
  meta: { key: "meta", name: "Llama (Meta)", color: "#0866FF", initial: "L" },
  qwen: { key: "qwen", name: "Qwen", color: "#7C3AED", initial: "Q" },
  local: { key: "local", name: "Local model", color: "#0EA5E9", initial: "◆" },
};

/** The vendor behind `provider`/`model`, or a generic badge from the provider's initial. */
export function vendorOf(provider: string, model: string): Vendor {
  const m = (model || "").toLowerCase();
  const p = (provider || "").toLowerCase();
  const hay = `${p} ${m}`;
  const has = (...keys: string[]) => keys.some((k) => hay.includes(k));

  if (has("claude", "anthropic", "haiku", "sonnet", "opus", "fable")) return VENDORS.anthropic;
  if (has("deepseek")) return VENDORS.deepseek;
  if (has("gemini", "agy", "google")) return VENDORS.google;
  if (has("gpt", "codex", "openai", "o1", "o3", "o4", "luna")) return VENDORS.openai;
  if (has("qwen")) return VENDORS.qwen;
  if (has("mistral", "mixtral", "codestral")) return VENDORS.mistral;
  if (has("llama", "meta-llama")) return VENDORS.meta;
  if (has("opencode")) return VENDORS.opencode;
  if (has("openrouter")) return VENDORS.openrouter;
  if (has("local", "ollama", "bge", "gguf")) return VENDORS.local;

  const initial = (p.replace(/[^a-z0-9]/g, "")[0] || "?").toUpperCase();
  return { key: p || "unknown", name: provider || "Unknown vendor", color: "#64748B", initial };
}
