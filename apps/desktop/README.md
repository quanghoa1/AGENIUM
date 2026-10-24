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

## Motion

Animations follow the animate-ui skill (https://github.com/imskyleen/animate-ui, `skills/animate-ui`): values from
`@animate-ui/react`'s motion tokens, `motion` for the animations, subtle level, everything inside
`ReducedMotionConfig`. Shared pieces live in `ui/src/components/motion.tsx`.
