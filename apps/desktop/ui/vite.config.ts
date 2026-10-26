import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri serves the dev UI on a fixed port and builds into dist/ (tauri.conf.json).
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: { port: 5173, strictPort: true },
  // A desktop app loads its chunks from disk: the main one (~740 kB, motion included) and, on demand, Monaco (~2.7 MB,
  // the side-by-side diff) and xterm.
  build: { target: "es2022", outDir: "dist", emptyOutDir: true, chunkSizeWarningLimit: 3000 },
});
