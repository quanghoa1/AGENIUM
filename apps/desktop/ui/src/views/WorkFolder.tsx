// The first start on Linux and macOS (ADR-154): which folder Agenium works in, asked once before anything is written.
// Windows asks in its installer. The folder holds the user's skills, MCP servers, agents, projects and memory; an
// update never touches it. The language is changed later with the VI/EN switch.
import { FolderOpen, Loader2 } from "lucide-react";
import { useState } from "react";
import { Logo } from "../components/Logo";
import { api } from "../lib/api";
import { friendly } from "../lib/errors";
import { useApp } from "../lib/store";

export function WorkFolderView() {
  const app = useApp();
  const [folder, setFolder] = useState(app.status?.choose_root ?? "");
  const [busy, setBusy] = useState(false);
  const pick = () =>
    api
      .pickWorkFolder()
      .then((f) => f && setFolder(f))
      .catch((e) => app.notify(friendly(e, app.t)));
  const start = () => {
    setBusy(true);
    api
      .chooseWorkFolder(folder)
      .then(app.reloadStatus)
      .catch((e) => (app.notify(friendly(e, app.t)), setBusy(false)));
  };
  return (
    <div className="flex h-full items-center justify-center bg-background p-6">
      <div className="card w-[520px] p-6">
        <div className="mb-4 flex items-center gap-3">
          <Logo className="h-10 w-10 shrink-0 rounded-lg shadow-sm" />
          <div className="text-base font-semibold">{app.t("setup.title")}</div>
        </div>
        <div className="text-xs text-muted">{app.t("setup.why")}</div>
        <div className="mt-3 flex items-center gap-2">
          <div className="min-w-0 flex-1 truncate rounded-lg border border-line bg-elevated px-2.5 py-2 font-mono text-[12px]" title={folder}>
            {folder}
          </div>
          <button className="btn shrink-0" disabled={busy} onClick={pick}>
            <FolderOpen className="h-3.5 w-3.5" /> {app.t("setup.pick")}
          </button>
        </div>
        <div className="mt-2 text-[11px] text-subtle">{app.t("setup.note")}</div>
        <button className="btn-primary mt-4 w-full justify-center" disabled={busy || !folder} onClick={start}>
          {busy && <Loader2 className="h-3.5 w-3.5 animate-spin" />} {app.t("setup.go")}
        </button>
      </div>
    </div>
  );
}
