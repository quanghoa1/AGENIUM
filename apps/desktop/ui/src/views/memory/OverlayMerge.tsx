// An agent's copy of a skill whose base changed under it (a plugin's vendor update, a library skill refined) and that
// did not rebase on its own (ADR-157): the helper drafts the copy on the new base, keeping what the agent learned; the
// user reads it, may edit it, and accepts it. Nothing changes until then.
import { GitMerge, Loader2, Wand2, X } from "lucide-react";
import { useEffect, useState } from "react";
import { DiffView } from "../../components/DiffView";
import { Overlay } from "../../components/motion";
import { api, type ModelArg } from "../../lib/api";
import { friendly } from "../../lib/errors";
import { remember, rememberedModel } from "../../lib/remembered";
import { useApp } from "../../lib/store";
import type { Capabilities, OverlayMergeDraft } from "../../lib/types";
import { ModelPicker, emptyModel } from "../organize/ModelPicker";

export function OverlayMerge({ agent, skill, onClose, onDone }: { agent: string; skill: string; onClose: () => void; onDone: () => void }) {
  const app = useApp();
  const [caps, setCaps] = useState<Capabilities | null>(null);
  const [model, setModel] = useState<ModelArg>(emptyModel);
  const [draft, setDraft] = useState<OverlayMergeDraft | null>(null);
  const [text, setText] = useState("");
  const [busy, setBusy] = useState<"draft" | "accept" | null>(null);
  useEffect(() => {
    // The skill writer's model: the helper that writes skills drafts this merge too.
    Promise.all([api.capabilities(false, true), api.preference("model.skill_writer").catch(() => null)])
      .then(([c, kept]) => {
        setCaps(c);
        const r = rememberedModel(kept, c);
        if (r.model) setModel(r.model);
      })
      .catch(() => undefined);
  }, []);
  const ready = !!model.provider && !!model.model;
  const ask = () => {
    setBusy("draft");
    remember("model.skill_writer", model);
    api
      .draftOverlayMerge(agent, skill, model)
      .then((d) => (setDraft(d), setText(d.merged)))
      .catch((e) => app.notify(friendly(e, app.t)))
      .finally(() => setBusy(null));
  };
  const accept = () => {
    setBusy("accept");
    api
      .acceptOverlayMerge(agent, skill, text)
      .then(() => (app.notify(app.t("merge.accepted")), onDone()))
      .catch((e) => app.notify(friendly(e, app.t)))
      .finally(() => setBusy(null));
  };
  return (
    <Overlay open onClose={() => !busy && onClose()} kind="dialog" top="pt-[4vh]">
      <div role="dialog" aria-modal="true" aria-label={app.t("merge.title")} className="card flex max-h-[90vh] w-[min(1200px,94vw)] flex-col gap-3 p-4">
        <div className="flex items-center gap-2">
          <GitMerge className="h-4 w-4 text-accent" />
          <span className="text-sm font-semibold">
            {app.t("merge.title")} · {skill}
          </span>
          <button className="btn ml-auto px-2" disabled={!!busy} onClick={onClose} title={app.t("common.cancel")}>
            <X className="h-3.5 w-3.5" />
          </button>
        </div>
        <div className="text-xs text-subtle">{app.t("merge.intro")}</div>
        <div className="rounded-lg border border-line bg-elevated/30 p-2.5">
          <div className="label mb-1.5">{app.t("helper.model")}</div>
          <ModelPicker value={model} onChange={setModel} caps={caps} />
        </div>
        <div className="flex justify-end">
          <button className="btn" disabled={!ready || !!busy} onClick={ask} data-testid="merge-draft">
            {busy === "draft" ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Wand2 className="h-3.5 w-3.5" />} {app.t(draft ? "merge.again" : "merge.draft")}
          </button>
        </div>
        {draft && (
          <div className="grid min-h-0 flex-1 gap-3 overflow-y-auto lg:grid-cols-2">
            <div className="flex min-h-0 flex-col gap-1">
              <div className="label">{app.t("merge.edit")}</div>
              <textarea className="input min-h-[320px] flex-1 font-mono text-[11px]" value={text} onChange={(e) => setText(e.target.value)} spellCheck={false} />
            </div>
            <div className="flex min-h-0 flex-col gap-1">
              <div className="label">{app.t("merge.diff")}</div>
              <DiffView diff={draft.diff} />
            </div>
          </div>
        )}
        <div className="flex justify-end gap-2">
          <button className="btn" disabled={!!busy} onClick={onClose}>
            {app.t("common.cancel")}
          </button>
          <button className="btn-primary" disabled={!draft || !text.trim() || !!busy} onClick={accept} data-testid="merge-accept">
            {busy === "accept" ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <GitMerge className="h-3.5 w-3.5" />} {app.t("merge.accept")}
          </button>
        </div>
      </div>
    </Overlay>
  );
}
