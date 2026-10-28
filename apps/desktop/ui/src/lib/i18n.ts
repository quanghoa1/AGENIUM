import { shell as en_shell } from "./strings/en/shell";
import { work as en_work } from "./strings/en/work";
import { organize as en_organize } from "./strings/en/organize";
import { settings as en_settings } from "./strings/en/settings";
import { shell as vi_shell } from "./strings/vi/shell";
import { work as vi_work } from "./strings/vi/work";
import { organize as vi_organize } from "./strings/vi/organize";
import { settings as vi_settings } from "./strings/vi/settings";

// UI strings in English and Vietnamese (decision: UI en + vi). Content written by agents (tickets, plans) stays in
// English below the Lead; the Lead talks to the user in the user's language.
// The strings live in strings/<language>/<area>.ts.
export type Lang = "en" | "vi";

const en = { ...en_shell, ...en_work, ...en_organize, ...en_settings };

export type Key = keyof typeof en;

const vi: Record<Key, string> = { ...vi_shell, ...vi_work, ...vi_organize, ...vi_settings };

export const dictionaries: Record<Lang, Record<Key, string>> = { en, vi };
