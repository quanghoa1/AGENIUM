import { Archive } from "lucide-react";
import { useState } from "react";
import { api } from "../lib/api";
import type { Key } from "../lib/i18n";
import { friendly } from "../lib/errors";
import { useApp } from "../lib/store";
import type { ProjectSummary } from "../lib/types";
import { ConfirmModal } from "./motion";

/** The user closes a done project after looking it over (ADR-130; no agent can): nothing runs on it any more and its
 *  team is set free, members and Lead together, to take another project in Teams. */
export function CloseProject({ p }: { p: ProjectSummary }) {
  const app = useApp();
  const [ask, setAsk] = useState(false);
  const fill = (k: Key, team?: string | null) =>
    app
      .t(k)
      .replace("{p}", p.name)
      .replace("{n}", String(p.tickets))
      .replace("{i}", `${p.plan_done}/${p.plan_items}`)
      .replace("{team}", team ?? p.team?.name ?? "–");
  return (
    <>
      <button className="btn" title={app.t("run.closeHint")} onClick={() => setAsk(true)}>
        <Archive className="h-3.5 w-3.5" /> {app.t("run.close")}
      </button>
      <ConfirmModal
        open={ask}
        title={fill("run.closeAsk")}
        note={fill(p.team ? "run.closeNote" : "run.closeNoteNoTeam")}
        confirm={app.t("run.close")}
        icon={<Archive className="h-3.5 w-3.5" />}
        onClose={() => setAsk(false)}
        onConfirm={() => {
          setAsk(false);
          api
            .closeProject(p.id)
            .then(({ team, taken_by }) => {
              app.notify(
                taken_by ? fill("run.closedTaken", team).replace("{q}", taken_by) : fill(team ? "run.closed" : "run.closedNoTeam", team),
              );
              app.refresh();
              // A freed team nobody waited for is given its next project on the Teams page.
              if (team && !taken_by) app.go("teams");
            })
            .catch((e) => app.notify(friendly(e, app.t)));
        }}
      />
    </>
  );
}
