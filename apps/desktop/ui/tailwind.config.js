/** Theme tokens from the AGENIUM-UX mockup, used through CSS variables (light and dark). */
export default {
  darkMode: "class",
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  theme: {
    extend: {
      fontFamily: {
        sans: ["Inter Variable", "Inter", "Segoe UI", "system-ui", "sans-serif"],
        mono: ["JetBrains Mono Variable", "JetBrains Mono", "Cascadia Mono", "Consolas", "monospace"],
      },
      colors: {
        background: "var(--bg-primary)",
        surface: "var(--bg-surface)",
        elevated: "var(--bg-elevated)",
        line: "var(--border-subtle)",
        "line-strong": "var(--border-strong)",
        fg: "var(--text-primary)",
        muted: "var(--text-secondary)",
        subtle: "var(--text-muted)",
        accent: { DEFAULT: "#3B82F6", hover: "#2563EB", cyan: "#06B6D4" },
      },
    },
  },
  plugins: [],
};
