// How a kind of node looks on the Map page: an icon and a colour. Code is one family (files, symbols), the rest by
// kind; an unknown kind gets a plain circle and the last colour.
import { AppWindow, Boxes, Braces, Building2, Circle, Database, FileCode, Globe, Lightbulb, Package, Plug, Route, Server, Settings2, SquareTerminal, type LucideIcon } from "lucide-react";

const ICONS: Record<string, LucideIcon> = {
  file: FileCode,
  symbol: Braces,
  module: Boxes,
  domain: Globe,
  vendor: Building2,
  company: Building2,
  product: AppWindow,
  app: AppWindow,
  cli: SquareTerminal,
  api: Plug,
  endpoint: Route,
  service: Server,
  server: Server,
  database: Database,
  library: Package,
  config: Settings2,
  concept: Lightbulb,
};

export const iconOf = (kind: string): LucideIcon => ICONS[kind] ?? Circle;

/** A colour series per kind, in the order kinds first appear (code first). */
export function colourOf(kinds: string[]) {
  const order = [...new Set(["file", "symbol", ...kinds])];
  return (kind: string) => {
    const i = order.indexOf(kind);
    return i >= 0 && i < 7 ? `var(--series-${i + 1})` : "var(--series-other)";
  };
}
