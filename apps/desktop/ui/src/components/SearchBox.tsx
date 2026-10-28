import { Search } from "lucide-react";

/** A page's own search: what the page lists, narrowed as you type. */
export function SearchBox({ value, onChange, placeholder }: { value: string; onChange: (v: string) => void; placeholder: string }) {
  return (
    <label className="flex w-64 items-center gap-2 rounded-lg border border-line bg-background px-3 py-[7px] text-xs text-subtle focus-within:border-line-strong">
      <Search className="h-3.5 w-3.5 shrink-0" />
      <input type="search" className="min-w-0 flex-1 bg-transparent text-fg outline-none placeholder:text-subtle" value={value} placeholder={placeholder} onChange={(e) => onChange(e.target.value)} />
    </label>
  );
}

/** Whether any of `fields` holds the search `q` (ignoring case); an empty search holds everywhere. */
export function found(q: string, ...fields: (string | null | undefined)[]): boolean {
  const s = q.trim().toLowerCase();
  return !s || fields.some((f) => f?.toLowerCase().includes(s));
}
