// The usage page's charts, drawn in pixels (text keeps its size at any width): columns stacked by series over time,
// horizontal bars ranked by value, a legend and the table a chart can be read as instead.
import { useEffect, useRef, useState, type ReactNode } from "react";

export interface Series {
  key: string;
  label: string;
  color: string;
  values: number[];
}

/** An element's width, followed as it changes. */
function useWidth<T extends HTMLElement>() {
  const ref = useRef<T>(null);
  const [w, setW] = useState(0);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const ro = new ResizeObserver(([e]) => setW(e.contentRect.width));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  return [ref, w] as const;
}

/** A round top for a scale: 1, 2, 2.5 or 5 times a power of ten, at or above `v`. */
function niceMax(v: number): number {
  if (v <= 0) return 1;
  const p = 10 ** Math.floor(Math.log10(v));
  return [1, 2, 2.5, 5, 10].map((m) => m * p).find((x) => x >= v * 0.999) ?? 10 * p;
}

/** A column's top segment: a 4px rounded data end, square at the bottom. */
function topRounded(x: number, y: number, w: number, h: number): string {
  const r = Math.min(4, w / 2, h);
  return `M${x},${y + h}V${y + r}Q${x},${y} ${x + r},${y}H${x + w - r}Q${x + w},${y} ${x + w},${y + r}V${y + h}Z`;
}

/** Which series is which: a swatch of each one's colour beside its name (the name keeps the text colour). */
export function Legend({ series }: { series: { key: string; label: string; color: string }[] }) {
  if (series.length < 2) return null;
  return (
    <div className="mb-2 flex flex-wrap gap-x-4 gap-y-1 text-[11px] text-muted">
      {series.map((s) => (
        <span key={s.key} className="inline-flex items-center gap-1.5">
          <span className="h-2.5 w-2.5 rounded-sm" style={{ background: s.color }} />
          {s.label}
        </span>
      ))}
    </div>
  );
}

/** Columns over `periods`, one stacked segment per series with a 2px gap between segments. Hovering or focusing a
 *  column shows every series' value there; the axis carries what no label shows. */
export function StackedColumns({ label, periods, names, series, format, height = 220 }: { label: string; periods: string[]; names: string[]; series: Series[]; format: (n: number) => string; height?: number }) {
  const [ref, width] = useWidth<HTMLDivElement>();
  const [hover, setHover] = useState<number | null>(null);
  const totals = periods.map((_, i) => series.reduce((s, x) => s + (x.values[i] ?? 0), 0));
  const max = niceMax(Math.max(0, ...totals));
  const left = 56;
  const top = 8;
  const bottom = 22;
  const plotW = Math.max(0, width - left - 4);
  const plotH = height - top - bottom;
  const slot = plotW / Math.max(1, periods.length);
  const bw = Math.max(2, Math.min(24, slot * 0.7));
  const y = (v: number) => top + plotH - (v / max) * plotH;
  const every = Math.max(1, Math.ceil(periods.length / 10));
  const tip = hover === null ? null : Math.min(Math.max(0, left + slot * hover + slot / 2 - 100), Math.max(0, width - 200));
  return (
    <div ref={ref} className="relative" onMouseLeave={() => setHover(null)}>
      {width > 0 && (
        <svg width={width} height={height} role="img" aria-label={label}>
          {[0, max / 2, max].map((t) => (
            <g key={t}>
              <line x1={left} x2={width - 4} y1={y(t)} y2={y(t)} stroke="var(--border-subtle)" strokeWidth={1} />
              <text x={left - 8} y={y(t)} dy="0.32em" textAnchor="end" fontSize={10} fill="var(--text-muted)" style={{ fontVariantNumeric: "tabular-nums" }}>
                {format(t)}
              </text>
            </g>
          ))}
          {periods.map((p, i) => {
            const x = left + slot * i + (slot - bw) / 2;
            const shown = series.map((s) => s.values[i] ?? 0);
            const last = shown.reduce((l, v, si) => (v > 0 ? si : l), -1);
            let acc = 0;
            return (
              <g key={p}>
                {hover === i && <rect x={left + slot * i} y={top} width={slot} height={plotH} fill="var(--bg-elevated)" />}
                {series.map((s, si) => {
                  const v = shown[si];
                  if (v <= 0) return null;
                  const y0 = y(acc + v);
                  const h = y(acc) - y0;
                  acc += v;
                  if (si === last) return <path key={s.key} d={topRounded(x, y0, bw, h)} fill={s.color} />;
                  // A 2px surface gap above every segment that has another on top.
                  return h > 2.5 ? <rect key={s.key} x={x} y={y0 + 2} width={bw} height={h - 2} fill={s.color} /> : null;
                })}
                {(i % every === 0 || i === periods.length - 1) && (
                  <text x={left + slot * i + slot / 2} y={height - 6} textAnchor="middle" fontSize={10} fill="var(--text-muted)">
                    {names[i]}
                  </text>
                )}
                <rect
                  x={left + slot * i}
                  y={top}
                  width={slot}
                  height={plotH}
                  fill="transparent"
                  tabIndex={0}
                  aria-label={`${names[i]}: ${format(totals[i])}`}
                  onMouseEnter={() => setHover(i)}
                  onFocus={() => setHover(i)}
                  onBlur={() => setHover(null)}
                />
              </g>
            );
          })}
        </svg>
      )}
      {hover !== null && tip !== null && (
        <div className="pointer-events-none absolute top-0 z-10 w-[200px] rounded-lg border border-line-strong bg-surface p-2.5 text-[11px] shadow-lg" style={{ left: tip }}>
          <div className="mb-1 text-subtle">{names[hover]}</div>
          {series
            .filter((s) => (s.values[hover] ?? 0) > 0)
            .map((s) => (
              <div key={s.key} className="flex items-center gap-2">
                <span className="h-0.5 w-3 shrink-0 rounded" style={{ background: s.color }} />
                <span className="font-mono font-semibold text-fg">{format(s.values[hover] ?? 0)}</span>
                <span className="truncate text-muted">{s.label}</span>
              </div>
            ))}
          <div className="mt-1 border-t border-line pt-1 font-mono text-fg">Σ {format(totals[hover])}</div>
        </div>
      )}
    </div>
  );
}

/** The same numbers as a table: a row per period, a column per series and the total. */
export function SeriesTable({ names, series, format, total }: { names: string[]; series: Series[]; format: (n: number) => string; total: string }) {
  return (
    <div className="max-h-[220px] overflow-auto">
      <table className="w-full text-[11px]">
        <thead className="sticky top-0 bg-surface">
          <tr className="label border-b border-line text-left">
            <th className="py-1 pr-3" />
            {series.map((s) => (
              <th key={s.key} className="px-2 text-right">
                {s.label}
              </th>
            ))}
            <th className="px-2 text-right">{total}</th>
          </tr>
        </thead>
        <tbody className="divide-y divide-line font-mono" style={{ fontVariantNumeric: "tabular-nums" }}>
          {names.map((n, i) => (
            <tr key={n}>
              <td className="py-1 pr-3 text-subtle">{n}</td>
              {series.map((s) => (
                <td key={s.key} className="px-2 text-right">
                  {format(s.values[i] ?? 0)}
                </td>
              ))}
              <td className="px-2 text-right">{format(series.reduce((t, s) => t + (s.values[i] ?? 0), 0))}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

export interface Bar {
  key: string;
  label: ReactNode;
  sub?: string;
  value: number;
  color: string;
}

/** Horizontal bars ranked by value, the value at each bar's tip. */
export function BarList({ rows, format }: { rows: Bar[]; format: (n: number) => string }) {
  const max = Math.max(0, ...rows.map((r) => r.value));
  return (
    <ul className="space-y-2.5">
      {rows.map((r) => (
        <li key={r.key} className="grid grid-cols-[minmax(0,15rem)_1fr] items-center gap-3 text-xs">
          <div className="min-w-0">
            <div className="truncate font-medium">{r.label}</div>
            {r.sub && <div className="truncate text-[11px] text-subtle">{r.sub}</div>}
          </div>
          <div className="flex items-center gap-2" title={`${format(r.value)}`}>
            <div className="h-3.5 shrink-0 rounded-r" style={{ width: `calc((100% - 5.5rem) * ${max ? r.value / max : 0})`, minWidth: r.value > 0 ? 2 : 0, background: r.color }} />
            <span className="shrink-0 font-mono text-muted" style={{ fontVariantNumeric: "tabular-nums" }}>
              {format(r.value)}
            </span>
          </div>
        </li>
      ))}
    </ul>
  );
}
