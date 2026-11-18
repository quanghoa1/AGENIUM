import { Trash2 } from "lucide-react";
import { useState } from "react";
import { api } from "../lib/api";
import { useApp } from "../lib/store";
import type { ProjectSummary } from "../lib/types";
import { ConfirmModal } from "./motion";

/** Removes a project that never started (no team, no tickets, no plan) from Agenium; its folder stays as it is. */
export function RemoveProject({ p }: { p: ProjectSummary }) {
  const app = useApp();
  const [ask, setAsk] = useState(false);
  return (
    <>
      <button className="btn" title={app.t("projects.removeHint")} onClick={() => setAsk(true)}>
        <Trash2 className="h-3.5 w-3.5" /> {app.t("projects.remove")}
      </button>
      <ConfirmModal
        open={ask}
        title={app.t("projects.removeAsk").replace("{p}", p.name)}
        note={app.t("projects.removeNote").replace("{path}", p.path)}
        confirm={app.t("projects.remove")}
        icon={<Trash2 className="h-3.5 w-3.5" />}
        onClose={() => setAsk(false)}
        onConfirm={() => {
          setAsk(false);
          api
            .removeProject(p.id)
            .then(() => {
              app.notify(app.t("projects.removed").replace("{p}", p.name));
              if (app.project === p.id) app.setProject(null);
              app.refresh();
            })
            .catch((e) => app.notify(String(e)));
        }}
      />
    </>
  );
}
