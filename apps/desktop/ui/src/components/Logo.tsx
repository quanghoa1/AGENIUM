// The Agenium mark at small sizes (sidebar, setup): the "A" of two rails from the Lead node to the workers, the ticket
// crossbar and the merge-queue arc, with strokes thickened to read at 32 px. The full artwork is `public/logo.svg`
// (favicon, app icons) and `docs/brand/`.
export function Logo({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 512 512" className={className} fill="none" aria-hidden="true">
      <rect width="512" height="512" rx="112" fill="#0B1120" />
      <path d="M 124 404 Q 256 446 388 404" stroke="#22D3EE" strokeWidth="10" strokeDasharray="16 14" strokeLinecap="round" fill="none" opacity="0.85" />
      <path d="M 256 104 L 120 404" stroke="#3B82F6" strokeWidth="44" strokeLinecap="round" />
      <path d="M 256 104 L 392 404" stroke="#06B6D4" strokeWidth="44" strokeLinecap="round" />
      <path
        d="M 136 238 H 376 A 14 14 0 0 1 390 252 V 266 A 18 18 0 0 0 390 302 V 316 A 14 14 0 0 1 376 330 H 136 A 14 14 0 0 1 122 316 V 302 A 18 18 0 0 0 122 266 V 252 A 14 14 0 0 1 136 238 Z"
        fill="#0F172A"
        stroke="#38BDF8"
        strokeWidth="12"
      />
      <circle cx="170" cy="284" r="18" fill="#10B981" />
      <rect x="210" y="264" width="136" height="16" rx="8" fill="#38BDF8" />
      <rect x="210" y="292" width="96" height="14" rx="7" fill="#94A3B8" />
      <circle cx="256" cy="104" r="46" fill="#0B1120" stroke="#38BDF8" strokeWidth="14" />
      <polygon points="256,74 286,104 256,134 226,104" fill="#38BDF8" />
      <circle cx="120" cy="404" r="34" fill="#0B1120" stroke="#3B82F6" strokeWidth="14" />
      <circle cx="120" cy="404" r="14" fill="#60A5FA" />
      <circle cx="392" cy="404" r="34" fill="#0B1120" stroke="#06B6D4" strokeWidth="14" />
      <circle cx="392" cy="404" r="14" fill="#22D3EE" />
    </svg>
  );
}
