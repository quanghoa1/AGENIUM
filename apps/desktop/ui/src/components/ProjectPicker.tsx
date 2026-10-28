import { useApp } from "../lib/store";

/** The project a page works on (its board, its Lead's chat, its audit trail, its report), picked on that page. */
export function ProjectPicker() {
  const app = useApp();
  return (
    <label className="flex items-center gap-2">
      <span className="label">{app.t("top.project")}</span>
      <select className="input w-52" value={app.project ?? ""} onChange={(e) => app.setProject(e.target.value || null)}>
        {!app.project && <option value="">–</option>}
        {app.projects.map((p) => (
          <option key={p.id} value={p.id}>
            {p.name}
          </option>
        ))}
      </select>
    </label>
  );
}
