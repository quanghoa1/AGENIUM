# Accessibility checklist (WCAG basics)

## Keyboard
- Every control reachable and operable with Tab / Enter / Space / Escape.
- Visible focus indicator; logical focus order; no focus trap.

## Names and labels
- Inputs have a programmatic label (`<label>`, `aria-label`, or `aria-labelledby`).
- Icon-only buttons have an accessible name.
- Images have `alt`; decorative images have empty `alt=""`.

## Roles and structure
- Native semantic elements first; ARIA roles only when needed and correct.
- Headings in order (no skipped levels); landmarks for main regions.

## Colour and contrast
- Text contrast >= 4.5:1 (normal) or 3:1 (large, >= 24px or 19px bold).
- Meaning never carried by colour alone (add text, icon or pattern).

## Motion
- Respect `prefers-reduced-motion` for non-essential animation.
- No content flashing more than 3 times per second.
