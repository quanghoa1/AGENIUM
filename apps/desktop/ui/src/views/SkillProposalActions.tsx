// An agent's skill proposal in Needs you (ADR-152): review it by hand in the skill editor, discuss it with the skill
// writer (a session that starts from the proposal and its evidence), or decline it. Whatever is saved goes into the
// library and is switched on for the agent that proposed it.
import { Loader2, Pencil, Wand2, X } from "lucide-react";
import { useState } from "react";
import { api, type SkillProposal } from "../lib/api";
import { friendly } from "../lib/errors";
import { useApp } from "../lib/store";
import { SkillEditor } from "./capabilities/SkillEditor";
import { SkillWriter } from "./capabilities/SkillWriter";

export function SkillProposalActions({ id }: { id: string }) {
  const app = useApp();
  const [busy, setBusy] = useState(false);
  const [open, setOpen] = useState<{ how: "hand" | "writer"; p: SkillProposal } | null>(null);
  const load = (how: "hand" | "writer") => {
    setBusy(true);
    api
      .skillProposal(id)
      .then((p) => setOpen({ how, p }))
      .catch((e) => app.notify(friendly(e, app.t)))
      .finally(() => setBusy(false));
  };
  const decline = () => {
    setBusy(true);
    api
      .decideSkillProposal(id, false, null, null)
      .then(() => (app.notify(app.t("proposal.declined")), app.refresh()))
      .catch((e) => app.notify(friendly(e, app.t)))
      .finally(() => setBusy(false));
  };
  const savedByHand = (p: SkillProposal, name: string) =>
    api
      .decideSkillProposal(p.id, true, name)
      .then(() => (app.notify(app.t("proposal.saved").replace("{name}", name).replace("{agent}", p.agent_name || "–")), setOpen(null), app.refresh()))
      .catch((e) => app.notify(friendly(e, app.t)));
  return (
    <>
      <button disabled={busy} className="btn-danger" title={app.t("proposal.decline")} onClick={decline}>
        <X className="h-3.5 w-3.5" />
      </button>
      <button disabled={busy} className="btn" title={app.t("proposal.discussHint")} onClick={() => load("writer")}>
        {busy ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Wand2 className="h-3.5 w-3.5" />} {app.t("proposal.discuss")}
      </button>
      <button disabled={busy} className="btn-primary" onClick={() => load("hand")}>
        <Pencil className="h-3.5 w-3.5" /> {app.t("proposal.byHand")}
      </button>
      {open?.how === "hand" && (
        <SkillEditor
          initial={open.p.text}
          create
          name0=""
          files0={[]}
          origin={`proposal:${open.p.contexts.length}`}
          onClose={() => setOpen(null)}
          onSaved={(name) => name && savedByHand(open.p, name)}
        />
      )}
      {open?.how === "writer" && <SkillWriter proposal={open.p} onClose={() => setOpen(null)} onSaved={() => (setOpen(null), app.refresh())} />}
    </>
  );
}
