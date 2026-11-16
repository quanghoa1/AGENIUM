// What a drafting helper's model has written so far (ADR-151): its pieces arrive as `agenium://helper` events while it
// writes; the chat shows the reply as it grows instead of a spinner for minutes.

/** One piece of a helper's answer as it is written. */
export interface HelperPiece {
  session: string | null;
  kind: "text" | "thinking" | "restart";
  text: string;
}

/** What the chat shows while the answer is written. */
export interface LiveAnswer {
  /** The answer's text so far (the helper answers in JSON: its `reply` is read from it). */
  raw: string;
  /** How much reasoning the model has written (shown as a count, not its text). */
  thinking: number;
}

export const emptyLive: LiveAnswer = { raw: "", thinking: 0 };

/** `live` with `piece` added. */
export function addPiece(live: LiveAnswer, piece: HelperPiece): LiveAnswer {
  if (piece.kind === "restart") return emptyLive;
  if (piece.kind === "thinking") return { ...live, thinking: live.thinking + piece.text.length };
  return { ...live, raw: live.raw + piece.text };
}

/** The `reply` written so far in a helper's partial JSON answer (`{"reply": "...", ...}`), escapes decoded; `null`
 *  before its value starts. An answer that is not JSON is shown as it is. */
export function partialReply(raw: string): string | null {
  const t = raw.trimStart();
  if (!t) return null;
  if (!t.startsWith("{") && !t.startsWith("```")) return t;
  const key = /"reply"\s*:\s*"/.exec(t);
  if (!key) return null;
  let out = "";
  for (let i = key.index + key[0].length; i < t.length; i++) {
    const c = t[i];
    if (c === '"') return out;
    if (c !== "\\") {
      out += c;
      continue;
    }
    const n = t[i + 1];
    if (n === undefined) return out;
    i++;
    if (n === "n") out += "\n";
    else if (n === "t") out += "\t";
    else if (n === "r") continue;
    else if (n === "u") {
      const hex = t.slice(i + 1, i + 5);
      if (hex.length < 4) return out;
      out += String.fromCharCode(parseInt(hex, 16));
      i += 4;
    } else out += n;
  }
  return out;
}
