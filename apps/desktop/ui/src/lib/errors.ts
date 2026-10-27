// The backend's errors are English sentences for logs and agents. The few a user meets while setting things up are
// said again in the interface language; anything else is shown as it came.
import type { Key } from "./i18n";

const KNOWN: [RegExp, Key][] = [
  [/a project named "?(.+?)"? already exists/i, "err.projectNameTaken"],
  [/the folder (.+?) already exists/i, "err.folderTaken"],
  [/git clone failed: (.+)/i, "err.cloneFailed"],
  [/an agent named "?(.+?)"? already exists|agent name "?(.+?)"? is (?:already )?(?:taken|in use)/i, "err.agentNameTaken"],
  [/a provider named "?(.+?)"? already exists|provider "?(.+?)"? already exists/i, "err.providerTaken"],
];

/** `e` as the user should read it: a known error in their language (its detail kept), else the text itself. */
export function friendly(e: unknown, t: (k: Key) => string): string {
  const text = String(e).replace(/^Error:\s*/, "").replace(/^(invalid|conflict|not found):\s*/i, "");
  for (const [re, key] of KNOWN) {
    const m = text.match(re);
    if (m) return t(key).replace("{x}", (m.slice(1).find(Boolean) ?? "").trim());
  }
  return text;
}
