// Capabilities the user manages (ADR-61): model providers (API key or local model; agent CLIs are detected), and the
// system's skill library and MCP servers that each agent switches on or off. The views live in capabilities/.
export { ProvidersView } from "./capabilities/ProvidersView";
export { LibraryView } from "./capabilities/LibraryView";
