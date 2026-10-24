import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri serves the dev UI on a fixed port and builds into dist/ (tauri.conf.json).
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: { port: 5173, strictPort: true },
  build: { target: "es2022", outDir: "dist", emptyOutDir: true },
});
