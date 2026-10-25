// A small monogram badge for the AI vendor behind an agent's model (ADR-89): the vendor's brand colour and initial,
// labelled with the vendor name. Our own mark, not the trademarked logo.
import { vendorOf } from "../lib/vendors";

export function VendorBadge({
  provider,
  model,
  size = 16,
  className = "",
  withName = false,
}: {
  provider: string;
  model: string;
  size?: number;
  className?: string;
  withName?: boolean;
}) {
  const v = vendorOf(provider, model);
  const badge = (
    <span
      className={`inline-flex shrink-0 items-center justify-center rounded font-semibold text-white ${className}`}
      style={{
        backgroundColor: v.color,
        width: size,
        height: size,
        fontSize: Math.round(size * (v.initial.length > 1 ? 0.42 : 0.56)),
        lineHeight: 1,
      }}
      title={v.name}
      aria-label={v.name}
      role="img"
    >
      {v.initial}
    </span>
  );
  if (!withName) return badge;
  return (
    <span className="inline-flex items-center gap-1.5">
      {badge}
      <span className="text-muted">{v.name}</span>
    </span>
  );
}
