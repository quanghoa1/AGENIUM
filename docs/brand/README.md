# Agenium brand

The mark is an "A" of three ribbons: two blades that meet at the top (blue, and cyan to green) and the ticket ribbon
across them (indigo to sky), with a white diamond at its centre. It sits on a light tile with a pale blue rim.

| File | Used for |
|---|---|
| `agenium-logo-banner-dark.svg` | the README banner |
| `agenium-logo-mark.svg` | the full mark |
| `agenium-logo-symbol.svg` | the symbol without its tile |
| `apps/desktop/ui/public/logo.svg` | the app's favicon and the source of the app icons: the same artwork as `agenium-logo-mark.svg` |
| `apps/desktop/ui/src/components/Logo.tsx` | the mark at 32–40 px (sidebar, About): the tile edge to edge with a thicker rim and no shadow, to read small |
| `apps/desktop/icons/` | window, taskbar and bundle icons (`tauri.conf.json` → `bundle.icon`), generated from `logo.svg` with the Tauri CLI (`tauri icon apps/desktop/ui/public/logo.svg`); the CLI also writes `android/` and `ios/`, which the desktop app does not use |
