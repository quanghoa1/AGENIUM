import { RotateCcw } from "lucide-react";
import { useEffect, useState } from "react";
import { api } from "../lib/api";
import { friendly } from "../lib/errors";
import { useApp } from "../lib/store";
import type { ProjectSummary, TeamsData } from "../lib/types";
import { Modal } from "./motion";

/** The user reopens a closed project (ADR-130; no agent can) and picks who takes it: by default the team that worked on
 *  it (it knows the project, its memory of it), now when it is free or as soon as it is free when it is busy on another
 *  project; any other free team now; or nobody yet. The choice is `team:<id>`, `wait:<id>` or empty. */
export function ReopenProject({ p }: { p: ProjectSummary }) {
  const app = useApp();
  const [open, setOpen] = useState(false);
  const [teams, setTeams] = useState<TeamsData["teams"]>([]);
  const [choice, setChoice] = useState("");
  useEffect(() => {
    if (!open) return;
    api.teams().then((d) => {
      setTeams(d.teams);
      const before = d.teams.find((t) => t.team.id === p.previous_team);
      const free = d.teams.filter((t) => !t.team.project);
      setChoice(before ? `${before.team.project ? "wait" : "team"}:${before.team.id}` : free[0] ? `team:${free[0].team.id}` : "");
    });
  }, [open, p.previous_team]);
  const name = p.name;
  const before = teams.find((t) => t.team.id === p.previous_team);
  const others = teams.filter((t) => !t.team.project && t.team.id !== p.previous_team);
  const reopen = () => {
    setOpen(false);
    const [kind, id] = choice.split(":");
    api
      .reopenProject(p.id, id || null, kind === "wait")
      .then(() => {
        const who = teams.find((t) => t.team.id === id)?.team.name ?? "";
        const key = !id ? "run.reopenedNoTeam" : kind === "wait" ? "run.reopenedWait" : "run.reopened";
        app.notify(
          app.t(key).replace("{p}", name).replace("{team}", who),
          kind === "team" ? { label: app.t("top.talk"), run: () => (app.setProject(p.id), app.go("chat")) } : undefined,
        );
        app.refresh();
      })
      .catch((e) => app.notify(friendly(e, app.t)));
  };
  return (
    <>
      <button className="btn" title={app.t("run.reopenHint")} onClick={() => setOpen(true)}>
        <RotateCcw className="h-3.5 w-3.5" /> {app.t("run.reopen")}
      </button>
      <Modal open={open} title={app.t("run.reopenAsk").replace("{p}", name)} onClose={() => setOpen(false)}>
        <p className="text-xs text-muted">{app.t("run.reopenNote")}</p>
        <label className="mt-3 block text-xs">
          <span className="label">{app.t("run.reopenTeam")}</span>
          <select className="input mt-1" value={choice} onChange={(e) => setChoice(e.target.value)}>
            {before && !before.team.project && (
              <option value={`team:${before.team.id}`}>
                {before.team.name} ({app.t("run.reopenPrevious")})
              </option>
            )}
            {before?.team.project && (
              <option value={`wait:${before.team.id}`}>
                {app.t("run.reopenWait").replace("{team}", before.team.name).replace("{q}", before.project_name ?? "–")}
              </option>
            )}
            {others.map((t) => (
              <option key={t.team.id} value={`team:${t.team.id}`}>
                {t.team.name}
              </option>
            ))}
            <option value="">{app.t("run.reopenNoTeam")}</option>
          </select>
        </label>
        <div className="mt-4 flex justify-end gap-2">
          <button className="btn" onClick={() => setOpen(false)}>
            {app.t("common.cancel")}
          </button>
          <button className="btn-primary" onClick={reopen}>
            <RotateCcw className="h-3.5 w-3.5" /> {app.t("run.reopen")}
          </button>
        </div>
      </Modal>
    </>
  );
}
