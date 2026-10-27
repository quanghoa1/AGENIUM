/** The first sentence or two of a role's Mission section, for a short preview (the full text is on the role's page). */
export function missionOf(prompt: string): string {
  const m = prompt.match(/##\s*Mission\s*\n+([\s\S]*?)(\n##|$)/);
  return (m ? m[1] : prompt).replace(/^[-*]\s+/gm, "").replace(/\s+/g, " ").trim();
}
