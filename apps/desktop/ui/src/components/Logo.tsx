// The Agenium mark at small sizes (sidebar, About): an "A" of three ribbons, two blades and the ticket across them,
// around a white diamond, on its tile edge to edge with a thicker rim and no shadow to read at 32 px. The full artwork
// is `public/logo.svg` (favicon, app icons) and `docs/brand/`.
export function Logo({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 512 512" className={className} fill="none" aria-hidden="true">
      <defs>
        <linearGradient id="ag-bg" x1="0" y1="0" x2="512" y2="512" gradientUnits="userSpaceOnUse">
          <stop offset="0%" stopColor="#F8FAFC" />
          <stop offset="100%" stopColor="#EFF6FF" />
        </linearGradient>
        <linearGradient id="ag-left" x1="256" y1="72" x2="104" y2="416" gradientUnits="userSpaceOnUse">
          <stop offset="0%" stopColor="#2563EB" />
          <stop offset="100%" stopColor="#38BDF8" />
        </linearGradient>
        <linearGradient id="ag-right" x1="256" y1="140" x2="408" y2="416" gradientUnits="userSpaceOnUse">
          <stop offset="0%" stopColor="#06B6D4" />
          <stop offset="100%" stopColor="#10B981" />
        </linearGradient>
        <linearGradient id="ag-bar" x1="144" y1="296" x2="368" y2="296" gradientUnits="userSpaceOnUse">
          <stop offset="0%" stopColor="#4F46E5" />
          <stop offset="100%" stopColor="#0EA5E9" />
        </linearGradient>
      </defs>
      <rect width="512" height="512" rx="116" fill="url(#ag-bg)" />
      <rect x="8" y="8" width="496" height="496" rx="112" stroke="#BFDBFE" strokeWidth="12" />
      <path d="M 238 86 C 248 64 278 64 288 86 L 202 280 L 146 406 C 136 428 104 422 96 400 L 84 368 C 78 352 82 334 90 316 L 238 86 Z" fill="url(#ag-left)" />
      <path d="M 284 162 L 420 368 C 430 384 422 412 400 416 L 362 416 C 344 416 330 404 322 388 L 236 234 L 284 162 Z" fill="url(#ag-right)" />
      <path d="M 164 268 H 348 C 364 268 376 280 376 296 C 376 312 364 324 348 324 H 164 C 148 324 136 312 136 296 C 136 280 148 268 164 268 Z" fill="url(#ag-bar)" opacity="0.94" />
      <polygon points="256,274 278,296 256,318 234,296" fill="#FFFFFF" />
    </svg>
  );
}
