import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri serves the dev UI on a fixed port and builds into dist/ (tauri.conf.json).
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: { port: 5173, strictPort: true },
  // motion makes one ~570 kB chunk; a desktop app loads it from disk.
  build: { target: "es2022", outDir: "dist", emptyOutDir: true, chunkSizeWarningLimit: 800 },
});
