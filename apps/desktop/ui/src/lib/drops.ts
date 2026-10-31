// Files dropped on the window (ADR-152). Tauri gives their paths to one listener; the top-most surface that takes
// drops gets them, so a chat open over the skills page takes a dropped document and the page does not also import it.
import { type MutableRefObject, useEffect, useRef } from "react";
import { inTauri } from "./api";

export interface DropEvent {
  type: "enter" | "over" | "leave" | "drop";
  paths: string[];
}
type Handler = (e: DropEvent) => void;

const stack: { id: number; fn: MutableRefObject<Handler> }[] = [];
let installed = false;
let nextId = 1;

function install() {
  if (installed || !inTauri) return;
  installed = true;
  import("@tauri-apps/api/webview")
    .then(({ getCurrentWebview }) =>
      getCurrentWebview().onDragDropEvent((e) => {
        const top = stack[stack.length - 1];
        const p = e.payload as { type: DropEvent["type"]; paths?: string[] };
        top?.fn.current({ type: p.type, paths: p.paths ?? [] });
      }),
    )
    .catch(() => (installed = false));
}

/** Takes files dropped on the window while `active`, unless a surface opened later takes them. */
export function useFileDrop(handler: Handler, active = true) {
  const ref = useRef(handler);
  ref.current = handler;
  useEffect(() => {
    if (!active) return;
    install();
    const id = nextId++;
    // The surface below stops showing a drop under way: it gets no more events until this one closes.
    stack[stack.length - 1]?.fn.current({ type: "leave", paths: [] });
    stack.push({ id, fn: ref });
    return () => {
      const i = stack.findIndex((x) => x.id === id);
      if (i >= 0) stack.splice(i, 1);
    };
  }, [active]);
}
