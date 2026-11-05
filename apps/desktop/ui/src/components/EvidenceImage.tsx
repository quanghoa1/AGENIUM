// A screenshot of a ticket's report (ADR-158): on the ticket, and in the Lead's chat when it shares one. A click shows it
// at full width; a missing one says so.
import { ImageOff } from "lucide-react";
import { useEffect, useState } from "react";
import { api } from "../lib/api";
import { useApp } from "../lib/store";
import { cn } from "./ui";

export function EvidenceImage({ project, ticket, image, caption }: { project: string; ticket: string; image: string; caption?: string }) {
  const app = useApp();
  const [src, setSrc] = useState<string | null | undefined>(undefined);
  const [big, setBig] = useState(false);
  useEffect(() => {
    let live = true;
    api.evidenceImage(project, ticket, image).then(
      (r) => live && setSrc(`data:${r.media_type};base64,${r.base64}`),
      () => live && setSrc(null),
    );
    return () => {
      live = false;
    };
  }, [project, ticket, image]);
  const label = caption?.trim() || image;
  return (
    <figure className="w-fit max-w-full overflow-hidden rounded-lg border border-line bg-elevated/40">
      {src === undefined && <div className="h-32 w-56 animate-pulse bg-elevated" />}
      {src === null && (
        <div className="flex items-center gap-1.5 px-3 py-2 text-[11px] text-subtle">
          <ImageOff className="h-3.5 w-3.5" /> {app.t("evidence.missing").replace("{name}", image)}
        </div>
      )}
      {src && (
        <button className="block max-w-full" title={app.t(big ? "evidence.smaller" : "evidence.bigger")} onClick={() => setBig(!big)}>
          <img src={src} alt={label} className={cn("block max-w-full object-contain", big ? "max-h-[80vh]" : "max-h-64")} />
        </button>
      )}
      <figcaption className="border-t border-line px-2.5 py-1 text-[11px] text-muted">
        <span className="font-mono text-subtle">{ticket}</span> · {label}
      </figcaption>
    </figure>
  );
}
