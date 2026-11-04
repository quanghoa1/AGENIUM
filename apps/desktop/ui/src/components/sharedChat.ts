// One state for every pane showing the same conversation: the chat page and the Live view's chat are one chat with
// the Lead, so a message sent in one, its turn in progress and the queue are seen in both at once.
import { useCallback, useEffect, useState } from "react";

type Entry = { value: unknown; subs: Set<(v: unknown) => void> };
const entries = new Map<string, Entry>();

function entry(key: string, init: unknown): Entry {
  let e = entries.get(key);
  if (!e) {
    e = { value: init, subs: new Set() };
    entries.set(key, e);
  }
  return e;
}

/** `useState` shared by every component that asks for the same `key` (a project's chat). */
export function useShared<T>(key: string, init: T): [T, (next: T | ((prev: T) => T)) => void] {
  const [value, setValue] = useState<T>(() => entry(key, init).value as T);
  useEffect(() => {
    const e = entry(key, init);
    const sub = (v: unknown) => setValue(v as T);
    e.subs.add(sub);
    setValue(e.value as T);
    return () => {
      e.subs.delete(sub);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key]);
  const set = useCallback(
    (next: T | ((prev: T) => T)) => {
      const e = entry(key, init);
      e.value = typeof next === "function" ? (next as (p: T) => T)(e.value as T) : next;
      e.subs.forEach((s) => s(e.value));
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [key],
  );
  return [value, set];
}
