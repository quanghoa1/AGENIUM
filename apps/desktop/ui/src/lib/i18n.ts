import { shell as en_shell } from "./strings/en/shell";
import { work as en_work } from "./strings/en/work";
import { organize as en_organize } from "./strings/en/organize";
import { library as en_library } from "./strings/en/library";
import { settings as en_settings } from "./strings/en/settings";
import { helpers as en_helpers } from "./strings/en/helpers";
import { shell as vi_shell } from "./strings/vi/shell";
import { work as vi_work } from "./strings/vi/work";
import { organize as vi_organize } from "./strings/vi/organize";
import { library as vi_library } from "./strings/vi/library";
import { settings as vi_settings } from "./strings/vi/settings";
import { helpers as vi_helpers } from "./strings/vi/helpers";

// UI strings in English and Vietnamese (decision: UI en + vi). Content written by agents (tickets, plans) stays in
// English below the Lead; the Lead talks to the user in the user's language.
// The strings live in strings/<language>/<area>.ts.
export type Lang = "en" | "vi";

const en = { ...en_shell, ...en_work, ...en_organize, ...en_library, ...en_settings, ...en_helpers };

export type Key = keyof typeof en;

const vi: Record<Key, string> = { ...vi_shell, ...vi_work, ...vi_organize, ...vi_library, ...vi_settings, ...vi_helpers };

export const dictionaries: Record<Lang, Record<Key, string>> = { en, vi };
