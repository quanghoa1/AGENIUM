# Agenium desktop app

Tauri 2 shell around the orchestrator (ADR-60). Its own Cargo workspace; the UI is the Vite app in `ui/`.

## Run it

```powershell
npm --prefix apps/desktop/ui install
npm --prefix apps/desktop/ui run build          # UI into ui/dist
cd apps/desktop
cargo run --features tauri/custom-protocol      # the app, with ui/dist embedded
```

The app opens the Agenium root the CLI uses (`AGENIUM_HOME`, else the root chosen at setup, else `~/.agenium`); an
empty root shows the setup screen. Set `AGENIUM_HOME` to try it on another root.

UI work without the Rust side: `npm --prefix apps/desktop/ui run dev` and open http://localhost:5173 — outside Tauri
the UI answers from demo data (`ui/src/lib/demo.ts`).

## Installers

```sh
cd apps/desktop
./ui/node_modules/.bin/tauri build --config tauri.bundle.json --bundles nsis,msi        # Windows
./ui/node_modules/.bin/tauri build --config tauri.bundle.json --bundles deb,appimage    # Linux (Ubuntu)
CI=true ./ui/node_modules/.bin/tauri build --config tauri.bundle.json --bundles app,dmg # macOS (ad-hoc signed)
```

`tauri.bundle.json` turns bundling on and ships the embedding model: its before-build step builds the UI and downloads
the model into `resources/models` (both commands run from `ui/`). Output: `target/release/bundle/<kind>/`. On Linux
the build needs `libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev libssl-dev patchelf` and
Node.js. Release 0.1 was checked on a clean Windows 11 VM (silent install `/S`, UI smoke test on the installed app,
silent uninstall) and on an Ubuntu 25.10 VM (`.deb` and AppImage).

## Motion

`motion` for the animations, a subtle level, everything inside `MotionConfig reducedMotion="user"`. The rules were
learned from the animate-ui skill (https://github.com/imskyleen/animate-ui); no code or package from it is used. Our
tokens (springs, timings) and shared pieces live in `ui/src/components/motion.tsx`.
